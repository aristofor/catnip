# FILE: tests/language/test_memoization_advanced.py
"""
Advanced tests for function cache with key_func and validator.
"""

import time
from pathlib import Path

import pytest

from catnip import Catnip


def test_cached_with_key_func():
    """Test with a custom key function"""
    cat = Catnip()

    # Inject a file-based cache key function
    def file_hash(files):
        """Simule un hash de fichiers"""
        return "|".join(sorted(files))

    cat.context.globals['file_hash'] = file_hash

    code = """
    counter = 0

    # Custom key function based on file hashes
    compute_key = (files) => { file_hash(files) }

    # Build with cache keyed on file content
    build = cached((files) => { counter = counter + 1; "output_" + str(counter) }, "build", compute_key)

    # First call
    r1 = build(list("a.txt", "b.txt"))
    count1 = counter

    # Second call (same files, same order): cache hit
    r2 = build(list("a.txt", "b.txt"))
    count2 = counter

    # Third call (same files, different order): cache hit because key_func sorts
    r3 = build(list("b.txt", "a.txt"))
    count3 = counter

    # Fourth call (different files): cache miss
    r4 = build(list("c.txt"))
    count4 = counter

    list(count1, count2, count3, count4)
    """

    cat.parse(code)
    result = cat.execute()

    # count1=1 (first execution)
    # count2=1 (cache hit)
    # count3=1 (cache hit, same key thanks to compute_key sorting)
    # count4=2 (new execution, different files)
    assert result == [1, 1, 1, 2]


def test_cached_with_validator():
    """Test avec une fonction de validation du cache"""
    cat = Catnip()

    # External flag to control validity
    cache_valid = {"value": True}

    def validator_func(cached_result, *args, **kwargs):
        """Validator that checks an external flag"""
        return cache_valid['value']

    cat.context.globals['validator'] = validator_func
    cat.context.globals['invalidate_external'] = lambda: cache_valid.update({'value': False})
    cat.context.globals['validate_external'] = lambda: cache_valid.update({'value': True})

    code = """
    counter = 0

    # Function with cache validation
    compute = cached((x) => { counter = counter + 1; x * 10 }, "compute", None, validator)

    # First call
    r1 = compute(5)
    count1 = counter

    # Second call: cache hit
    r2 = compute(5)
    count2 = counter

    # Invalidate via external validator
    invalidate_external()

    # Third call: cache invalid, re-execute
    r3 = compute(5)
    count3 = counter

    # Revalidate
    validate_external()

    # Fourth call: new cache entry
    r4 = compute(5)
    count4 = counter

    list(count1, count2, count3, count4)
    """

    cat.parse(code)
    result = cat.execute()

    # count1=1 (first execution)
    # count2=1 (cache hit)
    # count3=2 (invalidated by validator)
    # count4=2 (cache hit on new result)
    assert result == [1, 1, 2, 2]


def test_cached_file_based_invalidation():
    """Invalidation based on file timestamps"""
    cat = Catnip()

    # Create a temporary file
    test_file = Path("test_cache_file.txt")
    test_file.write_text("initial")
    initial_mtime = test_file.stat().st_mtime

    # Key function based on the file
    def file_key(filename):
        path = Path(filename)
        if path.exists():
            return f"{filename}:{path.stat().st_mtime}"
        return filename

    # Validator that checks file existence
    def file_validator(cached_result, filename):
        return Path(filename).exists()

    cat.context.globals['file_key'] = file_key
    cat.context.globals['file_validator'] = file_validator

    code = """
    counter = 0

    # Build with cache keyed on file timestamp
    process_file = cached((filename) => { counter = counter + 1; "processed_" + str(counter) }, "process_file", file_key, file_validator)

    # First call
    r1 = process_file("test_cache_file.txt")
    count1 = counter

    # Second call: cache hit (file unchanged)
    r2 = process_file("test_cache_file.txt")
    count2 = counter

    list(r1, r2, count1, count2)
    """

    cat.parse(code)
    result = cat.execute()

    assert result[0] == "processed_1"
    assert result[1] == "processed_1"  # Cache hit
    assert result[2] == 1
    assert result[3] == 1

    # Modify the file
    time.sleep(0.01)  # Ensure mtime changes
    test_file.write_text("modified")

    # Re-parse and execute
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization  # Reuse the same cache
    cat2.context.globals['file_key'] = file_key
    cat2.context.globals['file_validator'] = file_validator

    code2 = """
    counter = 2  # Continue the counter

    process_file = cached((filename) => { counter = counter + 1; "processed_" + str(counter) }, "process_file", file_key, file_validator)

    # File changed, so cache key changes
    r3 = process_file("test_cache_file.txt")
    count3 = counter

    list(r3, count3)
    """

    cat2.parse(code2)
    result2 = cat2.execute()

    # Re-executed because the timestamp changed
    assert result2[0] == "processed_3"
    assert result2[1] == 3

    # Cleanup
    test_file.unlink()


def test_cached_with_dependencies():
    """Test similar to BuildParserLegacy with dependencies"""
    cat = Catnip()

    # Simulate source files
    sources = {"a.txt": "content_a", "b.txt": "content_b"}

    # Function to compute dependencies (file hashes)
    def compute_deps(files_list):
        """Compute a hash based on file contents"""
        content = "|".join(sources.get(f, "") for f in files_list)
        return hash(content)

    # Key function that includes dependencies
    def build_key(files_list):
        deps = compute_deps(files_list)
        return f"{','.join(files_list)}:{deps}"

    cat.context.globals['build_key'] = build_key

    code = """
    builds = 0

    # Build function with dependency-based cache
    build_sass = cached((files) => { builds = builds + 1; "output_" + str(builds) + ".css" }, "build_sass", build_key)

    # First build
    out1 = build_sass(list("a.txt", "b.txt"))
    count1 = builds

    # Second build (same files, same content): cache hit
    out2 = build_sass(list("a.txt", "b.txt"))
    count2 = builds

    list(out1, out2, count1, count2)
    """

    cat.parse(code)
    result = cat.execute()

    assert result[0] == "output_1.css"
    assert result[1] == "output_1.css"  # Cache hit
    assert result[2] == 1
    assert result[3] == 1

    # Modify a source file content
    sources["a.txt"] = "modified_content_a"

    # Re-execute with modified content
    cat2 = Catnip()
    cat2.context.memoization = cat.context.memoization
    cat2.context.globals['build_key'] = build_key

    code2 = """
    builds = 1  # Continue the counter

    build_sass = cached((files) => { builds = builds + 1; "output_" + str(builds) + ".css" }, "build_sass", build_key)

    # Build with modified content: cache miss
    out3 = build_sass(list("a.txt", "b.txt"))
    count3 = builds

    list(out3, count3)
    """

    cat2.parse(code2)
    result2 = cat2.execute()

    # Re-executed because content changed
    assert result2[0] == "output_2.css"
    assert result2[1] == 2

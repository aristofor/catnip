#!/usr/bin/env python3
"""Serialization examples with pickle.

Demonstrates pickle support for Catnip objects:
- AST nodes (Op)
- Scopes with variables
- Lambdas and closures
- Practical use: disk cache

All Catnip internal objects are picklable for:
- Disk caching (avoid re-parsing)
- Multiprocessing (send code between workers)
- Debug snapshots (save execution state)

Usage:
    python docs/examples/advanced/pickle_example.py
"""

import pickle
import tempfile
from pathlib import Path


def example_pickle_ast():
    """Pickle an AST and restore it."""
    from catnip import Catnip

    print("⇒ Pickle AST")
    print("=" * 50)

    # Parse code to AST
    c = Catnip()
    ast = c.parse("(1 + 2) * 3 + 4")
    print(f"Original AST: {ast}")

    # Pickle the AST
    data = pickle.dumps(ast)
    print(f"Pickled: {len(data)} bytes")

    # Unpickle and execute
    restored_ast = pickle.loads(data)
    print(f"Restored AST: {restored_ast}")

    c2 = Catnip()
    c2.code = restored_ast
    result = c2.execute()
    print(f"Execution result: {result}")
    print()


def example_pickle_scope():
    """Pickle a Scope with variables."""
    from catnip._rs import Scope

    print("⇒ Pickle Scope")
    print("=" * 50)

    # Create scope with variables
    scope = Scope()
    scope._set('x', 42)
    scope._set('name', "catnip")
    scope._set('items', [1, 2, 3])

    print("Original scope:")
    print(f"  x = {scope._resolve('x')}")
    print(f"  name = {scope._resolve('name')}")
    print(f"  items = {scope._resolve('items')}")

    # Pickle
    data = pickle.dumps(scope)
    print(f"Pickled: {len(data)} bytes")

    # Unpickle
    restored = pickle.loads(data)
    print("Restored scope:")
    print(f"  x = {restored._resolve('x')}")
    print(f"  name = {restored._resolve('name')}")
    print(f"  items = {restored._resolve('items')}")
    print()


def example_pickle_lambda():
    """Pickle a simple lambda."""
    from catnip import Catnip
    from catnip._rs import set_global_registry

    print("⇒ Pickle Lambda")
    print("=" * 50)

    # Create lambda
    c = Catnip()
    c.parse("double = (x) => { x * 2 }")
    c.execute()

    # Required for unpickling (reconstructs opcodes)
    set_global_registry(c.registry)

    double = c.context.globals.get('double')
    print(f"Original lambda: {double}")
    print(f"Test: double(21) = {double(21)}")

    # Pickle
    data = pickle.dumps(double)
    print(f"Pickled: {len(data)} bytes")

    # Unpickle and use in new context
    restored = pickle.loads(data)
    print(f"Restored lambda: {restored}")

    c2 = Catnip()
    c2.context.globals['f'] = restored
    c2.parse("f(21)")
    result = c2.execute()
    print(f"Test: f(21) = {result}")
    print()


def example_pickle_closure():
    """Pickle a closure with captured variables."""
    from catnip import Catnip
    from catnip._rs import set_global_registry

    print("⇒ Pickle Closure")
    print("=" * 50)

    # Create closure that captures 'n'
    c = Catnip()
    c.parse("""
make_adder = (n) => {
    (x) => { x + n }
}
add10 = make_adder(10)
    """)
    c.execute()

    set_global_registry(c.registry)

    add10 = c.context.globals.get('add10')
    print(f"Original closure: {add10}")
    print(f"Test: add10(5) = {add10(5)}")
    print(f"Captured variable 'n' = 10")

    # Pickle
    data = pickle.dumps(add10)
    print(f"Pickled: {len(data)} bytes")

    # Unpickle and verify capture preserved
    restored = pickle.loads(data)
    print(f"Restored closure: {restored}")

    c2 = Catnip()
    c2.context.globals['adder'] = restored
    c2.parse("adder(5)")
    result = c2.execute()
    print(f"Test: adder(5) = {result}")
    print("✓ Captured variable 'n' preserved across pickle")
    print()


def example_disk_cache():
    """Practical example: disk cache for parsed AST."""
    from catnip import Catnip

    print("⇒ Disk Cache (Practical Use)")
    print("=" * 50)

    # Create temporary file for script
    with tempfile.NamedTemporaryFile(mode='w', suffix='.cat', delete=False) as f:
        script_path = Path(f.name)
        f.write("""
# Complex computation
fibonacci = (n) => {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}
fibonacci(10)
        """)

    cache_path = Path(f"{script_path}.cache")

    def load_with_cache(path):
        """Load script with AST cache."""
        if cache_path.exists():
            with open(cache_path, 'rb') as f:
                ast = pickle.load(f)
            print(f"✓ AST loaded from cache ({cache_path.name})")
            return ast
        else:
            c = Catnip()
            with open(path) as f:
                code = f.read()
            ast = c.parse(code)
            with open(cache_path, 'wb') as f:
                pickle.dump(ast, f)
            print(f"✓ AST parsed and cached to {cache_path.name}")
            return ast

    # First load: parse and cache
    print("First load:")
    ast1 = load_with_cache(script_path)

    # Second load: from cache (much faster)
    print("\nSecond load:")
    ast2 = load_with_cache(script_path)

    # Execute both to verify
    c = Catnip()
    c.code = ast2
    result = c.execute()
    print(f"\nExecution result: {result}")

    # Cleanup
    script_path.unlink()
    cache_path.unlink()
    print()


def example_protocols():
    """Test different pickle protocols."""
    from catnip import Catnip

    print("⇒ Pickle Protocols")
    print("=" * 50)

    c = Catnip()
    ast = c.parse("1 + 2 + 3 + 4 + 5")

    for protocol in [2, 3, 4, 5]:
        try:
            data = pickle.dumps(ast, protocol=protocol)
            restored = pickle.loads(data)

            c2 = Catnip()
            c2.code = restored
            result = c2.execute()

            print(f"Protocol {protocol}: {len(data):4d} bytes, result={result}")
        except Exception as e:
            print(f"Protocol {protocol}: not supported ({e})")

    print()


def example_size_comparison():
    """Compare pickle sizes."""
    from catnip import Catnip
    from catnip._rs import Scope

    print("⇒ Size Comparison")
    print("=" * 50)

    # Small AST
    c = Catnip()
    ast_small = c.parse("1 + 2")
    print(f"Small AST (1 + 2):           {len(pickle.dumps(ast_small)):5d} bytes")

    # Medium AST
    ast_medium = c.parse("(1 + 2) * (3 + 4) - (5 + 6)")
    print(f"Medium AST (arithmetic):     {len(pickle.dumps(ast_medium)):5d} bytes")

    # Large AST
    ast_large = c.parse("""
for i in range(100) {
    x = i * 2
    if x > 50 {
        print(x)
    }
}
    """)
    print(f"Large AST (loop + condition): {len(pickle.dumps(ast_large)):5d} bytes")

    # Scope with many variables
    scope = Scope()
    for i in range(100):
        scope._set(f'var_{i}', i * 10)
    print(f"Scope (100 variables):        {len(pickle.dumps(scope)):5d} bytes")

    print()


if __name__ == "__main__":
    print("\n" + "=" * 50)
    print("  Catnip Serialization Examples")
    print("=" * 50 + "\n")

    example_pickle_ast()
    example_pickle_scope()
    example_pickle_lambda()
    example_pickle_closure()
    example_disk_cache()
    example_protocols()
    example_size_comparison()

    print("=" * 50)
    print("All examples completed successfully!")
    print("=" * 50)

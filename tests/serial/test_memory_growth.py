# FILE: tests/serial/test_memory_growth.py
"""RSS-based leak witnesses for refcounts invisible to Python.

BigInt/Complex intermediates live in Rust Arcs: a leaked operand ref never
shows up in sys.getrefcount, but a chained-arithmetic loop grows the process
RSS linearly (measured 49 MB / 200k iterations before the operand-release
fix). RSS is process-global, hence serial and subprocess-isolated.
"""

import subprocess
import sys

import pytest


@pytest.mark.xdist_group('memory_growth')
def test_chained_bigint_intermediates_do_not_grow_rss():
    """Each chained BigInt op pops the previous intermediate; without the
    operand release its whole allocation leaked, once per operation."""
    script = """
import gc, os
from catnip import Catnip
from catnip.context import Context

def rss_kb():
    with open(f'/proc/{os.getpid()}/status') as f:
        for line in f:
            if line.startswith('VmRSS'):
                return int(line.split()[1])

def growth(code):
    c = Catnip(context=Context())
    c.parse(code)
    gc.collect()
    before = rss_kb()
    c.execute()
    gc.collect()
    return rss_kb() - before

# chained arithmetic intermediates (leaked ~50_000 kB before the fix)
g = growth('b = 10 ** 430\\ni = 0\\nwhile i < 200000 { x = (b + 1) + 2\\n i = i + 1 }\\n1')
assert g < 8000, f"RSS grew by {g} kB over 200k chained BigInt ops"
# collection elements: BuildList/BuildDict/SetItem popped BigInt refs, and
# BuildList popped struct-instance registry refs (leaked 14-50 MB each)
g = growth('b = 10 ** 430\\ni = 0\\nwhile i < 200000 { xs = [b + 1]\\n i = i + 1 }\\n1')
assert g < 8000, f"RSS grew by {g} kB over 200k BuildList BigInt elements"
g = growth('b = 10 ** 430\\nd = dict()\\ni = 0\\nwhile i < 100000 { d["k"] = b + 1\\n i = i + 1 }\\n1')
assert g < 8000, f"RSS grew by {g} kB over 100k SetItem BigInt values"
g = growth('struct P { a }\\ni = 0\\nwhile i < 200000 { xs = [P(1)]\\n i = i + 1 }\\n1')
assert g < 8000, f"RSS grew by {g} kB over 200k BuildList struct elements"
# error paths under an in-language except: the popped operand must be released
# before the Err propagates, or every caught error leaks it (Pos popped a
# struct instance and returned Err raw: +19 MB/200k before the fix; the
# registry Drop backstop only reclaims at the END of an execution)
g = growth('struct P { a }\\ni = 0\\nwhile i < 200000 { try { +P(1) } except { _ => { 0 } }\\n i = i + 1 }\\n1')
assert g < 8000, f"RSS grew by {g} kB over 200k caught unary-plus struct errors"
print('ok')
"""
    result = subprocess.run(
        [sys.executable, '-c', script],
        capture_output=True,
        text=True,
        timeout=120,
    )
    assert result.returncode == 0, result.stderr
    assert 'ok' in result.stdout

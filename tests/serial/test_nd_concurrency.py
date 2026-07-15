# FILE: tests/serial/test_nd_concurrency.py
"""
Tests for ND-recursion with 3 execution modes.

Modes:
- sequential: Single-threaded, no concurrency
- threads: ThreadPoolExecutor, shared memory, GIL-limited
- processes: ProcessPoolExecutor, true parallelism
"""

import pytest

from catnip import Catnip


def exec_catnip(code: str):
    """Helper to execute Catnip code."""
    catnip = Catnip()
    catnip.parse(code)
    return catnip.execute()


def _factorial_for_nd(n, recur):
    """Module-level factorial for ND tests (must be picklable)."""
    if n <= 1:
        return 1
    else:
        return n * recur(n - 1)


def _fibonacci_for_nd(n, recur):
    """Module-level fibonacci for ND tests (must be picklable)."""
    if n <= 1:
        return n
    else:
        return recur(n - 1) + recur(n - 2)


class TestNDModeSequential:
    """Sequential mode tests (baseline)."""

    def test_factorial_sequential(self):
        """Basic factorial with sequential mode (default)."""
        code = """
        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_explicit_sequential_mode(self):
        """Explicit pragma for sequential mode."""
        code = """
        pragma("nd_mode", ND.sequential)

        ~~(7, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 5040  # 7!

    def test_sequential_alias_seq(self):
        """'seq' alias for sequential mode."""
        code = """
        pragma("nd_mode", ND.sequential)

        ~~(5, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 120  # 5!


class TestNDModeThreads:
    """Thread mode tests (ThreadPoolExecutor)."""

    def test_factorial_thread(self):
        """Factorial with threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_thread_alias_thread(self):
        """'thread' alias for threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        ~~(6, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 720  # 6!

    def test_broadcast_thread(self):
        """Broadcast with threads mode."""
        code = """
        pragma("nd_mode", ND.thread)

        list(5, 6, 7, 8).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """
        result = exec_catnip(code)
        assert result == [120, 720, 5040, 40320]

    def test_memoization_thread(self):
        """Memoization should be shared across threads."""
        code = """
        pragma("nd_mode", ND.thread)
        pragma("nd_memoize", True)

        # Fibonacci benefits from memoization
        ~~(10, (n, recur) => {
            if n <= 1 { n }
            else { recur(n - 1) + recur(n - 2) }
        })
        """
        result = exec_catnip(code)
        assert result == 55  # fib(10)


class TestNDModeProcesses:
    """Process mode tests (ProcessPoolExecutor)."""

    def test_factorial_process(self):
        """Factorial with processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800  # 10!

    def test_process_alias_process(self):
        """'process' alias for processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(8, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 40320  # 8!

    def test_process_alias_parallel(self):
        """'parallel' alias for processes mode (backwards compat)."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(5, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 120  # 5!

    def test_process_alias_par(self):
        """'par' alias for processes mode."""
        code = """
        pragma("nd_mode", ND.process)

        ~~(6, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 720  # 6!

    def test_process_non_freezable_capture_falls_back(self):
        """A callback capturing a non-freezable value (a function) can't ship to
        a worker; the batch falls back rather than resolving the name against the
        worker's builtins. Result must match sequential -- guards against the
        'global shadowing a builtin diverges' non-bug (a captured non-freezable
        forces the fallback, so the worker path never resolves it wrong)."""
        code = """
        pragma("nd_mode", ND.MODE)
        helper = (x) => { x * 10 }
        list(1, 2, 3).[~~ (n, recur) => { helper(n) }]
        """
        proc = exec_catnip(code.replace("MODE", "process"))
        seq = exec_catnip(code.replace("MODE", "sequential"))
        assert proc == seq == [10, 20, 30]

    def test_process_shadowed_builtin_matches_sequential(self):
        """A user global shadowing a builtin (`str`) referenced by the callback
        can't ship to a fresh worker, which would resolve the name to its OWN
        builtin and diverge silently (no error to trigger the reactive fallback).
        has_shadowed_builtin_callable() detects it up front and forces the
        thread path. Must match sequential in both VM and AST modes -- guards the
        symmetric builtin-shadow hole in both engines."""
        code = """
        pragma("nd_mode", ND.MODE)
        str = (x) => { x * 100 }
        list(1, 2, 3).[~~ (n, recur) => { str(n) }]
        """
        proc = exec_catnip(code.replace("MODE", "process"))
        seq = exec_catnip(code.replace("MODE", "sequential"))
        assert proc == seq == [100, 200, 300]

    def test_process_global_struct_matches_sequential(self):
        """A global struct referenced by the callback is installed into the worker
        (thawed against its registry, reclaimed each task) and the refcounted
        results are released. Must match sequential -- guards the worker leak
        fixes (install_frozen_globals + release_owned)."""
        code = """
        pragma("nd_mode", ND.MODE)
        struct Point { x }
        g = Point(10)
        list(5, 6, 7, 8).[~~ (n, recur) => {
            if n <= 1 { g.x }
            else { n * recur(n - 1) }
        }]
        """
        proc = exec_catnip(code.replace("MODE", "process"))
        seq = exec_catnip(code.replace("MODE", "sequential"))
        assert proc == seq == [1200, 7200, 50400, 403200]


class TestNDSchedulerAPI:
    """Direct NDScheduler API tests."""

    def test_scheduler_mode_sequential(self):
        """Sequential execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='sequential')

        result = scheduler.execute_sync(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_mode_thread(self):
        """Thread execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='threads')

        result = scheduler.execute_thread(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_mode_process(self):
        """Process execution via NDScheduler."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=4, mode='processes')

        # Uses module-level function (picklable)
        result = scheduler.execute_process(5, _factorial_for_nd)
        assert result == 120

    def test_scheduler_set_mode(self):
        """set_mode() changes execution mode."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler(n_workers=2)
        assert scheduler.mode == 'sequential'

        scheduler.set_mode('thread')
        assert scheduler.mode == 'thread'

        scheduler.set_mode('process')
        assert scheduler.mode == 'process'

    def test_scheduler_invalid_mode(self):
        """Invalid mode raises ValueError."""
        from catnip.nd import NDScheduler

        scheduler = NDScheduler()

        with pytest.raises(ValueError):
            scheduler.set_mode('invalid')


class TestNDPragmas:
    """ND pragma tests."""

    def test_pragma_nd_workers(self):
        """nd_workers pragma configures worker count."""
        code = """
        pragma("nd_workers", 8)

        ~~(10, (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        })
        """
        result = exec_catnip(code)
        assert result == 3628800

    def test_pragma_nd_mode_with_workers(self):
        """Combined nd_mode and nd_workers pragmas."""
        code = """
        pragma("nd_mode", ND.thread)
        pragma("nd_workers", 4)

        list(5, 6, 7).[~~ (n, recur) => {
            if n <= 1 { 1 }
            else { n * recur(n - 1) }
        }]
        """
        result = exec_catnip(code)
        assert result == [120, 720, 5040]


class TestNDDeterminism:
    """Tests to ensure all modes produce the same results."""

    def test_all_modes_same_result(self):
        """All 3 modes should produce identical results."""
        factorial_code = """
        pragma("nd_mode", ND.{mode})

        ~~(8, (n, recur) => {{
            if n <= 1 {{ 1 }}
            else {{ n * recur(n - 1) }}
        }})
        """

        expected = 40320  # 8!

        for mode in ('sequential', 'thread', 'process'):
            code = factorial_code.format(mode=mode)
            result = exec_catnip(code)
            assert result == expected, f"Mode {mode} returned {result}, expected {expected}"

    def test_broadcast_all_modes(self):
        """Broadcast should work identically in all modes."""
        broadcast_code = """
        pragma("nd_mode", ND.{mode})

        list(3, 4, 5).[~~ (n, recur) => {{
            if n <= 1 {{ 1 }}
            else {{ n * recur(n - 1) }}
        }}]
        """

        expected = [6, 24, 120]  # 3!, 4!, 5!

        for mode in ('sequential', 'thread', 'process'):
            code = broadcast_code.format(mode=mode)
            result = exec_catnip(code)
            assert result == expected, f"Mode {mode} returned {result}, expected {expected}"


class TestNDThreadRegistries:
    """Thread workers inherit the parent's symbol_table, enum_registry and
    nullary-union-method bindings via NdRegistryHandle.

    Two tests are genuine regression guards (verified to fail without the
    corresponding propagation): a returned union variant demotes to a raw symbol
    index without symbol_table, and loses its methods without
    UNION_NULLARY_METHODS. The enum and struct tests are coverage, not isolating
    guards: nullary enum identity resolves via symbol_table (enum_registry is
    carried for parity with the non-worker nested-call path, which matters for
    imported-module enums, a case not exercised here), and func_table /
    struct_registry were already propagated before this change.
    """

    @staticmethod
    def _run(mode, body):
        # Build the source by concatenation (no f-string/format) so catnip's own
        # braces need no escaping.
        return exec_catnip('pragma("nd_mode", ND.' + mode + ')\n' + body)

    UNION_BODY = """
union Color { red; green; blue }
list(0, 1, 2).[~~ (n, recur) => {
    if n == 0 { Color.red } elif n == 1 { Color.green } else { Color.blue }
}]
"""

    def test_thread_union_variant_not_demoted(self):
        """symbol_table: union variants returned from Thread workers keep their identity."""
        seq = self._run('sequential', self.UNION_BODY)
        thr = self._run('thread', self.UNION_BODY)
        assert [str(x) for x in seq] == ['Color.red', 'Color.green', 'Color.blue']
        assert [str(x) for x in thr] == [str(x) for x in seq]
        # Without symbol_table propagation, resolve_symbol returns None and the
        # variant demotes to its raw symbol-index int.
        assert not any(isinstance(x, int) for x in thr), f"variant demoted to int: {thr}"

    UNION_METHOD_BODY = """
union Color {
    red; green; blue;
    label(self) => { "is-a-color" }
}
list(0, 1, 2).[~~ (n, recur) => {
    if n == 0 { Color.red } elif n == 1 { Color.green } else { Color.blue }
}]
"""

    def test_thread_union_variant_keeps_methods(self):
        """UNION_NULLARY_METHODS: a variant's methods survive the Thread-worker round-trip."""
        thr = self._run('thread', self.UNION_METHOD_BODY)
        # Without the method-table propagation the variant resolves its NAME but
        # to_pyobject rebuilds it method-less, so .label() raises AttributeError.
        assert [v.label() for v in thr] == ['is-a-color', 'is-a-color', 'is-a-color']

    ENUM_BODY = """
enum Status { active; done; failed }
list(0, 1, 2).[~~ (n, recur) => {
    v = if n == 0 { Status.active } elif n == 1 { Status.done } else { Status.failed }
    list(typeof(v), v == Status.active)
}]
"""

    def test_thread_enum_variant_resolves(self):
        """Coverage: enum variants resolve their type and compare equal inside a worker."""
        seq = self._run('sequential', self.ENUM_BODY)
        thr = self._run('thread', self.ENUM_BODY)
        assert seq == [['Status', True], ['Status', False], ['Status', False]]
        assert thr == seq

    STRUCT_BODY = """
struct Point { x; y }
list(1, 2, 3).[~~ (n, recur) => { Point(n, n * n) }]
"""

    def test_thread_struct_roundtrip(self):
        """struct_registry (already propagated): structs created in workers transplant back."""
        seq = self._run('sequential', self.STRUCT_BODY)
        thr = self._run('thread', self.STRUCT_BODY)
        assert [(p.x, p.y) for p in seq] == [(1, 1), (2, 4), (3, 9)]
        assert [(p.x, p.y) for p in thr] == [(p.x, p.y) for p in seq]


class TestNDThreadStructStress:
    """Hammer the shared StructRegistry from parallel Thread workers.

    Guards the interior-mutability refactor (RefCell fields + `&self` release
    paths + `unsafe impl Send/Sync`): the soundness claim is that every RefCell
    borrow is GIL-serialized, so no rayon worker races the non-atomic borrow
    flag. These tests create AND destroy structs inside the callback across many
    workers and many rounds, so the registry churns (create_instance, decref
    cascade into pyobj fields, transplant_to_parent) under real contention. A
    borrow-flag race or a reentrant double-borrow surfaces as a panic or a
    thread/sequential divergence.

    No isolating guard exists for a race (a race is nondeterministic); these are
    empirical stress coverage. Thread vs sequential equality catches both a
    worker panic (raised back as an exception) and any registry corruption that
    changes a result.
    """

    @staticmethod
    def _run(mode, body):
        return exec_catnip('pragma("nd_mode", ND.' + mode + ')\npragma("nd_workers", 8)\n' + body)

    # Callback churns the registry: builds structs whose fields hold a
    # list-of-structs (decref cascades through the list into sibling slots),
    # reassigns a temporary struct (StoreScope decrefs the displaced Box, whose
    # cascade releases its list mid-store -- the reentrant path the RefCell
    # refactor closes), then returns an int derived from the surviving struct.
    CHURN_BODY = """
struct Leaf { v }
struct Box { items; tag }

results = []
for round in range(40) {
    r = range(150).[~~ (n, recur) => {
        a = Leaf(n)
        bundle = list(a, Leaf(n * 2), Leaf(n + 1))
        box = Box(bundle, Leaf(n))
        tmp = Box(list(Leaf(n), Leaf(n)), Leaf(0))
        tmp = Leaf(-1)
        box.tag.v + box.items[2].v
    }]
    results.append(r)
}
results
"""

    def test_thread_struct_churn_matches_sequential(self):
        """Registry churn + reentrant cascade under 8 workers must equal sequential."""
        expected = [[2 * n + 1 for n in range(150)] for _ in range(40)]
        seq = self._run('sequential', self.CHURN_BODY)
        assert seq == expected
        thr = self._run('thread', self.CHURN_BODY)
        assert thr == seq

    # Callback returns freshly-created nested structs, exercising
    # transplant_to_parent (the child registry drains its slots into the parent
    # under GIL) on every element, across rounds.
    TRANSPLANT_BODY = """
struct Leaf { v }
struct Box { items; tag }

results = []
for round in range(30) {
    r = range(120).[~~ (n, recur) => {
        Box(list(Leaf(n), Leaf(n * 2)), Leaf(n + 1))
    }]
    results.append(r)
}
results
"""

    def test_thread_struct_transplant_matches_sequential(self):
        """Structs returned from workers transplant back intact under contention."""
        seq = self._run('sequential', self.TRANSPLANT_BODY)
        thr = self._run('thread', self.TRANSPLANT_BODY)

        def shape(rounds):
            return [[(b.tag.v, b.items[0].v, b.items[1].v) for b in r] for r in rounds]

        expected = [[(n + 1, n, n * 2) for n in range(120)] for _ in range(30)]
        assert shape(seq) == expected
        assert shape(thr) == shape(seq)


class TestNDRefcountLedger:
    """Ledger guards for the ND paths (fixes 2026-07-13).

    The native process worker result loop thaws each FrozenValue into an owned
    Value, clones it into the result list, and must release the thawed value --
    skipping that stranded one ObjectTable handle per str/list result element
    (measured: +batch_size slots per run). The NdRecursion/NdMap opcodes must
    release their popped operands like the Broadcast arm does.

    If no worker binary is resolvable, process mode falls back to the thread
    path, which must be ledger-clean too: the guard stays meaningful.
    """

    def _ledger_delta(self, code, runs=3):
        import gc

        from catnip import _rs

        def counts():
            # Triple collect (same as test_refcount_proptest): flushes deferred
            # finalizers from earlier tests in the same xdist worker, so the
            # window only sees this test's allocations.
            for _ in range(3):
                gc.collect()
            return _rs._debug_live_counts()

        exec_catnip(code)  # warmup: worker pool, tables, caches
        base = counts()
        for _ in range(runs):
            exec_catnip(code)
        after = counts()
        return tuple(a - b for a, b in zip(after, base))

    def test_process_string_results_ledger_clean(self):
        """Native worker str results must not strand ObjectTable handles."""
        code = """
        pragma("nd_mode", ND.process)
        range(10).[~> (n) => { "x" + str(n) }]
        """
        assert self._ledger_delta(code) == (0, 0, 0, 0, 0)

    def test_nd_combinator_operands_ledger_clean(self):
        """~~/~> combinator forms must release their popped pyobj operands."""
        code = """
        pragma("nd_mode", ND.sequential)
        a = ~~("abc", (s, recur) => { s + "!" })
        b = ~>(list("a", "b", "c"), (x) => { x + "!" })
        list(a, b)
        """
        assert self._ledger_delta(code) == (0, 0, 0, 0, 0)

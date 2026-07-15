# FILE: tests/language/test_gc_context.py
"""
Regression tests for cyclic-GC collection of the execution Context.

Rust pyclasses are opaque to CPython's cyclic collector: any that can reach the
Context (a struct type via its methods, a function/bound method via its captured
context, the import loader, the runtime) must implement `__traverse__`/`__clear__`
or they pin the whole Context for the life of the process -- a silent leak that
matters for long-lived hosts (REPL, server, MCP).

Two bugs in this class were found only by hand before these tests existed: the
generic Context cycles, and the over-counted ObjectTable handle when a type was
bound under two names. Each case below builds a session, drops it, and asserts
the Context is collected -- so a future ctx-reaching pyclass that forgets its GC
hooks fails CI instead of leaking in production.
"""

import gc
import os
import weakref

import pytest

from catnip import Catnip

# Each case exercises a distinct path from a global binding back to the Context.
# Historically these leaked: a method-bearing type bound to a name pins the
# Context through `type -> method -> context`, and a bound method through
# `func`/`instance`.
GC_CASES = {
    'type_as_result': 'struct P{x\nf(self)=>{1}}\nP',
    'type_aliased': 'struct P{x\nf(self)=>{1}}\nv=P',
    'type_aliased_twice': 'struct P{x\nf(self)=>{1}}\na=P\nb=P',
    'union_variant_type': 'union U{V(x)\nf(self)=>{1}}\nU.V',
    'union_variant_call': 'union U{V(x)\nf(self)=>{self.x}}\nU.V(1).f()',
    'bound_method': 'struct P{x\nf(self)=>{1}}\nm=P(1).f',
    'function_aliased': 'g=(a)=>{a+1}\nh=g',
    'two_types_aliased': 'struct P{x\nf(self)=>{1}}\nstruct Q{y\ng(self)=>{2}}\nw=Q\nv=P',
    'extends_with_method': 'struct B{x\ng(self)=>{self.x}}\nstruct C extends(B){y}\nc=C',
}


@pytest.mark.parametrize('code', GC_CASES.values(), ids=list(GC_CASES))
def test_context_is_collected(code, vm_mode):
    """A finished session's Context must be reclaimable by the cyclic GC."""
    c = Catnip(vm_mode=vm_mode)
    c.parse(code)
    c.execute()
    ref = weakref.ref(c.context)
    del c
    gc.collect()
    assert ref() is None, "Context pinned after the session was dropped (GC cycle leak)"


def test_no_context_accumulates_across_many_sessions(vm_mode):
    """Repeated method-bearing-type sessions must not accumulate live Contexts."""
    refs = []
    for _ in range(30):
        c = Catnip(vm_mode=vm_mode)
        c.parse('struct P{x\nf(self)=>{self.x}}\nv=P\nv(7).f()')
        c.execute()
        refs.append(weakref.ref(c.context))
        del c
    gc.collect()
    alive = sum(1 for r in refs if r() is not None)
    assert alive == 0, f"{alive}/30 Contexts still pinned after GC"


def test_live_session_survives_collect(vm_mode):
    """The mirror invariant of the collection cases: a cyclic GC pass must
    never strip a LIVE session of its builtins.

    The import wrapper cluster is only reachable from the pipeline through the
    VM globals map (opaque Rust): without `PyPipeline.__traverse__` reporting
    the map's handles, the cluster looks like a closed dead cycle, the
    collector clears the loader, and every builtin lookup afterwards raises
    NameError (symptom moved with the GC's allocation thresholds -- found via
    check_doc_assertions on FOLD_GUIDE, 2026-07-02).
    """
    c = Catnip(vm_mode=vm_mode)
    c.parse('y = 2')
    c.execute()
    gc.collect()
    c.parse('len([1, 2, 3])')
    assert c.execute() == 3, "builtins lost after a GC pass on a live session"


def test_live_pipelines_survive_collect_in_batch():
    """Several live raw pipelines sharing one GC pass all keep their builtins
    (the historical reproduction: dead-looking wrapper cycles per pipeline)."""
    from catnip._rs import Pipeline

    pipes = [Pipeline() for _ in range(3)]
    for p in pipes:
        p.execute_quiet('y = 2')
    gc.collect()
    for p in pipes:
        assert p.execute_quiet('len([1])') == 1, "builtins lost after GC on live pipeline"


class _RefWitness:
    """Plain Python object whose CPython refcount is the oracle."""


class _RaisingWitness:
    """Witness whose comparison/arith dunders raise, for error-path probes."""

    def __eq__(self, other):
        raise ValueError("eq")

    def __ne__(self, other):
        raise ValueError("ne")

    def __neg__(self):
        raise ValueError("neg")

    def __contains__(self, item):
        raise ValueError("in")

    __hash__ = object.__hash__


def _witness_delta(code, extra=None, expect_error=False, witness_cls=_RefWitness):
    """Refcount delta of a witness pyobj across a full session (0 = balanced).
    The delta is captured before the assert (pytest's assertion rewriting
    re-evaluates expressions, which would re-measure), and the collector runs
    several passes (finalizer chains)."""
    import sys

    from catnip.context import Context

    w = witness_cls()

    class _Ctx(Context):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.globals['w'] = w
            if extra:
                self.globals.update(extra)

    for _ in range(3):
        gc.collect()
    base = sys.getrefcount(w)
    c = Catnip(context=_Ctx())
    c.parse(code)
    if expect_error:
        with pytest.raises(Exception):
            c.execute()
    else:
        r = c.execute()
        del r
    del c
    for _ in range(3):
        gc.collect()
    return sys.getrefcount(w) - base


def test_globals_refcount_balances_after_session():
    """A pyobj stored in a top-level global must not be retained after the
    session dies: the VM globals map owns one ref per entry, released at the
    VM's destruction (Drop), and the Halt sync takes its own ref instead of
    aliasing the slot's."""
    assert _witness_delta('x = w\n1') == 0


def test_globals_resync_after_reentrant_call_balances():
    """The post-call globals resync (GLOBALS_GEN) releases the old local and
    gives self.globals its OWN ref of the resynced value -- one ref for two
    owning slots was a latent double release."""

    def caller(f):
        return f(1)

    delta = _witness_delta(
        'flag = 0\nx = w\npoke = (v) => { flag = 1\n v }\ncaller(poke)\nx',
        extra={'caller': caller},
    )
    assert delta == 0


def test_type_redefinition_releases_old_entry():
    """Redefining a struct releases the overwritten VM-globals entry (the old
    type object stays alive through its immortal marker-map ref)."""
    assert _witness_delta('x = w\nstruct P { a }\nstruct P { a; b }\n1') == 0


def test_toplevel_pyobj_reassignment_balances():
    """Reassigning a top-level pyobj global must not leak: the StoreScope
    eq-skip releases the redundant popped ref (StoreLocal;LoadLocal;StoreScope
    parks the value in the slot first)."""
    assert _witness_delta('x = w\nx = 1\nx = w\nx = None\n1') == 0


def test_global_struct_instance_field_ref_balance():
    """A struct instance that transits the host globals must not retain its
    registry slot -- nor a ref of its pyobj field -- past teardown.

    Fixed 2026-07-07 (was: +1 per session in VM mode). Two mechanisms, both
    consequences of `Value::decref` being a deliberate no-op on TAG_STRUCT
    (struct release = registry in hand):

    - `StructRegistry::drop` dropped surviving slots' fields raw; it now
      releases their non-struct heap fields (the terminal backstop -- oracle
      `registry_drop_releases_surviving_slot_heap_fields`, structs.rs).
    - `VMHost::store_global`/`delete_global` released the displaced map entry
      with the struct-no-op `Value::decref`; the `VmHost` trait now hands the
      displaced entry back to the dispatch, which releases it registry-aware
      (`decref_discard`) -- so reassignment no longer strands slots mid-session
      on long-lived hosts (REPL/MCP)."""
    assert _witness_delta('struct P { a }\np = P(w)\n1') == 0
    assert _witness_delta('struct P { a }\np = P(w)\np = P(1)\np = None\n1') == 0


def test_broadcast_snapshot_setattr_balances():
    """Broadcast child registries are field-by-field bit-copies of the parent
    (clone_from_parent, shared global counts). The snapshot owns one count per
    heap field, so a child-side release (SetAttr overwrite, cascade) consumes
    the child's ref -- never the parent's -- and broadcast sessions reclaim
    surviving slot fields at death (the parent backstop is no longer disarmed
    by spawning a child; was +1 per instance per session)."""
    # Witness in a sibling field; the callback mutates another.
    assert (
        _witness_delta('struct P { a; log }\nitems = list(P(w, 0), P(w, 0))\nitems.[(p) => { p.log = 1\n 2 }]\n1') == 0
    )
    # Witness IS the overwritten field: the release must consume the child's count.
    assert _witness_delta('struct P { log }\nitems = list(P(w), P(w))\nitems.[(p) => { p.log = 1\n 2 }]\n1') == 0
    # Pure traversal, no mutation: the untouched snapshot balances at child death.
    assert _witness_delta('struct P { a }\nitems = list(P(w), P(w))\nitems.[(p) => { p.a }]\n1') == 0


def test_broadcast_result_value_balances():
    """The broadcast callback's result Value is owned (execute_with_host
    contract); VMFunction.__call__ must release its non-struct heap count
    after to_pyobject, or every invocation strands one count (+1 pinned Py
    per handle -- was +1 per callback on plain pyobjs)."""
    assert _witness_delta('items = list(w, w)\nitems.[(p) => { p }]\n1') == 0


def test_broadcast_setattr_parent_field_stays_live():
    """A child-side SetAttr releases its own snapshot count, not the parent's:
    reading the field back after the broadcast returns the original, live
    object (no dangling slot). VM only: the VM's broadcast children are
    snapshots (mutation invisible to the parent), while the AST interpreter
    mutates the shared instance in place -- a pre-existing semantic
    divergence, traced in wip/DEFAUTS_SOURNOIS.md."""
    from catnip.context import Context

    w = _RefWitness()

    class _Ctx(Context):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.globals['w'] = w

    c = Catnip(context=_Ctx(), vm_mode='on')
    c.parse('struct P { log }\nitems = list(P(w), P(w))\nitems.[(p) => { p.log = 1\n 2 }]\nitems[0].log')
    assert c.execute() is w


def test_globals_balance_on_gc_teardown_path():
    """Same witness balance when the session dies through the CYCLIC GC path
    (__clear__ drains host.globals while the VM is still alive) instead of
    plain refcounting: every store owns one ref per entry, so both teardown
    orders release exactly what was taken (wip/GLOBALS_OWNERSHIP.md)."""
    import sys

    from catnip.context import Context

    w = _RefWitness()

    class _Ctx(Context):
        def __init__(self, **kwargs):
            super().__init__(**kwargs)
            self.globals['w'] = w

    for _ in range(3):
        gc.collect()
    base = sys.getrefcount(w)
    c = Catnip(context=_Ctx())
    # A method-bearing struct pins the Context in a cycle (the GC_CASES
    # pattern), forcing the __clear__ drain path; x holds the witness.
    c.parse('struct P { a\n f(self) => { 1 } }\nx = w\nv = P\n1')
    r = c.execute()
    del r, c
    for _ in range(3):
        gc.collect()
    assert sys.getrefcount(w) - base == 0


def test_closure_store_paths_do_not_corrupt():
    """The three closure-store hazards from the ownership review: a store to a
    PyGlobals/Globals-backed name from inside a function (the popped ref is
    consumed by the transfer, every other store takes its own ref BEFORE it),
    and a mutable-closure overwrite (the captured map owns its entries since
    MakeFunction clones at capture). These used to double-release: the checks
    here are no-crash plus the values staying correct."""
    c = Catnip()
    c.parse('x = 1\nf = () => { tmp = x\n x = tmp + 41\n x }\nf()\nx')
    assert c.execute() == 42
    # The captured overwrite is observed through inner's own return value:
    # whether the write propagates back to outer's local is a pre-existing
    # VM/AST divergence (traced in wip/DEFAUTS_SOURNOIS.md), not this test's
    # subject.
    c2 = Catnip()
    c2.parse('outer = () => { c = "a"\n inner = () => { tmp = c\n c = tmp + "b"\n c }\n inner() }\nouter()')
    assert c2.execute() == 'ab'


def test_type_redefinition_keeps_old_instances_working():
    """Redefining a type purges the old marker-map entry and releases the old
    ref: live instances of the OLD type keep dispatching through the Python
    slow path (pinning instead would retain the whole context cluster)."""
    assert (
        _witness_delta(
            'struct P { a\n f(self) => { self.a } }\n'
            'p1 = P(41)\n'
            'struct P { a; b\n f(self) => { self.b } }\n'
            'p1.f() + 1'
        )
        == 0
    )


def test_function_return_does_not_accumulate_leftover_locals():
    """The Return handler releases the frame's leftover owned pyobj locals
    (Voie A, same as the end-of-bytecode path): a bare drop leaked one ref per
    leftover pyobj local on EVERY return, so N calls leaked N refs. The delta
    must not grow with the call count (a constant residual, traced in
    wip/GLOBALS_OWNERSHIP.md, is tolerated here -- the accumulation is not)."""
    one = _witness_delta('f = () => { tmp = w\n 1 }\nf()\n1')
    many = _witness_delta('f = () => { tmp = w\n 1 }\nf()\nf()\nf()\nf()\nf()\n1')
    assert many == one
    assert one <= 1


def test_block_local_heap_value_does_not_leak():
    """A block-local slot holding a heap value is released when the block pops.
    push_block takes an independent refcount for each saved slot; pop_block
    releases the overwritten locals before restoring the snapshot."""
    assert _witness_delta('f = () => { tmp = w\n 1 }\nf()\n1') == 0


def test_match_capture_binding_does_not_leak():
    """A `match` that binds the subject to a variable must not retain a ref.
    BindMatch clones each binding into its slot (a guarded arm binds twice); the
    owned refs kept by match_bindings are released when it is overwritten or the
    frame is torn down. A prior version cloned without releasing, leaking one ref
    per capture-binding match."""
    assert _witness_delta('match w { y => { y\n 1 } }') == 0
    assert _witness_delta('match w { y if y != None => { 1 } }') == 0
    # Distinct match sites and repeated matches must not accumulate.
    assert _witness_delta('match w { y => { 1 } }\nmatch w { z => { 1 } }') == 0
    assert _witness_delta('f = () => { match w { y => { 1 } } }\nf()\nf()\nf()\n1') == 0


def test_match_non_capturing_subject_does_not_leak():
    """The subject is a DupTop copy owned by MatchPatternVM. When no capture
    moves it into a binding (wildcard, literal, guard-fail fallthrough) it is
    released there instead of leaking one ref per match."""
    assert _witness_delta('match w { _ => { 1 } }') == 0
    assert _witness_delta('match w { 1 => { 1 }\n _ => { 2 } }') == 0
    assert _witness_delta('match w { y if False => { 1 }\n z => { 2 } }') == 0


def test_match_tuple_item_not_consumed_by_binding_does_not_leak():
    """A tuple-pattern item is an owned from_pyobject ref. When no binding takes
    it (wildcard/literal sub-pattern, star coverage, full mismatch) it must be
    released by the op, not abandoned in the matcher."""
    # heap item under a wildcard sub-pattern (the nominal +1 per successful match)
    assert _witness_delta('xs = [w, 1]\nmatch xs { (_, y) => { 1 } }\n1') == 0
    # heap item under a literal sub-pattern
    assert _witness_delta('xs = [w, 1]\nmatch xs { (_, 1) => { 1 } }\n1') == 0
    # heap item covered by a star
    assert _witness_delta('xs = [1, w, 2]\nmatch xs { (a, *mid, b) => { 1 } }\n1') == 0
    # full mismatch: no item is consumed, all must still be released
    assert _witness_delta('xs = [w, 9]\nmatch xs { (1, 2) => { 1 }\n _ => { 2 } }\n1') == 0
    # repeated matches must not accumulate
    assert _witness_delta('xs = [w, 1]\nf = () => { match xs { (_, y) => { 1 } } }\nf()\nf()\nf()\n1') == 0


def test_operand_popping_opcodes_release_heap_operands():
    """TypeOf, FormatValue (f-string interpolation), ToBool (and/or),
    SetAttr (non-struct receiver) and Broadcast popped heap operands without
    ever releasing them: +1 per use, once per loop iteration in loops."""
    assert _witness_delta('f = () => { typeof(w)\n 1 }\nf()\n1') == 0
    assert _witness_delta('f = () => { s = f"{w}"\n 1 }\nf()\n1') == 0
    assert _witness_delta('f = () => { w and 1\n 1 }\nf()\n1') == 0
    assert _witness_delta('f = () => { 1 and w\n 1 }\nf()\n1') == 0
    assert _witness_delta('f = () => { w or 1\n 1 }\nf()\n1') == 0
    assert _witness_delta('f = () => { w.attr = 5\n 1 }\nf()\n1') == 0
    # value operand: the second setattr drops the carrier's ref on w, so only
    # an opcode leak would remain (the carrier itself outlives the session)
    assert _witness_delta('f = () => { o.attr = w\n o.attr = 1\n 1 }\nf()\n1', extra={'o': _RefWitness()}) == 0
    assert _witness_delta('f = () => { xs = [w, 1]\n xs.[(v) => { v }]\n 1 }\nf()\n1') == 0


def test_compare_error_paths_release_operands():
    """Eq/Ne/Neg/In/NotIn released their popped operands only AFTER the
    fallible Python dunder call: a raising dunder leaked both operands."""
    for code in (
        'f = () => { w == 1\n 1 }\nf()\n1',
        'f = () => { w != 1\n 1 }\nf()\n1',
        'f = () => { -w\n 1 }\nf()\n1',
        'f = () => { 1 in w\n 1 }\nf()\n1',
        'f = () => { 1 not in w\n 1 }\nf()\n1',
    ):
        assert _witness_delta(code, expect_error=True, witness_cls=_RaisingWitness) == 0, code


def test_bitwise_error_paths_release_operands():
    """The bitwise arms (BOr/BXor/BAnd/BNot/LShift/RShift) have no Python
    fallback consuming pyobj refs: their bare Err(TypeError) path must release
    the popped operands itself (the pyobj-excluding helper of the arith arms,
    whose fallback does consume, leaked them here)."""
    for code in (
        'f = () => { w | 1\n 1 }\nf()\n1',
        'f = () => { 1 | w\n 1 }\nf()\n1',
        'f = () => { w & 1\n 1 }\nf()\n1',
        'f = () => { w ^ 1\n 1 }\nf()\n1',
        'f = () => { w << 1\n 1 }\nf()\n1',
        'f = () => { w >> 1\n 1 }\nf()\n1',
        'f = () => { ~w\n 1 }\nf()\n1',
    ):
        assert _witness_delta(code, expect_error=True) == 0, code


def test_getattr_error_path_releases_receiver():
    """GetAttr's non-struct branch released the popped receiver only on
    success: the obj_getattr AttributeError (and the early Err exits) bypassed
    the release point."""
    assert _witness_delta('f = () => { w.missing\n 1 }\nf()\n1', expect_error=True) == 0


def test_setattr_unknown_struct_field_releases_value():
    """SetAttr's struct arm released the popped receiver but not the popped
    value on the unknown-field error branch (the sibling frozen branch
    releases both)."""
    assert _witness_delta('struct P { a }\nf = () => { p = P(1)\n p.nofield = w\n 1 }\nf()\n1', expect_error=True) == 0


def test_unpack_assign_subject_does_not_leak():
    """MatchAssignPatternVM (star/nested patterns) and UnpackSequence pop the
    subject -- a DupTop copy or the loaded value -- and never released it: +1
    per unpacking. Function bodies show it directly; at top level the GC cycle
    collection masked it for fresh subjects while host-reachable ones stayed
    retained (the persistent form)."""
    # star pattern -> MatchAssignPatternVM; the leaked subject copy retains xs
    assert _witness_delta('f = () => { xs = [w, 1, 2]\n (p, *q) = xs\n 1 }\nf()\n1') == 0
    # flat pattern -> UnpackSequence
    assert _witness_delta('f = () => { xs = [w, 1]\n (p, q) = xs\n 1 }\nf()\n1') == 0
    # top-level, fresh direct subject
    assert _witness_delta('(p, *q) = [w, 1]\n1') == 0
    # for-loop tuple unpacking goes through the same opcodes, once per iteration
    assert _witness_delta('xs = [[w, 1], [w, 2]]\nfor (p, q) in xs { p }\n1') == 0
    assert _witness_delta('xs = [[w, 1, 2]]\nfor (p, *q) in xs { p }\n1') == 0


def test_unpack_assign_item_not_consumed_by_binding_does_not_leak():
    """vm_match_assign_pattern items are owned from_pyobject refs; one not
    transferred to a binding (star coverage, compound sub-pattern) leaked."""
    # heap item covered by the star
    assert _witness_delta('xs = [1, w, 2]\n(p, *q, r) = xs\n1') == 0
    # outer item consumed by a compound sub-pattern, not by a direct binding
    assert _witness_delta('xs = [1, [w, 2]]\n(p, (q, r)) = xs\n1') == 0


def test_match_midway_failure_releases_partial_bindings():
    """A compound pattern that bails after collecting bindings (a later item
    mismatches) must release those partial bindings -- each owns its ref."""
    # (x, 2) against [w, 1]: x=w already bound when 2 mismatches
    assert _witness_delta('xs = [w, 1]\nmatch xs { (x, 2) => { 1 }\n _ => { 2 } }\n1') == 0
    # same shape through an Or: the failed alternative's partials must not leak
    assert _witness_delta('xs = [w, 1]\nmatch xs { (x, 2) | (y, 1) => { 1 } }\n1') == 0
    # struct pattern bailing on a missing field after binding one
    assert _witness_delta('struct P { a; b }\np = P(w, 1)\nmatch p { P{a, b} => { 1 } }\n1') == 0


def test_tail_call_released_locals_do_not_leak():
    """A tail call reuses the caller's frame; its old locals must be RELEASED,
    not NIL-overwritten. When an enclosing local (a captured pyobj) is live at a
    tail call whose result propagates out, a bare NIL overwrite (and a truncating
    resize) leaked one ref -- the tail-call path, not letrec (mis-diagnosed as an
    Rc self-cycle). The clean mirrors already balanced and must stay 0."""
    # leaking shape: enclosing local x=w, nested f captures x, f() in tail position
    assert _witness_delta('o = () => { x = w\n f = () => { x }\n f() }\no()\n1') == 0
    # mirrors that balance for a different reason and must not regress
    assert _witness_delta('o = () => { f = () => { w }\n f() }\no()\n1') == 0  # direct global read, no capture
    assert (
        _witness_delta('o = () => { x = w\n f = () => { x }\n r = f()\n r }\no()\n1') == 0
    )  # f() out of tail position
    assert _witness_delta('o = () => { x = w\n f = () => { x }\n f(1)\n 99 }\no()\n1') == 0  # result discarded
    # disabling TCO was already balanced (the leak was TCO-only)
    assert _witness_delta('pragma("tco", False)\no = () => { x = w\n f = () => { x }\n f() }\no()\n1') == 0


def test_tail_call_leak_balances_over_distinct_objects():
    """The tail-call local leak pinned one ref for the process lifetime, so a
    long-lived host tail-returning captured objects would grow. N outers each
    capturing a distinct global and tail-returning it must all balance; the
    tracked witness is measured single-ref-style (no retaining container)."""
    import sys

    from catnip.context import Context

    def delta_tracked(n, tracked):
        w = _RefWitness()

        class _Ctx(Context):
            def __init__(self, **kwargs):
                super().__init__(**kwargs)
                for i in range(n):
                    self.globals[f'w{i}'] = w if i == tracked else _RefWitness()

        code = '\n'.join(f'o{i} = () => {{ x = w{i}\n f = () => {{ x }}\n f() }}\no{i}()' for i in range(n)) + '\n1'
        for _ in range(3):
            gc.collect()
        base = sys.getrefcount(w)
        c = Catnip(context=_Ctx())
        c.parse(code)
        c.execute()
        del c
        for _ in range(3):
            gc.collect()
        return sys.getrefcount(w) - base

    for n in (1, 4, 8):
        for tracked in (0, n // 2, n - 1):
            assert delta_tracked(n, tracked) == 0, f"tail-call leak at n={n}, tracked={tracked}"


def test_letrec_self_reference_does_not_leak():
    """Regression guard against re-chasing the 'letrec Rc self-cycle' phantom:
    self-reference itself never leaked (the residual was the tail-call path,
    fixed above). Named self-recursion and mutual recursion must balance."""
    assert _witness_delta('f = (n) => { if n <= 0 { w } else { f(n - 1) } }\nf(1)\n1') == 0
    assert (
        _witness_delta(
            'is_even = (n) => { if n == 0 { w } else { is_odd(n - 1) } }\n'
            'is_odd = (n) => { if n == 0 { w } else { is_even(n - 1) } }\nis_even(2)\n1'
        )
        == 0
    )


def test_vmhost_globals_pending_ref_reclaimed_only_by_collection():
    """The refcount-pure teardown path of the VMHost globals map.

    Unlike the witness probes above (which exercise the ContextHost -- a Python
    dict, reclaimed by CPython refcounting), the raw `Pipeline`'s `host.globals`
    is a Rust `Rc<RefCell<IndexMap<String, Value>>>` holding OBJECT_TABLE handles.
    It has NO non-GC releaser: `drain_globals` runs only from the two cyclic-GC
    `__clear__` paths (`PyPipeline`, `ImportLoader`), deliberately not from a Drop
    (draining early would strip the handles the collector needs to report -- see
    wip/GLOBALS_OWNERSHIP.md, "Contrat de teardown honnête").

    So a witness stored in `host.globals` and torn down by plain refcounting with
    the cyclic GC disabled must sit as ONE pending host ref -- memory-safe, not
    lost -- and a subsequent collection reclaims it exactly. The witness probes
    validate the GC path (delta 0); this test locks the refcount-pure side.
    """
    import sys

    from catnip._rs import Pipeline

    w = _RefWitness()
    for _ in range(3):
        gc.collect()
    base = sys.getrefcount(w)

    gc.disable()
    try:
        p = Pipeline()
        # GlobalsProxy.__setitem__ stores the witness as an owned OBJECT_TABLE
        # handle aliased in host.globals (the exact map with no non-GC drainer).
        p.globals()['w'] = w
        del p
        # Plain refcount teardown, GC disabled: drain_globals never runs, so the
        # host ref stays pending. The witness is retained, not leaked-and-lost.
        pending = sys.getrefcount(w) - base
    finally:
        gc.enable()
    assert pending == 1, f"expected exactly the pending host ref, got {pending}"

    for _ in range(3):
        gc.collect()
    assert sys.getrefcount(w) - base == 0, "a collection must reclaim the pending host ref"


def test_globals_proxy_struct_overwrite_releases_displaced():
    """Overwriting a struct global through the Python-facing GlobalsProxy must
    release the displaced instance.

    `from_pyobject` increfs the slot on every store, but `old.decref()` is a
    no-op on TAG_STRUCT (a struct release needs its registry in hand). So a
    struct-blind proxy pinned the instance: its refcount grew +1 per assign and
    never came back (measured 4->9 over 5 overwrites, on both __setitem__ and
    update). The fix installs the proxy's registry as the conversion thread-local
    so the stored struct is indexed into it, then releases the displaced value
    against that registry (`proxy_registry_decref`) -- an overwrite loop is
    rc-neutral.

    The global count ledger (`_debug_live_counts`) cannot witness this: it counts
    live instance SLOTS, not the sum of refcounts, and this leak inflates one live
    slot's rc without allocating a new one. The named struct-rc sum
    (`_debug_struct_rc_sum`, the struct counterpart of OBJECT_TABLE's `refs`) is
    the metric that closes the blind spot.
    """
    from catnip._rs import Pipeline

    p = Pipeline()
    proxy = p.execute('struct P { x }\np_inst = P(42)\np_inst')
    g = p.globals()

    # __setitem__ overwrite loop must be refcount-neutral.
    g['s'] = proxy
    before = p._debug_struct_rc_sum()
    for _ in range(5):
        g['s'] = proxy
    assert p._debug_struct_rc_sum() == before, f"__setitem__ leaked {p._debug_struct_rc_sum() - before}"

    # update() overwrite loop must be refcount-neutral too.
    before = p._debug_struct_rc_sum()
    for _ in range(5):
        g.update(dict(s=proxy))
    assert p._debug_struct_rc_sum() == before, f"update leaked {p._debug_struct_rc_sum() - before}"


def test_globals_proxy_releases_native_struct_global():
    """A struct global stored purely in-VM (never materialized to a Python
    proxy) must still be released when overwritten through the proxy.

    Releasing via the registry-identity table only resolves a registry that is
    IN that table -- populated when a proxy is materialized. A pipeline whose
    struct never round-tripped to Python was absent, so the struct-aware release
    silently no-op'd and the slot leaked exactly as before the fix.
    `Pipeline::globals()` now registers the registry unconditionally, so the
    native struct is reclaimed on overwrite.
    """
    from catnip._rs import Pipeline

    p = Pipeline()
    # Last result is int 0, so no CatnipStruct proxy is ever materialized.
    p.execute('struct Point { x; y }\npt = Point(1, 2)\n0')
    rc_before = p._debug_struct_rc_sum()
    assert rc_before > 0, "expected the native Point instance to be live"

    g = p.globals()
    g['pt'] = 42  # overwrite the struct global with a non-struct

    assert (
        p._debug_struct_rc_sum() < rc_before
    ), f"native struct was not released ({rc_before} -> {p._debug_struct_rc_sum()})"


def test_globals_proxy_release_targets_only_its_own_registry():
    """A struct written through pipeline A's proxy while a DIFFERENT pipeline's
    registry is the ambient thread-local must never decref an unrelated instance
    in A on displacement.

    Without installing the proxy's registry as the conversion thread-local,
    `from_pyobject` indexed the foreign struct into pipeline B's index space,
    but the release targeted A's registry by id -- an in-range-but-unrelated
    index collision decref'd a live A instance (measured victim rc 2 -> 1, a
    leak-to-corruption regression). The fix installs A's registry so the stored
    struct is re-homed into A; the release then only touches A's own slot.
    """
    from catnip._rs import Pipeline

    def a_slots(pl):
        return {idx: rc for idx, rc, _ in pl._debug_struct_slots()}

    pa = Pipeline()
    pa.execute('struct P { x }\nvictim = P(7)\nvictim')
    victim_before = a_slots(pa)
    ga = pa.globals()

    pb = Pipeline()
    # pb executes last, so the ambient thread-local registry is B's when the
    # cross-write below runs.
    proxy_b = pb.execute('struct P { x }\nq = P(99)\nq')

    ga['s'] = proxy_b  # foreign struct, ambient registry = B
    ga['s'] = 0  # displace it -> must not touch victim

    victim_after = a_slots(pa)
    for idx, rc in victim_before.items():
        assert (
            victim_after.get(idx) == rc
        ), f"unrelated A instance {idx} was mis-decref'd ({rc} -> {victim_after.get(idx)})"
    assert pa.execute('victim.x') == 7, "victim instance was corrupted by a cross-registry release"


def test_ledger_struct_rc_sum_catches_pure_rc_pin():
    """The named struct-rc sum closes the ledger blind spot the live-slot COUNT
    leaves open: a pure-rc pin on a PERSISTENT live instance. Re-storing the same
    proxy into a persistent struct global is rc-neutral (fixed) -- the live-slot
    count never moves (the instance is always referenced), so the summed rc is the
    only load-bearing signal. A regression of the displaced-release would climb
    `_debug_struct_rc_sum` while `_debug_struct_slots` stays flat (verified: the
    metric goes non-zero, the count does not, when the release is reverted)."""
    from catnip._rs import Pipeline

    p = Pipeline()
    proxy = p.execute('struct P { x }\np_inst = P(1)\np_inst')
    g = p.globals()
    g['s'] = proxy

    count_before = len(p._debug_struct_slots())
    rc_before = p._debug_struct_rc_sum()
    for _ in range(10):
        g['s'] = proxy

    # The blind spot: the live-slot COUNT is unchanged (the instance is always live).
    assert len(p._debug_struct_slots()) == count_before, "live-slot count must stay flat -- the blind spot"
    # The metric that sees through it: summed rc back to baseline.
    assert p._debug_struct_rc_sum() == rc_before, f"struct-rc sum leaked {p._debug_struct_rc_sum() - rc_before}"


def test_extension_proxy_struct_overwrite_is_refcount_neutral():
    """An extension `register()` hook overwriting a struct global through
    `ctx.globals` (the extension `GlobalsProxy`) must release the displaced
    instance struct-aware.

    Those proxies were built struct-blind (`registry_id = 0`): a struct release
    no-ops without its registry in hand, so re-storing a struct global via an
    extension pinned the instance -- its refcount grew +1 per assign and never
    came back (a benign but real leak). The loader now threads the feeder VM's
    struct registry id into the import/extension proxies, so an overwrite loop is
    refcount-neutral. Measured on the named struct-rc sum after the hook
    (`_debug_struct_rc_sum`); the live-instance count ledger is blind to a pure-rc
    pin, and the worker-path proxies stay `0` (see wip/DEFAUTS_SOURNOIS.md).
    """
    import sys
    import types

    from catnip import Catnip

    def run_with_restores(n):
        # Same pipeline shape every time -- only the re-store count varies, so
        # the final struct-slot refcounts are identical unless a re-store leaks.
        mod_name = f'_catnip_test_ext_restore_{n}'
        m = types.ModuleType(mod_name)

        def register(ctx):
            proxy = ctx.globals['pt']  # the live Point, as a struct proxy
            for _ in range(n):
                ctx.globals['pt'] = proxy  # re-store: displaces the previous entry

        m.__catnip_extension__ = dict(
            name=f'restore-{n}',
            version='0.1.0',
            register=register,
        )
        sys.modules[mod_name] = m
        try:
            # VM-only: the leak is a struct-slot refcount in the VM registry;
            # AST mode keeps globals in Python dicts with no such slots.
            c = Catnip(vm_mode='on')
            c.parse(f'struct Point {{ x; y }}\npt = Point(1, 2)\nimport("{mod_name}")\n0')
            c.execute()
            # Read after execute: reading mid-hook re-enters the borrowed registry.
            return c._pipeline._debug_struct_rc_sum()
        finally:
            sys.modules.pop(mod_name, None)

    base = run_with_restores(0)
    many = run_with_restores(20)
    assert base, "expected a live Point slot"
    assert base == many, f"extension overwrite leaked: {base} (0 re-stores) vs {many} (20)"


def test_python_callable_args_are_released():
    """Owned args passed to a generic Python callable must be released by the
    call opcode after conversion (`to_pyobject` only reads them).

    Fixed 2026-07-07 (was: +1 per heap arg per call in VM mode -- the leak
    behind the broadcast-transplant lead: in a broadcast child, a struct
    constructor resolves as a Python callable instead of native CallStruct, so
    every `P(w)` leaked its argument's ref). Covers the Call, TailCall (tail
    position in a lambda body), CallKw, and CallMethod dispatch sites."""

    class Receiver:
        def m(self, x):
            return None

        def mkw(self, a=None):
            return None

    extra = dict(pyid=lambda x: x, pykw=lambda a=None: None, o=Receiver())
    assert _witness_delta('s = pyid(w)\ns = None\n1', extra=extra) == 0
    assert _witness_delta('f = () => { pyid(w) }\nf()\n1', extra=extra) == 0
    assert _witness_delta('s = pykw(a=w)\ns = None\n1', extra=extra) == 0
    assert _witness_delta('f = () => { pykw(a=w) }\nf()\n1', extra=extra) == 0
    assert _witness_delta('s = o.m(w)\ns = None\n1', extra=extra) == 0
    assert _witness_delta('f = () => { o.m(w) }\nf()\n1', extra=extra) == 0
    assert _witness_delta('s = o.mkw(a=w)\ns = None\n1', extra=extra) == 0


def test_excess_and_vararg_args_are_released():
    """Args beyond the parameter count are accepted and discarded; the frame
    binding must release their moved refs instead of dropping them raw. The
    vararg slot re-boxes excess args into a PyList holding independent refs,
    so the originals are released too. A kwarg displacing a positional slot
    releases the displaced owned value (fixed 2026-07-07)."""
    assert _witness_delta('f = (x) => { 1 }\ns = f(1, w)\ns = None\n1') == 0
    assert _witness_delta('g = (x) => { 1 }\nf = () => { g(1, w) }\nf()\n1') == 0
    assert _witness_delta('f = (*xs) => { 1 }\ns = f(w)\ns = None\n1') == 0
    assert _witness_delta('g = (*xs) => { 1 }\nf = () => { g(w) }\nf()\n1') == 0
    assert _witness_delta('f = (x) => { 1 }\ns = f(w, x=2)\ns = None\n1') == 0


def test_struct_type_heap_default_is_released_at_teardown():
    """A struct type owns one ref per heap field default (evaluated at the
    definition); `StructRegistry::drop` releases them -- shadowed (redefined)
    types stay in the vec and are covered by the same pass. Inherited fields
    are cloned into the child type WITH their own default ref (mirroring
    inherited methods' clone_ref), so the per-type release is balanced even
    through a diamond MRO (fixed 2026-07-07; the missing incref crashed
    test_super_init_chaining by over-release)."""
    assert _witness_delta('struct S { a; b = w }\n1') == 0
    assert _witness_delta('struct S { a; b = w }\nx = S(1)\ny = S(a=2)\nx = None\ny = None\n1') == 0
    assert _witness_delta('struct S { b = w }\nstruct S { b = w }\nx = S()\nx = None\n1') == 0
    assert (
        _witness_delta(
            'struct A { b = w }\nstruct B extends(A) { }\nstruct C extends(A) { }\n'
            'struct D extends(B, C) { }\nd = D()\nd = None\n1'
        )
        == 0
    )


def test_broadcast_child_struct_creation_balances():
    """The original broadcast-transplant lead: a struct created inside a
    broadcast child (`items.[(x) => { P(w) }]`) resolves `P` through the
    closure chain as a Python callable -- the constructor call must release
    its owned args like every Python-callable dispatch (fixed 2026-07-07,
    was +1 per session)."""
    delta = _witness_delta('struct P { a }\nitems = [1, 2]\nr = items.[(x) => { P(w) }]\nr = None\n1')
    assert delta == 0


def test_call_error_paths_release_owned_args():
    """A failing call must release its in-flight popped operands: the error
    exits of the call opcodes' fast paths (struct arity/kwargs/typed fields/
    abstract guard) and of CallMethod's lookup failures used to drop them raw
    (fixed 2026-07-07 -- one ref per heap arg per failed call). Covers Call,
    TailCall (tail position), CallKw, and CallMethod on both struct and
    Python receivers."""

    class Receiver:
        pass

    extra = dict(o=Receiver())
    cases = [
        'struct P { a; b }\nP(w)\n1',
        'struct P { a }\nP(w, 2)\n1',
        'struct P { a }\nP(z=w)\n1',
        'struct P { a }\nP(w, a=1)\n1',
        'struct P { a }\nP(1, a=w)\n1',
        'struct P { a; b }\nP(a=w)\n1',
        'struct P { a: int }\nP(w)\n1',
        'struct A { a; @abstract m(self) }\nA(w)\n1',
        'struct P { a }\np = P(1)\np.nope(w)\n1',
        'o.nope(w)\n1',
        'struct P { a; b }\nf = () => { P(w) }\nf()\n1',
        'struct P { a }\nf = () => { P(w, a=1) }\nf()\n1',
    ]
    for code in cases:
        assert _witness_delta(code, extra=extra, expect_error=True) == 0, code


# ---------------------------------------------------------------------------
# Refcount ledger -- exact live counts of the manually refcounted heap classes
# ---------------------------------------------------------------------------
#
# `_rs._debug_live_counts()` reads (occupied OBJECT_TABLE slots, summed handle
# refcounts, live BigInt allocations, live Complex allocations, live struct
# instance slots). Unlike the witness probes above (one tracked pyobj) or the
# RSS witness (volume with a threshold), the ledger counts every allocation of
# every class exactly: a forgotten operand release in ANY opcode shows up as a
# non-zero per-class delta on whichever grid form exercises it, without
# knowing the object's identity in advance.
#
# Two oracles: `_ledger_delta` measures a full session lifecycle (create ->
# execute -> drop -> collect), `_ledger_intra_delta` measures repeated runs on
# a REUSED pipeline -- the only boundary where struct-instance leaks are
# visible, since the registry (and its slots) dies with the pipeline.

BALANCED = (0, 0, 0, 0, 0)


def _ledger_counts():
    from catnip import _rs

    for _ in range(3):
        gc.collect()
    return _rs._debug_live_counts()


def _ledger_delta(code, expect_error=False):
    """Ledger delta across a full session (BALANCED = every ref released).
    A warm-up session absorbs one-time initializations (symbol interning,
    caches, builtin singletons) so the measured session is steady-state."""

    def run():
        c = Catnip()
        c.parse(code)
        if expect_error:
            with pytest.raises(Exception):
                c.execute()
        else:
            r = c.execute()
            del r
        del c

    run()
    base = _ledger_counts()
    run()
    return tuple(a - b for a, b in zip(_ledger_counts(), base))


def _ledger_intra_delta(code, runs=3, expect_error=False):
    """Ledger delta across `runs` executions of the SAME pipeline (steady
    state reached by two warm-up runs: the first populates the globals, the
    second exercises their overwrite path). Catches per-run leaks the session
    boundary cannot see -- a leaked struct-registry count is reclaimed
    wholesale when the registry dies with the pipeline."""

    c = Catnip()
    c.parse(code)

    def run(pipeline):
        if expect_error:
            with pytest.raises(Exception):
                pipeline.execute()
        else:
            r = pipeline.execute()
            del r

    run(c)
    run(c)
    base = _ledger_counts()
    for _ in range(runs):
        run(c)
    delta = tuple(a - b for a, b in zip(_ledger_counts(), base))
    del c
    return delta


def test_ledger_bigint_arith_chain_balances():
    """Chained BigInt arithmetic frees every intermediate allocation (the
    operand-audit family: leaked whole allocations, invisible to
    getrefcount)."""
    assert _ledger_delta('x = 10**25\ny = x * x + x - 1\nz = y % 7\n1') == BALANCED
    assert _ledger_delta('acc = 10**25\nfor i in range(20) { acc = acc + i * 10**20 }\n1') == BALANCED


def test_ledger_complex_arith_balances():
    """Complex intermediates are Arc allocations like BigInt; the same chain
    forms must balance."""
    assert _ledger_delta('z = 2j\ny = z * z + 1j - z\n1') == BALANCED
    assert _ledger_delta('acc = 0j\nfor i in range(20) { acc = acc + 1j }\n1') == BALANCED


def test_ledger_collections_of_heap_elements_balance():
    """Build/index/setitem on collections holding BigInt elements release the
    element refs (the collection-opcode family)."""
    assert _ledger_delta('xs = [10**25, 10**26]\nxs[0] + xs[1]\n1') == BALANCED
    assert _ledger_delta('xs = [10**25]\nxs[0] = 10**26\n1') == BALANCED
    assert _ledger_delta('d = {"a": 10**25}\nd["a"] + 1\n1') == BALANCED


def test_ledger_unpack_and_match_balance():
    """for-unpack, assign-unpack and match subjects/bindings (the family that
    leaked per iteration)."""
    assert _ledger_delta('xs = [[10**25, 1], [10**26, 2]]\nfor (a, b) in xs { a + b }\n1') == BALANCED
    assert _ledger_delta('xs = [10**25, 10**26]\n(a, b) = xs\n1') == BALANCED
    assert _ledger_delta('match 10**25 { x if x > 0 => { x + 1 }\n_ => { 0 } }\n1') == BALANCED


def test_ledger_functions_and_structs_balance():
    """Function calls (locals freed on every return path), closures, struct
    fields holding heap values, and type redefinition."""
    assert _ledger_delta('f = (n) => { n + 10**25 }\nf(1)\nf(2)\n1') == BALANCED
    assert _ledger_delta('struct P { a }\np = P(10**25)\nq = p.a + 1\np = None\n1') == BALANCED
    assert _ledger_delta('struct P { a = 10**25 }\nstruct P { a = 10**26 }\nx = P()\nx = None\n1') == BALANCED


def test_ledger_strings_exercise_object_table():
    """String building transits the OBJECT_TABLE; slot and handle counts must
    return to baseline."""
    assert _ledger_delta('s = "abc" + "def"\nt = f"{s}-{s}"\nu = t * 3\n1') == BALANCED


def test_ledger_broadcast_balances():
    """Broadcast owns and releases its popped refs (the B1 double-free/leak
    pair)."""
    assert _ledger_delta('xs = [10**25, 10**26]\nr = xs.[(x) => { x * x }]\nr = None\n1') == BALANCED


def test_ledger_error_paths_balance():
    """Error exits release in-flight operands (the error-path family)."""
    assert _ledger_delta('x = 10**25\nx < "a"\n1', expect_error=True) == BALANCED
    assert _ledger_delta('x = 10**25\nx | 1.5\n1', expect_error=True) == BALANCED
    assert _ledger_delta('struct P { a; b }\nP(10**25)\n1', expect_error=True) == BALANCED


def test_ledger_struct_instances_intra_session_balance():
    """Per-run struct-instance balance on a reused pipeline: the registry dies
    with the pipeline, so a leaked registry count is only visible here. The
    global-store form leaked +1/run through the wrapper's globals re-injection
    (`from_pyobject` on the proxy increfs the slot, the displaced entry was
    released with a struct-blind `decref`), the bare-result form through
    `consume_result` (same blindness on the Halt-popped ref)."""
    assert _ledger_intra_delta('struct P { a }\np = P(1)\n1') == BALANCED
    assert _ledger_intra_delta('struct P { a }\nP(1)') == BALANCED
    assert _ledger_intra_delta('struct P { a }\nP(1)\n1') == BALANCED
    assert _ledger_intra_delta('struct P { a }\nfor i in range(10) { P(i) }\n1') == BALANCED
    assert _ledger_intra_delta('struct P { a }\nmatch P(1) { P{a} => { a }\n_ => { 0 } }\n1') == BALANCED
    assert _ledger_intra_delta('struct P { a }\nxs = [1, 2]\nr = xs.[(x) => { P(x) }]\nr = None\n1') == BALANCED
    assert _ledger_intra_delta('struct P { a }\np = P("xyz" * 3)\nq = p.a\np = None\nq = None\n1') == BALANCED


def test_ledger_struct_error_paths_intra_session_balance():
    """Error paths must not leak registry counts per run either -- including
    the caught-error form the RSS witness first flagged (`+P(1)` without
    op_pos, real intra-execution, invisible post-mortem)."""
    assert _ledger_intra_delta('struct P { a; b }\nP(1)\n1', expect_error=True) == BALANCED
    assert _ledger_intra_delta('struct P { a }\ntry { +P(1) } except { _ => { 0 } }\n1') == BALANCED


def test_ledger_broadcast_error_path_balances():
    """An erroring broadcast callback must release the popped target list (and
    its pinned struct elements): the in-VM fast path releases operands
    unconditionally, then propagates the error. A bare `?` before the release
    leaked the list handle per error -- unbounded on a pipeline that catches it."""
    assert _ledger_intra_delta('xs = [1, 2, 3]\nr = xs.[(x) => { x / 0 }]', expect_error=True) == BALANCED
    assert (
        _ledger_intra_delta('struct P { a }\nitems = [P(1), P(2)]\nr = items.[(p) => { p.a / 0 }]', expect_error=True)
        == BALANCED
    )

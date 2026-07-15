# FILE: tests/serial/test_refcount_proptest.py
"""Differential refcount property test.

Generates small, valid, deterministic Catnip programs biased toward the
historically leaky constructs (BigInt/Complex chains, collections of heap
elements, structs, match, closures, unpacking, broadcast, global overwrites)
and checks four oracles per program:

  O1  session ledger delta == 0 (VM mode: create -> run -> drop -> collect)
  O2  intra-session ledger delta == 0 (VM mode, repeated runs on ONE pipeline
      -- the only boundary where a leaked struct-registry count is visible)
  O3  result convergence: VM repr == AST repr
  O4  session ledger delta == 0 (AST mode)

The ledger is `_rs._debug_live_counts()` (OBJECT_TABLE slots, handle refs,
BigInt allocs, Complex allocs, struct instance slots). On failure the program
is line-shrunk before reporting, so the assertion shows a minimal repro.

Two by-design exclusions, deliberate in the generator:

- type-level defaults stay SCALAR: a heap default re-evaluated at every
  re-run of a def is retained by the append-only types vec until registry
  Drop -- a documented retention that would drown real per-run leaks in O2;
- no error paths: they have their own witness grids in test_gc_context.py.

Gate runs CASES seeds from SEED0; campaigns override via env:
  CATNIP_PROPTEST_CASES=300 CATNIP_PROPTEST_SEED=1000 pytest tests/serial/test_refcount_proptest.py
"""

import gc
import os
import random

import pytest

from catnip import Catnip, _rs

CASES = int(os.environ.get('CATNIP_PROPTEST_CASES', '12'))
SEED0 = int(os.environ.get('CATNIP_PROPTEST_SEED', '1'))
SIZE = 10
INTRA_RUNS = 3
ZERO = (0, 0, 0, 0, 0)


class _Gen:
    """Compositional program generator: every expression only references
    variables already defined with a compatible kind, so emitted programs are
    valid by construction (a NameError would test the resolver, not refcounts).
    """

    def __init__(self, seed):
        self.r = random.Random(seed)
        self.n = 0
        self.vars = {'big': [], 'cpx': [], 'str': [], 'listn': []}
        self.n_structs = 0
        self.insts = []  # (var, [field names])
        self.n_funcs = 0
        self.stmts = []

    def fresh(self):
        self.n += 1
        return f'v{self.n}'

    def pick(self, kind):
        xs = self.vars[kind]
        return self.r.choice(xs) if xs else None

    # -- expressions
    def big_expr(self):
        base = f'10**{self.r.randint(21, 30)} + {self.r.randint(0, 999)}'
        v = self.pick('big')
        if v and self.r.random() < 0.6:
            return self.r.choice([f'{v} + {base}', f'{v} * 3 + 1', f'({v} - 7) * 2', f'{v} % 10**12 + 10**21'])
        return base

    def cpx_expr(self):
        v = self.pick('cpx')
        if v and self.r.random() < 0.6:
            return self.r.choice([f'{v} + 2j', f'{v} * 1j', f'{v} - 0.5j + 1'])
        return f'{self.r.randint(1, 9)}j'

    def str_expr(self):
        v = self.pick('str')
        if v and self.r.random() < 0.5:
            return self.r.choice([f'{v} + "x{self.r.randint(0, 99)}"', f'{v} * 2'])
        return f'"s{self.r.randint(0, 999)}" * {self.r.randint(1, 3)}'

    def heap_expr(self):
        return self.r.choice([self.big_expr, self.cpx_expr, self.str_expr])()

    # -- statements (each registers new vars only AFTER building the
    #    expressions that could otherwise self-reference)
    def st_assign(self):
        kind, mk = self.r.choice([('big', self.big_expr), ('cpx', self.cpx_expr), ('str', self.str_expr)])
        expr = mk()
        v = self.fresh()
        self.vars[kind].append(v)
        self.stmts.append(f'{v} = {expr}')

    def st_overwrite(self):
        kinds = [k for k in ('big', 'cpx', 'str') if self.vars[k]]
        if not kinds:
            return self.st_assign()
        kind = self.r.choice(kinds)
        v = self.pick(kind)
        if self.r.random() < 0.3:
            self.stmts.append(f'{v} = None')
            self.vars[kind].remove(v)
        else:
            mk = {'big': self.big_expr, 'cpx': self.cpx_expr, 'str': self.str_expr}[kind]
            self.stmts.append(f'{v} = {mk()}')

    def st_list(self):
        items = ', '.join(self.big_expr() for _ in range(self.r.randint(2, 3)))
        v = self.fresh()
        self.vars['listn'].append(v)
        self.stmts.append(f'{v} = [{items}]')
        if self.r.random() < 0.5:
            self.stmts.append(f'{v}[{self.r.randint(0, 1)}] = {self.big_expr()}')
        if self.r.random() < 0.5:
            w = self.fresh()
            self.vars['big'].append(w)
            self.stmts.append(f'{w} = {v}[0] + 1')

    def st_dict(self):
        expr_a, expr_b = self.big_expr(), self.heap_expr()
        v = self.fresh()
        self.stmts.append(f'{v} = {{"a": {expr_a}, "b": {expr_b}}}')
        w = self.fresh()
        self.vars['big'].append(w)
        self.stmts.append(f'{w} = {v}["a"] + 2')

    def st_unpack(self):
        xs = self.fresh()
        self.stmts.append(f'{xs} = [{self.big_expr()}, {self.big_expr()}]')
        a, b = self.fresh(), self.fresh()
        self.vars['big'] += [a, b]
        self.stmts.append(f'({a}, {b}) = {xs}')

    def st_for_unpack(self):
        xs = self.fresh()
        self.stmts.append(f'{xs} = [[{self.big_expr()}, 1], [{self.big_expr()}, 2]]')
        acc, p, q = self.fresh(), self.fresh(), self.fresh()
        self.vars['big'].append(acc)
        self.stmts.append(f'{acc} = 0\nfor ({p}, {q}) in {xs} {{ {acc} = {acc} + {p} + {q} }}')

    def st_struct(self):
        tn = f'S{self.n_structs}'
        self.n_structs += 1
        fields = [f'f{i}' for i in range(self.r.randint(1, 2))]
        body = '; '.join(fields)
        if self.r.random() < 0.4:
            body += f'; fd = {self.r.randint(1, 99)}'  # scalar only, see module doc
            fields.append('fd')
            args = ', '.join(self.big_expr() for _ in fields[:-1])
        else:
            args = ', '.join(self.big_expr() for _ in fields)
        self.stmts.append(f'struct {tn} {{ {body} }}')
        sv = self.fresh()
        self.insts.append((sv, fields))
        self.stmts.append(f'{sv} = {tn}({args})')
        if self.r.random() < 0.6:
            w = self.fresh()
            self.vars['big'].append(w)
            self.stmts.append(f'{w} = {sv}.{fields[0]} + 1')
        if self.r.random() < 0.4:
            self.stmts.append(f'{sv}.{fields[0]} = {self.big_expr()}')

    def st_method_struct(self):
        tn = f'M{self.n_structs}'
        self.n_structs += 1
        self.stmts.append(f'struct {tn} {{ a\ngetv(self) => {{ self.a + 1 }} }}')
        arg = self.big_expr()
        sv = self.fresh()
        self.insts.append((sv, ['a']))
        self.stmts.append(f'{sv} = {tn}({arg})')
        w = self.fresh()
        self.vars['big'].append(w)
        self.stmts.append(f'{w} = {sv}.getv()')

    def st_func(self):
        fn = f'g{self.n_funcs}'
        self.n_funcs += 1
        cap = self.pick('big')
        body = f'n_ + {cap}' if (cap and self.r.random() < 0.5) else f'n_ * 2 + {self.r.randint(1, 9)}'
        self.stmts.append(f'{fn} = (n_) => {{ {body} }}')
        arg = self.big_expr()
        w = self.fresh()
        self.vars['big'].append(w)
        self.stmts.append(f'{w} = {fn}({arg})')

    def st_match(self):
        v = self.pick('big') or '7'
        arm = self.big_expr()
        w = self.fresh()
        self.vars['big'].append(w)
        self.stmts.append(f'{w} = match {v} % 3 {{ 0 => {{ {arm} }}\nx_ if x_ > 1 => {{ x_ + 1 }}\n_ => {{ 42 }} }}')

    def st_broadcast(self):
        xs = self.pick('listn')
        if not xs:
            return self.st_list()
        w = self.fresh()
        self.stmts.append(f'{w} = {xs}.[(e_) => {{ e_ * 2 + 1 }}]')
        y = self.fresh()
        self.vars['big'].append(y)
        self.stmts.append(f'{y} = {w}[0]')

    def st_struct_broadcast(self):
        # the ephemeral-host / materialized-copy family: instances flowing
        # through a broadcast callback, pass-through included
        if not self.insts:
            return self.st_struct()
        sv, fields = self.r.choice(self.insts)
        w = self.fresh()
        self.stmts.append(f'{w} = [{sv}].[(x_) => {{ x_ }}]')
        y = self.fresh()
        self.vars['big'].append(y)
        self.stmts.append(f'{y} = {w}[0].{fields[0]} + 0')

    def st_fstring(self):
        parts = [self.pick(k) for k in ('big', 'str', 'cpx')]
        parts = [p for p in parts if p]
        if not parts:
            return self.st_assign()
        inner = '|'.join('{' + p + '}' for p in parts)
        v = self.fresh()
        self.vars['str'].append(v)
        self.stmts.append(f'{v} = f"{inner}"')

    STATEMENTS = [
        (st_assign, 3),
        (st_overwrite, 2),
        (st_list, 2),
        (st_dict, 1),
        (st_unpack, 1),
        (st_for_unpack, 1),
        (st_struct, 2),
        (st_method_struct, 1),
        (st_func, 2),
        (st_match, 1),
        (st_broadcast, 1),
        (st_struct_broadcast, 1),
        (st_fstring, 1),
    ]

    def program(self, size=SIZE):
        pool = [f for f, w in self.STATEMENTS for _ in range(w)]
        for _ in range(size):
            self.r.choice(pool)(self)
        # deterministic final expression touching live state (feeds O3)
        tails = []
        for k in ('big', 'str', 'cpx'):
            v = self.pick(k)
            if v:
                tails.append('{' + v + '}')
        for sv, fields in self.insts[-2:]:
            tails.append('{' + f'{sv}.{fields[0]}' + '}')
        final = 'f"' + '#'.join(tails) + '"' if tails else '1'
        return '\n'.join(self.stmts + [final])


def _counts():
    for _ in range(3):
        gc.collect()
    return _rs._debug_live_counts()


def _run_once(code, vm_mode):
    c = Catnip(vm_mode=vm_mode)
    c.parse(code)
    r = c.execute()
    out = repr(r)
    del r, c
    return out


def _session_delta(code, vm_mode):
    _run_once(code, vm_mode)
    base = _counts()
    result = _run_once(code, vm_mode)
    return tuple(a - b for a, b in zip(_counts(), base)), result


def _intra_delta(code):
    c = Catnip()
    c.parse(code)
    c.execute()
    c.execute()
    base = _counts()
    for _ in range(INTRA_RUNS):
        r = c.execute()
        del r
    delta = tuple(a - b for a, b in zip(_counts(), base))
    del c
    return delta


def _violations(code):
    """All oracle violations for one program (empty list = clean)."""
    out = []
    d_vm, res_vm = _session_delta(code, 'on')
    if d_vm != ZERO:
        out.append(f'session VM delta {d_vm}')
    d_intra = _intra_delta(code)
    if d_intra != ZERO:
        out.append(f'intra VM delta {d_intra}')
    d_ast, res_ast = _session_delta(code, 'off')
    if d_ast != ZERO:
        out.append(f'session AST delta {d_ast}')
    if res_vm != res_ast:
        out.append(f'VM/AST divergence: VM={res_vm!r} AST={res_ast!r}')
    return out


def _shrink(code):
    """Line-removal shrink while ANY oracle still fails (a removal that makes
    the program invalid raises and is rejected)."""
    lines = code.split('\n')
    changed = True
    while changed:
        changed = False
        for i in reversed(range(len(lines))):
            cand = lines[:i] + lines[i + 1 :]
            if not cand:
                continue
            try:
                if _violations('\n'.join(cand)):
                    lines = cand
                    changed = True
            except Exception:
                pass
    return '\n'.join(lines)


@pytest.mark.xdist_group('refcount_proptest')
@pytest.mark.parametrize('seed', range(SEED0, SEED0 + CASES))
def test_generated_program_balances_and_converges(seed):
    """One generated program through the four oracles; minimal repro on failure."""
    code = _Gen(seed).program()
    bad = _violations(code)
    if bad:
        small = _shrink(code)
        pytest.fail(
            f"seed {seed}: {'; '.join(bad)}\n" f"--- minimized repro (re-check: {_violations(small)}):\n{small}"
        )

# FILE: tests/language/test_broadcast_mutation.py
"""
Sémantique de mutation des éléments struct dans un callback broadcast
(décision 2026-07-07, option A) : le callback reçoit une copie shallow
privée de l'élément — les mutations ne s'échappent pas vers la collection
source ; retourner la copie mutée la transporte dans le résultat.

Grille différentielle : les mêmes formes doivent converger en mode VM et
AST (le troisième exécuteur, la VM pure, a sa grille dans
catnip_vm/src/pipeline/tests.rs).
"""

import pytest

from catnip import Catnip


def both_modes(code):
    results = []
    for mode in ('on', 'off'):
        c = Catnip(vm_mode=mode)
        c.parse(code)
        results.append(c.execute())
    assert results[0] == results[1], f"VM={results[0]} != AST={results[1]}"
    return results[0]


CASES = [
    # (label, code, attendu)
    (
        'map : mutation invisible sur la source',
        'struct P { log }\nitems = [P(0), P(0)]\nitems.[(p) => { p.log = 1 }]\nitems[0].log',
        0,
    ),
    (
        'filter : mutation invisible sur la source',
        'struct P { log }\nitems = [P(0), P(0)]\nr = items.[if (p) => { p.log = 1\nTrue }]\nitems[0].log',
        0,
    ),
    (
        'map : retourner la copie mutee la transporte',
        'struct P { log }\nitems = [P(0)]\nr = items.[(p) => { p.log = 5\np }]\nr[0].log',
        5,
    ),
    (
        'map identite : le resultat est une copie detachee',
        'struct P { log }\nitems = [P(0)]\nr = items.[(p) => { p }]\nitems[0].log = 9\nr[0].log',
        0,
    ),
    (
        'map : creation fraiche intacte',
        'struct P { log }\nitems = [1, 2]\nr = items.[(x) => { P(x) }]\nr[1].log',
        2,
    ),
    (
        'transform : source intacte, resultat mute',
        'struct P { log }\nitems = [P(3)]\nitems2 = items.[(p) => { p.log = p.log + 1\np }]\n'
        'items[0].log * 10 + items2[0].log',
        34,
    ),
    (
        'nd map ~> : mutation invisible sur la source',
        'struct P { log }\nitems = [P(0), P(0)]\nr = ~>(items, (p) => { p.log = 1\n0 })\nitems[0].log',
        0,
    ),
]


@pytest.mark.parametrize('label,code,expected', CASES, ids=[c[0] for c in CASES])
def test_broadcast_struct_element_copy_semantics(label, code, expected):
    assert both_modes(code) == expected


class TestExecutionBoundary:
    """Règle générale (décision 2026-07-07, extension de la décision 4) :
    rester en Catnip = référence partagée ; traverser Python = copie privée.

    Un callback Catnip invoqué DEPUIS du Python arbitraire (HOF du contexte)
    reçoit des copies shallow de ses arguments struct — comme en VM, où le
    child re-entrant snapshote. Les appels internes (dispatch, init, tail
    calls) gardent le partage.
    """

    CASES = [
        # (label, code, attendu)
        ('direct partage', 'struct P { log }\np = P(0)\nf = (q) => { q.log = 1 }\nf(p)\np.log', 1),
        ('methode partage', 'struct P { log; m(self) => { self.log = 1 } }\np = P(0)\np.m()\np.log', 1),
        ('hof python isole', 'struct P { log }\np = P(0)\nhof((q) => { q.log = 1 }, p)\np.log', 0),
        ('hof each isole', 'struct P { log }\np = P(0)\nhof_each((q) => { q.log = 1 }, [p])\np.log', 0),
        ('builtin map isole', 'struct P { log }\np = P(0)\nr = map((q) => { q.log = 1 }, [p])\nlist(r)\np.log', 0),
    ]

    @pytest.mark.parametrize('label,code,expected', CASES, ids=[c[0] for c in CASES])
    def test_boundary_rule_converges(self, label, code, expected):
        from catnip.context import Context

        def hof(f, x):
            return f(x)

        def hof_each(f, xs):
            for x in xs:
                f(x)

        for mode in ('on', 'off'):

            class Ctx(Context):
                def __init__(self, **kw):
                    super().__init__(**kw)
                    self.globals.update(dict(hof=hof, hof_each=hof_each))

            c = Catnip(vm_mode=mode, context=Ctx())
            c.parse(code)
            assert c.execute() == expected, f'vm_mode={mode}'


class TestNestedDeepCopy:
    """Mutation d'un struct imbriqué : isolation profonde (2026-07-11, (5,1)).
    Un map ne mute pas sa source à AUCUNE profondeur ; la copie mutée est
    transportée dans le résultat. Les trois exécuteurs convergent (VM, AST,
    PureVM) : le redesign in-VM du broadcast catnip_rs VM
    (wip/BROADCAST_STRUCT_CALLBACK.md) aligne la VM sur AST/PureVM. Chaque cas
    encode [résultat, source].
    """

    CASES = [
        (
            'nested mute',
            'struct Q { x }\nstruct P { q }\nitems = [P(Q(1))]\nr = items.[(p) => { p.q.x = 5\np }]\n[r[0].q.x, items[0].q.x]',
            [5, 1],
        ),
        (
            'nested fresh assign',
            'struct Q { x }\nstruct P { q }\nitems = [P(Q(1))]\nr = items.[(p) => { p.q = Q(5)\np }]\n[r[0].q.x, items[0].q.x]',
            [5, 1],
        ),
        (
            'nested DAG identity',
            'struct Q { x }\nstruct P { a; b }\nq = Q(1)\nitems = [P(q, q)]\nr = items.[(p) => { p.a.x = 9\np }]\n[r[0].b.x, items[0].a.x]',
            [9, 1],
        ),
        (
            'two levels deep',
            'struct R { y }\nstruct Q { r }\nstruct P { q }\nitems = [P(Q(R(1)))]\nr = items.[(p) => { p.q.r.y = 7\np }]\n[r[0].q.r.y, items[0].q.r.y]',
            [7, 1],
        ),
        (
            'nested-list target (deep at container depth)',
            'struct Q { x }\nstruct P { q }\ngrid = [[P(Q(1))]]\nr = grid.[(p) => { p.q.x = 5\np }]\n[r[0][0].q.x, grid[0][0].q.x]',
            [5, 1],
        ),
    ]

    @pytest.mark.parametrize('label,code,expected', CASES, ids=[c[0] for c in CASES])
    def test_nested_deep_isolation_converges(self, label, code, expected):
        assert both_modes(code) == expected


def test_broadcast_frozen_struct_copy_stays_frozen():
    """A hashed (frozen) struct passed through a broadcast keeps its frozen flag
    in the private copy: mutating it in the callback is rejected in every
    executor. The deep snapshot must preserve `frozen` (like AST's
    `detached_copy` and the old child-VM `clone_from_parent`), or the copy would
    become silently mutable and break the hash/eq invariant."""
    code = 'struct P { a }\np = P(1)\nd = {p: 1}\nr = [p].[(x) => { x.a = 99\nx }]\nr[0].a'
    for mode in ('on', 'off'):
        c = Catnip(vm_mode=mode)
        c.parse(code)
        with pytest.raises(Exception):
            c.execute()

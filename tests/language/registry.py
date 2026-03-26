# FILE: tests/language/registry.py
from catnip.context import Context
from catnip.nodes import Op, Ref
from catnip.registry import Registry

ctx = Context()
ctx.local = dict(a=1, b=4)

reg = Registry(ctx)
debug = ctx.globals['debug']

debug("test")

op = Op("add", ((1, 2, 3),), ())
op = Op("set_local", ("a", op), ())
print(op)
reg.resolve_stmt(op)

op = Op("sub", ((1, 2, 3),), ())
op = Op("set_local", ("b", op), ())
print(op)
reg.resolve_stmt(op)

op = Op("add", (("d", "e", "f"),), ())
op = Op("set_local", ("c", op), ())
print(op)
reg.resolve_stmt(op)

print(ctx.locals)

print(reg.resolve_stmt(Op("neg", (99,))))
print(reg.resolve_stmt(Op("inv", (-99,))))
print(reg.resolve_stmt(Op("pos", (99,))))

print(reg.resolve_stmt(Op("mul", ((17, 2, 0.88),))))
print(reg.resolve_stmt(Op("truediv", ((17, 2, 0.88),))))
print(reg.resolve_stmt(Op("floordiv", ((17, 2, 0.88),))))
print(reg.resolve_stmt(Op("mod", ((17, 2, 0.88),))))

print(reg.resolve_stmt(Op("pow", ((2, 3, 4),))))

print(reg.resolve_stmt(Op("bool_not", (True,))))
print(reg.resolve_stmt(Op("bool_or", ((False, True),))))
print(reg.resolve_stmt(Op("bool_and", ((True, True, False),))))

print(reg.resolve_stmt(Op("bit_or", ((1, 8, 512),))))
print(reg.resolve_stmt(Op("bit_xor", ((65535, 1513, 5647),))))
print(reg.resolve_stmt(Op("bit_and", ((65535, 1513, 5647),))))

print(reg.resolve_stmt(Op("mul", (("*", 3, 4),))))

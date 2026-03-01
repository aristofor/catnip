# ND VM Architecture

Ce document explique comment les opérations non-déterministes (\~~, ~>, ~[]) passent dans la VM et le bytecode.

## Vue d'ensemble

Les opérations ND sont compilées en opcodes VM dédiés qui délèguent l'exécution au Registry Rust. L'architecture
garantit :

- **Cohérence sémantique** : même comportement en AST et en VM
- **Performance** : dispatch O(1) via opcodes dédiés
- **Réutilisation** : délégation au NDScheduler existant (modes sequential/thread/process)

## Opcodes VM

### NdEmptyTopos (66)

Singleton vide `~[]` - élément identité des opérations ND.

**Stack effect** : `() -> NDTopos`

**Implémentation** :

```rust
OpCode::NdEmptyTopos => {
    let nd_topos = NDTopos.instance()  // Import depuis catnip.nd
    frame.push(nd_topos)
}
```

**Bytecode** :

```
NdEmptyTopos 0
```

### NdRecursion (67)

Opérateur `~~` - récursion non-déterministe avec 2 formes.

**Stack effect** : variable selon arg

- Form 0 (combinator) : `(seed, lambda) -> result`
- Form 1 (declaration) : `(lambda) -> lambda`

**Arguments** :

- `arg = 0` : Combinator form `~~(seed, lambda)`
- `arg = 1` : Declaration form `~~ lambda`

**Implémentation** :

```rust
OpCode::NdRecursion => {
    if arg == 0 {
        // Combinator: pop lambda, pop seed, execute
        let lambda = frame.pop()
        let seed = frame.pop()
        let result = registry.execute_nd_recursion_py(seed, lambda)
        frame.push(result)
    } else {
        // Declaration: pop lambda, push back (no-op)
        let lambda = frame.pop()
        frame.push(lambda)
    }
}
```

**Bytecode** :

```
# Combinator: ~~(5, (n, r) => n - 1)
LoadConst 0        # Push 5
LoadConst 1        # Push lambda
NdRecursion 0      # Execute combinator

# Declaration: countdown = ~~ (n, r) => n - 1
LoadConst 0        # Push lambda
NdRecursion 1      # Declaration (no-op)
StoreLocal 0       # Store countdown
```

### NdMap (68)

Opérateur `~>` - map non-déterministe avec 2 formes.

**Stack effect** : variable selon arg

- Form 0 (applicative) : `(data, func) -> result`
- Form 1 (lift) : `(func) -> func`

**Arguments** :

- `arg = 0` : Applicative form `~>(data, f)`
- `arg = 1` : Lift form `~> f`

**Implémentation** :

```rust
OpCode::NdMap => {
    if arg == 0 {
        // Applicative: pop func, pop data, execute
        let func = frame.pop()
        let data = frame.pop()
        let result = registry.execute_nd_map_py(data, func)
        frame.push(result)
    } else {
        // Lift: pop func, push back (no-op)
        let func = frame.pop()
        frame.push(func)
    }
}
```

**Bytecode** :

```
# Applicative: ~>([1,2,3], (x) => x * 2)
BuildList 3        # Push [1,2,3]
LoadConst 0        # Push lambda
NdMap 0            # Execute applicative

# Lift: double = ~> (x) => x * 2
LoadConst 0        # Push lambda
NdMap 1            # Lift (no-op)
StoreLocal 0       # Store double
```

## Broadcast ND

Les formes broadcast `data.[~~ lambda]` et `data.[~> f]` utilisent l'opcode `Broadcast` existant avec des flags
spéciaux.

### Flags Broadcast

```rust
const FLAG_FILTER: u32 = 1       // bit 0: mode filter
const FLAG_OPERAND: u32 = 2      // bit 1: has operand
const FLAG_ND_RECURSION: u32 = 4 // bit 2: ND recursion
const FLAG_ND_MAP: u32 = 8       // bit 3: ND map
```

### Compilation Broadcast ND

Le compilateur détecte si l'opérateur d'un Broadcast est une opération ND :

```rust
fn compile_broadcast(node: Broadcast) {
    let operator = node.operator

    if operator is Op(ND_RECURSION) {
        // Extract lambda from Op.args[0]
        compile_node(target)
        compile_node(lambda)
        emit(Broadcast, FLAG_ND_RECURSION)
    }
    else if operator is Op(ND_MAP) {
        compile_node(target)
        compile_node(func)
        emit(Broadcast, FLAG_ND_MAP)
    }
    else {
        // Regular broadcast
        compile_node(target)
        compile_node(operator)
        if has_operand: compile_node(operand)
        emit(Broadcast, flags)
    }
}
```

### Exécution Broadcast ND

Le handler VM détecte les flags ND et itère manuellement :

```rust
OpCode::Broadcast => {
    if flags & FLAG_ND_RECURSION {
        let lambda = frame.pop()
        let target = frame.pop()
        let results = []

        for elem in target {
            result = registry.execute_nd_recursion_py(elem, lambda)
            results.append(result)
        }

        frame.push(results)  // Preserve tuple type if needed
    }
    else if flags & FLAG_ND_MAP {
        // Similar for ND map
    }
    else {
        // Regular broadcast via registry._apply_broadcast
    }
}
```

**Bytecode** :

```
# list(5,3,7).[~~ (n, r) => if n <= 1 { 1 } else { n * r(n-1) }]
BuildList 3        # Push [5,3,7]
LoadConst 0        # Push lambda
Broadcast 4        # FLAG_ND_RECURSION
```

## Délégation au Registry

Les opcodes ND délèguent au Registry Rust qui appelle le NDScheduler :

```rust
// Registry (catnip_rs/src/core/registry/nd.rs)
pub fn execute_nd_recursion_py(seed, lambda) -> result {
    // Get NDScheduler from context (modes: sequential/thread/process)
    let scheduler = context.nd_scheduler

    // Wrap lambda if needed (make callable)
    let callable = wrap_function_for_nd(lambda)

    // Dispatch based on mode
    match scheduler.mode {
        "thread" => scheduler.execute_thread(seed, callable),
        "process" => scheduler.execute_process(seed, callable),
        _ => scheduler.execute_sync(seed, callable),
    }
}
```

Le NDScheduler gère :

- **Memoization** : cache des résultats (si `pragma("nd_memoize", True)`)
- **Concurrence** : ThreadPoolExecutor ou ProcessPoolExecutor
- **Batching** : traitement par lots (si `pragma("nd_batch_size", N)`)

## Pragmas Supportés

Les pragmas ND sont lus par le Context Python et passés au NDScheduler :

```python
pragma("nd_mode", "sequential")  # ou "thread", "process"
pragma("nd_workers", 8)          # nombre de workers
pragma("nd_memoize", True)       # activer memoization
pragma("nd_batch_size", 100)     # taille des batches
```

Configuration via CLI :

```bash
catnip -o nd_mode:thread -o nd_workers:8 script.cat
```

## Truthiness et Conditionals

La VM utilise `is_truthy_py()` pour respecter la sémantique Python des PyObjects :

```rust
// value.rs
impl Value {
    pub fn is_truthy_py(self, py: Python) -> bool {
        if self.is_pyobj() {
            // Delegate to Python's __bool__()
            let obj = self.to_pyobject(py)
            obj.is_truthy().unwrap_or(true)
        }
        else {
            // Fast path for primitives (int, float, bool)
            self.is_truthy()
        }
    }
}
```

Cela permet à `NDTopos.instance()` d'être falsy :

```python
if ~[] { 1 } else { 2 }  # Returns 2 (NDTopos is falsy)
```

## Performance

### Dispatch O(1)

Les opcodes ND utilisent le dispatch direct du VM (jump table) :

```rust
match opcode {
    OpCode::NdRecursion => { /* ... */ }
    OpCode::NdMap => { /* ... */ }
    // ...
}
```

Coût : ~5-10 ns par dispatch (vs ~100-200 ns pour lookup Python dict).

### Allocation Minimale

Les formes declaration/lift sont des no-ops qui évitent toute allocation :

```rust
// Declaration: ~~ lambda
NdRecursion 1      # Pop + Push same lambda (0 alloc)

// Lift: ~> f
NdMap 1            # Pop + Push same func (0 alloc)
```

### JIT Potential

Les opérations ND actuelles ne sont pas JIT-compilables car elles :

1. Appellent du code Python (NDScheduler)
1. Peuvent bloquer (modes thread/process)
1. Ont des side-effects (memoization)

Pistes d'optimisation futures :

- **Inline sequential mode** : compiler la récursion directement en loop
- **Specialize pure functions** : détection + compilation native
- **Batch optimization** : fusionner plusieurs appels ND consécutifs

## Tests

Les opérations ND sont couvertes dans `tests/language/`, `tests/optimization/` et `tests/serial/`. Tous passent en modes
AST et VM.

## Références

- `catnip_rs/src/vm/` : opcodes, compilation, handlers VM
- `catnip_rs/src/core/registry/` : logique ND (AST + VM)
- `catnip/nd.py` : NDScheduler, NDTopos, NDRecur
- `docs/lang/ND_RECURSION.md` : spécification langage

> Cette architecture unifie les modes AST et VM via une délégation au Registry. L'utilisateur ne voit aucune différence
> de comportement, seule la performance change.
>
> Le VM évite l'overhead de l'interprétation AST (~10-20x plus rapide) tout en conservant la flexibilité du NDScheduler
> pour les modes parallèles.

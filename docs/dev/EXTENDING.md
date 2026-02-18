# Extension de Catnip

Guide pour ajouter des fonctionnalités à Catnip, sans se perdre dans les couches.

## Ajouter une nouvelle opération

Pour ajouter une nouvelle opération au langage :

### 1. Définir l'opcode

Ajouter l'opcode dans `catnip_rs/src/ir/opcode.rs` (Rust est la source de vérité) :

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum OpCode {
    // ... existing opcodes
    MyOp = 99,
}
```

Puis regénérer le fichier Python :

```bash
python catnip_rs/gen_opcodes.py
```

### 2. Ajouter la règle de grammaire

Modifier `catnip_grammar/grammar.js` pour définir la syntaxe :

```javascript
// Exemple : opérateur binaire @@
my_op: $ => prec.left(PREC.my_op, seq(
    field('left', $._expression),
    '@@',
    field('right', $._expression)
)),

// Ajouter dans _expression
_expression: $ => choice(
    // ... existing choices
    $.my_op,
),
```

Regénérer le parser :

```bash
make grammar-deps
```

### 3. Ajouter le transformer

Créer ou modifier un fichier dans `catnip_rs/src/parser/transforms/` :

```rust
pub fn transform_my_op(
    py: Python,
    node: Node,
    source: &str,
    transformer: &TreeSitterParser,
) -> PyResult<PyObject> {
    let left_node = node.child_by_field_name("left").unwrap();
    let right_node = node.child_by_field_name("right").unwrap();

    let left = transformer.transform_node(py, left_node, source)?;
    let right = transformer.transform_node(py, right_node, source)?;

    let opcode = OpCode::MyOp as i32;
    let args = PyTuple::new(py, &[left, right])?;

    create_ir(py, opcode, args.into_any(), py.None())
}
```

Enregistrer dans `catnip_rs/src/parser/core.rs` :

```rust
"my_op" => transform_my_op(py, node, source, self),
```

### 4. Ajouter l'implémentation dans le Registry

Ajouter le handler dans `catnip_rs/src/core/registry/` (nouveau module ou existant) :

```rust
// Dans arithmetic.rs ou nouveau fichier
impl Registry {
    pub fn op_my_op(&self, py: Python, args: &Bound<PyTuple>) -> PyResult<PyObject> {
        let left = self.exec_stmt(py, args.get_item(0)?)?;
        let right = self.exec_stmt(py, args.get_item(1)?)?;

        // Implémentation de l'opération
        let result = my_implementation(left, right)?;

        Ok(result.into_py(py))
    }
}
```

Ajouter le dispatch dans `execution.rs` :

```rust
fn try_rust_dispatch(&self, py: Python, opcode: i32, args: &Bound<PyTuple>) -> PyResult<Option<PyObject>> {
    match opcode {
        // ... existing cases
        x if x == OpCode::MyOp as i32 => Some(self.op_my_op(py, args)),
        _ => None,
    }
}
```

### 5. Compiler et tester

```bash
uv pip install -e .
make test
```

## Ajouter un opcode VM

Pour ajouter un opcode au niveau bytecode de la VM :

### 1. Définir l'opcode VM

Dans `catnip_rs/src/vm/opcode.rs` :

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VMOpCode {
    // ... existing opcodes
    MyVMOp = 71,
}
```

Regénérer Python :

```bash
python catnip_rs/gen_opcodes.py
```

### 2. Implémenter le dispatch

Dans `catnip_rs/src/vm/core.rs` :

```rust
match opcode {
    // ... existing cases
    VMOpCode::MyVMOp => {
        let arg = frame.pop()?;
        let result = my_vm_operation(arg);
        frame.push(result);
    }
}
```

### 3. Ajouter au compiler

Dans `catnip_rs/src/vm/compiler.rs` :

```rust
fn compile_node(&mut self, py: Python, node: &Bound<PyAny>) -> PyResult<()> {
    match opcode {
        // ... existing cases
        x if x == OpCode::MyOp as i32 => {
            self.compile_node(py, &args.get_item(0)?)?;
            self.compile_node(py, &args.get_item(1)?)?;
            self.emit(VMOpCode::MyVMOp, 0);
        }
    }
    Ok(())
}
```

## Étendre le contexte

Ajouter des fonctions ou variables globales disponibles dans Catnip :

```python
from catnip import Catnip
from catnip.context import Context

# Créer un contexte personnalisé
ctx = Context()

# Ajouter une fonction Python
def my_func(x, y):
    return x + y

ctx.globals._set('my_func', my_func)

# Ajouter une constante
ctx.globals._set('PI', 3.14159)

# Utiliser avec Catnip
cat = Catnip(context=ctx)
cat.parse('my_func(1, 2) + PI')
result = cat.execute()  # 6.14159
```

## Décorateurs

### @pure

Marque une fonction comme pure (sans effets de bord) pour permettre des optimisations :

```python
from catnip import pure

@pure
def square(x):
    return x ** 2

ctx.globals._set('square', square)
```

Les fonctions pures peuvent être optimisées par le broadcast et potentiellement mémoïsées.

### @pass_context

Passe le contexte d'exécution comme premier argument :

```python
from catnip import pass_context

@pass_context
def inspect_scope(ctx):
    return list(ctx.current_scope._symbols.keys())

ctx.globals._set('inspect_scope', inspect_scope)
```

## Créer des passes d'optimisation

Créer une nouvelle passe d'optimisation en Rust :

### 1. Créer le module

Dans `catnip_rs/src/semantic/my_pass.rs` :

```rust
use pyo3::prelude::*;
use super::OptimizationPass;

pub struct MyOptimizationPass;

impl OptimizationPass for MyOptimizationPass {
    fn name(&self) -> &'static str {
        "my_pass"
    }

    fn visit_ir(&self, py: Python, node: &Bound<PyAny>) -> PyResult<PyObject> {
        // Visiter d'abord les enfants
        let node = self.visit_children(py, node)?;

        // Appliquer l'optimisation
        let opcode = node.getattr("ident")?.extract::<i32>()?;

        if opcode == OpCode::MyTargetOp as i32 {
            // Transformer le nœud
            return Ok(optimized_node.into_py(py));
        }

        Ok(node.into_py(py))
    }
}
```

### 2. Enregistrer la passe

Dans `catnip_rs/src/semantic/mod.rs` :

```rust
pub fn create_default_passes() -> Vec<Box<dyn OptimizationPass>> {
    vec![
        // ... existing passes
        Box::new(MyOptimizationPass),
    ]
}
```

### Utilisation Python

```python
from catnip.semantic import Optimizer, ConstantFoldingPass

# Optimiseur personnalisé avec passes spécifiques
optimizer = Optimizer(passes=[
    ConstantFoldingPass(),
    # autres passes...
])

semantic = Semantic(registry, context)
semantic.optimizer = optimizer
```

## Ajouter une commande CLI

Les commandes CLI utilisent un système de plugins via entry points.

### 1. Créer la commande

```python
# my_plugin/commands.py
import click

@click.command()
@click.argument('file')
def mycommand(file):
    """Ma commande personnalisée."""
    click.echo(f"Processing {file}")
```

### 2. Enregistrer via entry points

Dans `pyproject.toml` du plugin :

```toml
[project.entry-points."catnip.commands"]
mycommand = "my_plugin.commands:mycommand"
```

### 3. Installer et utiliser

```bash
pip install my-plugin
catnip mycommand file.cat
```

## Workflow de développement

```bash
# 1. Modifier le code Rust
vim catnip_rs/src/...

# 2. Tests Rust rapides
make rust-test-fast

# 3. Recompiler
uv pip install -e .

# 4. Tests Python complets
make test

# 5. Après modification de grammar.js
make grammar-deps
```

## Structure des fichiers importants

```
catnip_rs/src/
├── ir/opcode.rs           # OpCodes IR (source de vérité)
├── vm/opcode.rs           # OpCodes VM (source de vérité)
├── parser/
│   ├── core.rs            # TreeSitterParser principal
│   └── transforms/        # Transformateurs par catégorie
├── semantic/
│   ├── analyzer.rs        # Semantic analyzer
│   └── *.rs               # Passes d'optimisation
└── core/registry/
    ├── mod.rs             # Registry struct
    ├── execution.rs       # Dispatch principal
    └── *.rs               # Implémentations par catégorie
```

> Étendre Catnip revient à ajouter une pièce à un puzzle multi-couches. Il faut qu'elle s'emboîte partout :
> grammaire, transformation, sémantique, exécution. Si une couche refuse la pièce, tout casse.

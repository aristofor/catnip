// FILE: catnip_rs/src/vm/frame.rs
//! Frame and CodeObject for the Catnip Rust VM.
//!
//! Mirrors catnip/vm/frame.pyx structure for compatibility.

use super::opcode::{Instruction, ParamCheck, VMOpCode};
use super::pattern::VMPattern;
use super::structs::StructRegistry;
use super::value::Value;
use crate::constants::*;
use indexmap::IndexMap;
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub const NO_VARARG_IDX: i32 = -1;

/// Compiled bytecode for a function or lambda.
pub struct CodeObject {
    /// Bytecode instructions
    pub instructions: Vec<Instruction>,
    /// Constant pool (NaN-boxed values)
    pub constants: Vec<Value>,
    /// Variable names for LOAD_NAME/STORE_NAME
    pub names: Vec<String>,
    /// Number of local variable slots
    pub nlocals: usize,
    /// Names of local variables (for debugging)
    pub varnames: Vec<String>,
    /// Map from variable name to slot index
    pub slotmap: HashMap<String, usize>,
    /// Number of parameters (not including *args)
    pub nargs: usize,
    /// Default parameter values
    pub defaults: Vec<Value>,
    /// Function name
    pub name: String,
    /// Free variables (closure captures)
    pub freevars: Vec<String>,
    /// Index of *args parameter (-1 if none)
    pub vararg_idx: i32,
    /// Function marked pure (no side effects)
    pub is_pure: bool,
    /// Complexity estimate (number of instructions) for inline decision
    pub complexity: usize,
    /// Source position table: line_table[i] = start_byte of the Op that generated instruction i
    pub line_table: Vec<u32>,
    /// Pre-compiled VM-native patterns for match expressions
    pub patterns: Vec<VMPattern>,
    /// Pre-classified type-union member specs, indexed by the `CheckUnion` arg.
    pub union_checks: Vec<Box<[ParamCheck]>>,
    /// Pre-classified composite specs, indexed by the `CheckComposite` arg.
    pub composite_checks: Vec<ParamCheck>,
    /// Pre-classified generic-nominal specs, indexed by the `CheckGeneric` arg.
    pub generic_checks: Vec<ParamCheck>,
    /// Cached bytecode hash for JIT trace cache (computed once on demand)
    pub(crate) bytecode_hash: std::sync::OnceLock<u64>,
    /// Postcard-encoded IR body for ND worker IPC transport.
    /// Populated during compile_lambda/compile_fn_def; None for top-level code.
    pub encoded_ir: Option<Arc<Vec<u8>>>,
}

impl CodeObject {
    /// Create a new empty CodeObject.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            names: Vec::new(),
            nlocals: 0,
            varnames: Vec::new(),
            slotmap: HashMap::new(),
            nargs: 0,
            defaults: Vec::new(),
            name: name.into(),
            freevars: Vec::new(),
            vararg_idx: NO_VARARG_IDX,
            is_pure: false,
            complexity: 0,
            line_table: Vec::new(),
            patterns: Vec::new(),
            union_checks: Vec::new(),
            composite_checks: Vec::new(),
            generic_checks: Vec::new(),
            bytecode_hash: std::sync::OnceLock::new(),
            encoded_ir: None,
        }
    }

    /// Compute or retrieve cached bytecode hash (FNV-1a) for JIT trace cache.
    pub fn bytecode_hash(&self) -> u64 {
        *self.bytecode_hash.get_or_init(|| {
            let mut bytes = Vec::with_capacity(self.instructions.len() * 5 + self.constants.len() * 8);
            for i in &self.instructions {
                bytes.push(i.op as u8);
                bytes.extend_from_slice(&i.arg.to_le_bytes());
            }
            for c in &self.constants {
                bytes.extend_from_slice(&c.to_raw().to_le_bytes());
            }
            for n in &self.names {
                bytes.extend_from_slice(n.as_bytes());
                bytes.push(0);
            }
            crate::jit::hash_bytecode(&bytes)
        })
    }
}

impl std::fmt::Debug for CodeObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "<CodeObject {} nlocals={} nargs={}>",
            self.name, self.nlocals, self.nargs
        )
    }
}

/// The pools own one reference per slot: producers insert freshly-built
/// `Value`s, consumers (`LoadConst`, default binding, the match engine)
/// borrow or incref, never decrement. Releasing here is the counterpart.
/// Struct/vmfunc tags never reach a pool (the compiler only emits literals);
/// `decref` ignores them, same as before this Drop existed.
impl Drop for CodeObject {
    fn drop(&mut self) {
        for v in self.constants.drain(..) {
            v.decref();
        }
        for v in self.defaults.drain(..) {
            v.decref();
        }
        for p in self.patterns.drain(..) {
            p.decref_values();
        }
    }
}

impl CodeObject {
    /// Generate unique function ID for JIT profiling.
    ///
    /// Uses fast hash of bytecode + function name for stable identification.
    /// Two functions with identical bytecode and name will share the same ID,
    /// which is desirable for JIT optimization.
    pub fn func_id(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash bytecode (opcode + arg pairs)
        for instr in &self.instructions {
            (instr.op as u8).hash(&mut hasher);
            instr.arg.hash(&mut hasher);
        }

        // Hash function name for disambiguation
        self.name.hash(&mut hasher);

        // Hash nargs to distinguish overloads
        self.nargs.hash(&mut hasher);

        let hash = hasher.finish();
        format!("fn_{:016x}", hash)
    }
}

/// Python-exposed wrapper for CodeObject.
///
/// Exposes the same interface as the Cython CodeObject so VMFunction
/// can use it transparently.
#[pyclass(name = "CodeObject", module = "catnip._rs")]
pub struct PyCodeObject {
    pub inner: Arc<CodeObject>,
}

struct PyCodeObjectInit {
    instructions: Vec<Instruction>,
    constants: Vec<Value>,
    names: Vec<String>,
    nlocals: usize,
    varnames: Vec<String>,
    slotmap: HashMap<String, usize>,
    nargs: usize,
    defaults: Vec<Value>,
    name: String,
    freevars: Vec<String>,
    vararg_idx: i32,
    complexity: usize,
    patterns: Vec<VMPattern>,
}

#[pymethods]
impl PyCodeObject {
    #[new]
    #[pyo3(signature = (bytecode, constants, names, nlocals, varnames, slotmap, nargs, defaults, name, freevars, vararg_idx, patterns=None))]
    #[allow(clippy::too_many_arguments)]
    fn py_new(
        py: Python<'_>,
        bytecode: &Bound<'_, PyAny>,
        constants: &Bound<'_, PyAny>,
        names: &Bound<'_, PyAny>,
        nlocals: usize,
        varnames: &Bound<'_, PyAny>,
        slotmap: &Bound<'_, PyDict>,
        nargs: usize,
        defaults: &Bound<'_, PyAny>,
        name: String,
        freevars: &Bound<'_, PyAny>,
        vararg_idx: i32,
        patterns: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        // Parse bytecode (tuple of (opcode, arg) pairs)
        let bytecode_seq = bytecode.cast::<PyTuple>()?;
        let mut instructions = Vec::new();
        for item in bytecode_seq.iter() {
            let pair = item.cast::<PyTuple>()?;
            let op = pair.get_item(0)?.extract::<u8>()?;
            let arg = pair.get_item(1)?.extract::<u32>()?;
            let opcode = VMOpCode::from_u8(op)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err(format!("Invalid opcode: {}", op)))?;
            instructions.push(Instruction { op: opcode, arg });
        }

        // Parse constants
        let constants_seq = constants.cast::<PyTuple>()?;
        let mut constants_vec = Vec::new();
        for item in constants_seq.iter() {
            constants_vec.push(Value::from_pyobject(py, &item)?);
        }

        // Parse names
        let names_seq = names.cast::<PyTuple>()?;
        let names_vec: Vec<String> = names_seq
            .iter()
            .map(|s| s.extract::<String>())
            .collect::<Result<Vec<_>, _>>()?;

        // Parse varnames
        let varnames_seq = varnames.cast::<PyTuple>()?;
        let varnames_vec: Vec<String> = varnames_seq
            .iter()
            .map(|s| s.extract::<String>())
            .collect::<Result<Vec<_>, _>>()?;

        // Parse slotmap
        let mut slotmap_map = HashMap::new();
        for (key, value) in slotmap.iter() {
            let name = key.extract::<String>()?;
            let slot = value.extract::<usize>()?;
            slotmap_map.insert(name, slot);
        }

        // Parse defaults
        let defaults_seq = defaults.cast::<PyTuple>()?;
        let mut defaults_vec = Vec::new();
        for item in defaults_seq.iter() {
            defaults_vec.push(Value::from_pyobject(py, &item)?);
        }

        // Parse freevars
        let freevars_seq = freevars.cast::<PyTuple>()?;
        let freevars_vec: Vec<String> = freevars_seq
            .iter()
            .map(|s| s.extract::<String>())
            .collect::<Result<Vec<_>, _>>()?;

        // Calculate complexity as instruction count
        let complexity = instructions.len();

        Ok(Self::from_init(PyCodeObjectInit {
            instructions,
            constants: constants_vec,
            names: names_vec,
            nlocals,
            varnames: varnames_vec,
            slotmap: slotmap_map,
            nargs,
            defaults: defaults_vec,
            name,
            freevars: freevars_vec,
            vararg_idx,
            complexity,
            patterns: match patterns {
                Some(p) => vmpattern_vec_from_py(py, p)?,
                None => Vec::new(),
            },
        }))
    }

    /// Bytecode as tuple of (opcode, arg) pairs.
    #[getter]
    fn bytecode(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items: Vec<Py<PyAny>> = self
            .inner
            .instructions
            .iter()
            .map(|i| {
                let tuple = PyTuple::new(py, [i.op as u8 as u32, i.arg]).unwrap();
                tuple.into_any().unbind()
            })
            .collect();
        Ok(PyTuple::new(py, items)?.into_any().unbind())
    }

    /// Constant pool as tuple.
    #[getter]
    fn constants(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items: Vec<Py<PyAny>> = self.inner.constants.iter().map(|v| v.to_pyobject(py)).collect();
        Ok(PyTuple::new(py, items)?.into_any().unbind())
    }

    /// Variable names tuple.
    #[getter]
    fn names(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(PyTuple::new(py, &self.inner.names)?.into_any().unbind())
    }

    /// Number of local variable slots.
    #[getter]
    fn nlocals(&self) -> usize {
        self.inner.nlocals
    }

    /// Local variable names tuple.
    #[getter]
    fn varnames(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(PyTuple::new(py, &self.inner.varnames)?.into_any().unbind())
    }

    /// Map from variable name to slot index.
    #[getter]
    fn slotmap(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);
        for (name, &slot) in &self.inner.slotmap {
            dict.set_item(name, slot)?;
        }
        Ok(dict.into_any().unbind())
    }

    /// Number of parameters.
    #[getter]
    fn nargs(&self) -> usize {
        self.inner.nargs
    }

    /// Default parameter values tuple.
    #[getter]
    fn defaults(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items: Vec<Py<PyAny>> = self.inner.defaults.iter().map(|v| v.to_pyobject(py)).collect();
        Ok(PyTuple::new(py, items)?.into_any().unbind())
    }

    /// Function name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Free variables (closure captures).
    #[getter]
    fn freevars(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(PyTuple::new(py, &self.inner.freevars)?.into_any().unbind())
    }

    /// Index of *args parameter (-1 if none).
    #[getter]
    fn vararg_idx(&self) -> i32 {
        self.inner.vararg_idx
    }

    /// Function marked as pure (no side effects).
    #[getter]
    fn is_pure(&self) -> bool {
        self.inner.is_pure
    }

    /// Complexity estimate (instruction count).
    #[getter]
    fn complexity(&self) -> usize {
        self.inner.complexity
    }

    /// Source position table (start_byte per instruction).
    #[getter]
    fn line_table(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let items: Vec<u32> = self.inner.line_table.clone();
        Ok(PyTuple::new(py, items)?.into_any().unbind())
    }

    /// Set function as pure for JIT inlining.
    ///
    /// Uses `Arc::get_mut` (succeeds during compilation when unshared).
    /// No-op if the code is already shared (Arc refcount > 1).
    #[setter]
    fn set_is_pure(&mut self, value: bool) {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.is_pure = value;
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "<RustCodeObject {} nlocals={} nargs={}>",
            self.inner.name, self.inner.nlocals, self.inner.nargs
        )
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let cls = py.get_type::<Self>();
        let args = PyTuple::new(
            py,
            [
                self.bytecode(py)?,
                self.constants(py)?,
                self.names(py)?,
                self.nlocals().into_pyobject(py)?.into_any().unbind(),
                self.varnames(py)?,
                self.slotmap(py)?,
                self.nargs().into_pyobject(py)?.into_any().unbind(),
                self.defaults(py)?,
                self.name().into_pyobject(py)?.into_any().unbind(),
                self.freevars(py)?,
                self.vararg_idx().into_pyobject(py)?.into_any().unbind(),
                vmpattern_vec_to_py(py, &self.inner.patterns)?,
            ],
        )?;
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?.into_any().unbind())
    }

    /// Print bytecode disassembly to stdout.
    fn disassemble(&self, py: Python<'_>) -> PyResult<()> {
        println!("=== {} ===", self.inner.name);
        println!("  nlocals: {}", self.inner.nlocals);
        println!("  nargs: {}", self.inner.nargs);
        if !self.inner.varnames.is_empty() {
            println!("  varnames: {:?}", self.inner.varnames);
        }
        println!("  bytecode:");
        for (i, instr) in self.inner.instructions.iter().enumerate() {
            let op_name = format!("{:?}", instr.op);
            if instr.arg != 0 || instr.op.has_arg() {
                // Show extra info for certain opcodes
                let extra = match instr.op {
                    super::opcode::VMOpCode::LoadConst => {
                        if let Some(val) = self.inner.constants.get(instr.arg as usize) {
                            format!(" ({})", val.to_pyobject(py))
                        } else {
                            String::new()
                        }
                    }
                    super::opcode::VMOpCode::LoadLocal | super::opcode::VMOpCode::StoreLocal => {
                        if let Some(name) = self.inner.varnames.get(instr.arg as usize) {
                            format!(" ({})", name)
                        } else {
                            String::new()
                        }
                    }
                    super::opcode::VMOpCode::LoadScope
                    | super::opcode::VMOpCode::StoreScope
                    | super::opcode::VMOpCode::LoadGlobal
                    | super::opcode::VMOpCode::GetAttr
                    | super::opcode::VMOpCode::SetAttr => {
                        if let Some(name) = self.inner.names.get(instr.arg as usize) {
                            format!(" ({})", name)
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                };
                println!("    {:4}: {} {}{}", i, op_name, instr.arg, extra);
            } else {
                println!("    {:4}: {}", i, op_name);
            }
        }
        if !self.inner.constants.is_empty() {
            println!("  constants:");
            for (i, c) in self.inner.constants.iter().enumerate() {
                println!("    {:4}: {}", i, c.to_pyobject(py));
            }
        }
        Ok(())
    }
}

impl PyCodeObject {
    /// Create a new PyCodeObject from a CodeObject.
    pub fn new(inner: CodeObject) -> Self {
        Self { inner: Arc::new(inner) }
    }

    fn from_init(init: PyCodeObjectInit) -> Self {
        Self::new(CodeObject {
            instructions: init.instructions,
            constants: init.constants,
            names: init.names,
            nlocals: init.nlocals,
            varnames: init.varnames,
            slotmap: init.slotmap,
            nargs: init.nargs,
            defaults: init.defaults,
            name: init.name,
            freevars: init.freevars,
            vararg_idx: init.vararg_idx,
            is_pure: false,
            complexity: init.complexity,
            line_table: Vec::new(),
            patterns: init.patterns,
            union_checks: Vec::new(),
            composite_checks: Vec::new(),
            generic_checks: Vec::new(),
            bytecode_hash: std::sync::OnceLock::new(),
            encoded_ir: None,
        })
    }

    /// Get function name.
    pub fn get_name(&self) -> &str {
        &self.inner.name
    }
}

// ---------------------------------------------------------------------------
// NativeClosureScope - pure-Rust closure chain for captured variables
// ---------------------------------------------------------------------------

/// Shared Rust globals for standalone mode (no Python Context).
pub type Globals = Rc<RefCell<IndexMap<String, Value>>>;

/// Closure parent in the scope chain.
pub enum ClosureParent {
    /// No parent (top-level function without context)
    None,
    /// Parent is another native closure scope (nested closures)
    Native(NativeClosureScope),
    /// Terminal: module-level globals (only crossing left)
    PyGlobals(Py<PyDict>),
    /// Terminal: Rust-owned globals (standalone, no Python Context)
    Globals(Globals),
}

struct ClosureScopeInner {
    captured: RefCell<IndexMap<String, Value>>,
    parent: ClosureParent,
    /// Names bound by letrec (self-reference or sibling injected by
    /// MakeFunction/PatchClosure). Skipped at serialization: they form
    /// reference cycles that pickle cannot memoize (fresh wrappers).
    letrec_names: RefCell<std::collections::HashSet<String>>,
}

/// The captured map OWNS one ref per entry (MakeFunction clones at capture,
/// set/set_with_py release the overwritten one): release them when the last
/// scope handle dies. `Value::decref` covers pyobj/bigint/complex without a
/// registry; a captured struct instance has no registry access here and its
/// registry ref is deliberately left (narrow, traced in
/// wip/GLOBALS_OWNERSHIP.md -- draining it at Drop through the thread-local
/// registry pointer would be unsound at teardown).
impl Drop for ClosureScopeInner {
    fn drop(&mut self) {
        for (_, v) in self.captured.borrow_mut().drain(..) {
            if v.is_pyobj() || v.is_bigint() || v.is_complex() {
                v.decref();
            }
        }
    }
}

/// Pure-Rust closure scope eliminating Python boundary crossings for
/// captured variable access. Uses `Rc` because sharing is single-threaded
/// only, and `RefCell` for interior mutability.
#[derive(Clone)]
pub struct NativeClosureScope {
    inner: Rc<ClosureScopeInner>,
}

impl NativeClosureScope {
    /// Report this scope's Python references to the cyclic GC: the captured
    /// pyobj handles and a PyGlobals parent terminal. Shared Rc scopes are
    /// reported once per holder (the GC tolerates over-reporting; the
    /// under-reporting was the invisible-cycle leak).
    pub(crate) fn gc_traverse(&self, visit: &pyo3::gc::PyVisit<'_>) -> Result<(), pyo3::PyTraverseError> {
        if let Ok(captured) = self.inner.captured.try_borrow() {
            super::value::visit_obj_handles(captured.values().copied(), visit)?;
        }
        if let ClosureParent::PyGlobals(ref g) = self.inner.parent {
            visit.call(g)?;
        }
        Ok(())
    }

    /// Release the captured entries (same coverage as the Drop: pyobj/bigint/
    /// complex, no registry here). Idempotent -- the Drop drains whatever is
    /// left of an already-drained map.
    pub(crate) fn gc_clear(&self) {
        if let Ok(mut captured) = self.inner.captured.try_borrow_mut() {
            for (_, v) in captured.drain(..) {
                if v.is_pyobj() || v.is_bigint() || v.is_complex() {
                    v.decref();
                }
            }
        }
    }

    /// Release the struct-instance captures against `registry`. The `Drop`
    /// covers pyobj/bigint/complex but deliberately leaves struct captures (no
    /// registry access there). A caller that owns the registry (the ND process
    /// worker, which thaws a fresh scope per task) uses this to reclaim them.
    /// Collects out of the borrow first (a cascade release can run `__del__`
    /// that re-enters this scope) and NILs each slot so a second call and the
    /// `Drop` are no-ops.
    pub(crate) fn release_captured_structs(&self, registry: &StructRegistry) {
        let mut structs: Vec<Value> = Vec::new();
        {
            let mut captured = self.inner.captured.borrow_mut();
            for (_, v) in captured.iter_mut() {
                if v.is_struct_instance() {
                    structs.push(*v);
                    *v = Value::NIL;
                }
            }
        }
        for v in structs {
            super::core::decref_discard(registry, v);
        }
    }

    pub(crate) fn new(captured: IndexMap<String, Value>, parent: ClosureParent) -> Self {
        Self {
            inner: Rc::new(ClosureScopeInner {
                captured: RefCell::new(captured),
                parent,
                letrec_names: RefCell::new(std::collections::HashSet::new()),
            }),
        }
    }

    /// Bind a name directly in this scope's captured set, regardless of the
    /// parent chain (letrec binding by MakeFunction/PatchClosure). The name
    /// is marked as a letrec entry and skipped at serialization.
    pub fn insert_captured(&self, name: &str, value: Value) {
        self.inner.captured.borrow_mut().insert(name.to_string(), value);
        self.inner.letrec_names.borrow_mut().insert(name.to_string());
    }

    /// Whether a captured name was bound by letrec (not serializable).
    pub fn is_letrec_entry(&self, name: &str) -> bool {
        self.inner.letrec_names.borrow().contains(name)
    }

    /// Identity comparison (same underlying scope).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Return all captured variable entries (name, value) in this scope only.
    pub fn captured_entries(&self) -> Vec<(String, Value)> {
        self.inner
            .captured
            .borrow()
            .iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect()
    }

    /// Dump captured variables into a Python dict (for locals() intrinsic).
    pub fn dump_into_dict(&self, py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        let captured = self.inner.captured.borrow();
        for (k, &v) in captured.iter() {
            if !v.is_nil() {
                dict.set_item(k, v.to_pyobject(py))?;
            }
        }
        Ok(())
    }

    /// Pure Rust resolve. Returns `None` when the name is only in PyGlobals.
    pub fn resolve(&self, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                return Some(val);
            }
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.resolve(name),
            ClosureParent::Globals(globals) => globals.borrow().get(name).copied(),
            _ => None,
        }
    }

    /// Resolve from the captured vars of this scope and every enclosing closure,
    /// stopping before the globals terminal. This is the lexical (enclosing)
    /// portion of LEGB: an enclosing closure's binding must shadow a global
    /// homonym, so it has to be consulted before VM/host globals. A doubly
    /// nested closure that does not itself capture a name reaches the enclosing
    /// binding through its parent chain here (no GIL, captured maps only).
    pub fn resolve_captured_chain(&self, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                // Voie A: return a fully owned value (pyobj handle, bigint Arc,
                // struct registry slot) so every resolver path matches the
                // from_pyobject paths; the captured map keeps its own ref and
                // the caller must not add another.
                val.clone_refcount();
                return Some(val);
            }
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.resolve_captured_chain(name),
            _ => None,
        }
    }

    /// Resolve with PyGlobals fallback (needs GIL).
    ///
    /// Ownership: every return path hands back a fully owned value (pyobj
    /// handle, bigint Arc, struct registry slot). Callers push or consume it
    /// without adding a reference of their own.
    pub fn resolve_with_py(&self, py: Python<'_>, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                val.clone_refcount();
                return Some(val);
            }
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.resolve_with_py(py, name),
            // PyGlobals re-boxes via from_pyobject -> already owned.
            ClosureParent::PyGlobals(globals) => globals
                .bind(py)
                .get_item(name)
                .ok()
                .flatten()
                .and_then(|v| Value::from_pyobject(py, &v).ok()),
            ClosureParent::Globals(globals) => globals.borrow().get(name).copied().inspect(|v| {
                v.clone_refcount();
            }),
            ClosureParent::None => None,
        }
    }

    /// Existence check with PyGlobals fallback (needs GIL). Same walk as
    /// [`resolve_with_py`] but builds no value: no refcount side effect, so
    /// callers that only need to know whether a name is bound (StoreScope)
    /// don't have to release an owned result they never use.
    pub fn contains_with_py(&self, py: Python<'_>, name: &str) -> bool {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                return true;
            }
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.contains_with_py(py, name),
            ClosureParent::PyGlobals(globals) => matches!(globals.bind(py).get_item(name), Ok(Some(_))),
            // No nil filter: mirrors resolve_with_py, which returns Some(nil) here.
            ClosureParent::Globals(globals) => globals.borrow().get(name).is_some(),
            ClosureParent::None => false,
        }
    }

    /// Set with PyGlobals fallback (needs GIL).
    pub fn set_with_py(&self, py: Python<'_>, name: &str, value: Value, registry: &StructRegistry) -> bool {
        let mut captured = self.inner.captured.borrow_mut();
        if captured.contains_key(name) {
            // Owned-in on success (wip/GLOBALS_OWNERSHIP.md): the map takes
            // the incoming ref, the overwritten entry releases hers.
            if let Some(old) = captured.insert(name.to_string(), value) {
                old.decref();
            }
            return true;
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.set_with_py(py, name, value, registry),
            ClosureParent::PyGlobals(globals) => {
                if let Ok(Some(_)) = globals.bind(py).get_item(name) {
                    let py_value = value.to_pyobject(py);
                    let stored = globals.bind(py).set_item(name, py_value).is_ok();
                    if stored {
                        // The dict holds a CPython ref, not the handle: the
                        // incoming ref is consumed here (owned-in on success).
                        value.decref();
                    }
                    stored
                } else {
                    false
                }
            }
            ClosureParent::Globals(globals) => {
                let mut g = globals.borrow_mut();
                if g.contains_key(name) {
                    if let Some(old) = g.insert(name.to_string(), value) {
                        // Registry-aware: the host globals map owns struct counts
                        // and bigint refs; a plain decref is a struct no-op and
                        // skips decref_bigint (same bug store_global documents).
                        decref_frame_value(old, registry);
                    }
                    true
                } else {
                    false
                }
            }
            ClosureParent::None => false,
        }
    }

    /// Remap symbol Values in this scope's captured vars (for cross-VM enum transplant).
    /// Does NOT recurse into parents -- the parent Globals Rc is remapped separately.
    pub fn remap_symbols(&self, remap: &std::collections::HashMap<u32, u32>) {
        let mut captured = self.inner.captured.borrow_mut();
        for (_, value) in captured.iter_mut() {
            if value.is_symbol() {
                if let Some(child_sym) = value.as_symbol() {
                    if let Some(&parent_sym) = remap.get(&child_sym) {
                        *value = Value::from_symbol(parent_sym);
                    }
                }
            }
        }
    }

    /// Shift VMFunc Values in this scope's captured vars by `func_base` (for
    /// cross-VM function transplant). Captures hold letrec self/sibling
    /// references as raw child func_table indices; after the child table is
    /// appended to the parent at offset `func_base`, those indices move too.
    /// Does NOT recurse into parents -- the parent Globals Rc is remapped separately.
    pub fn remap_vmfuncs(&self, func_base: u32) {
        if func_base == 0 {
            return;
        }
        let mut captured = self.inner.captured.borrow_mut();
        for (_, value) in captured.iter_mut() {
            if value.is_vmfunc() && !value.is_invalid() {
                *value = Value::from_vmfunc(value.as_vmfunc_idx() + func_base);
            }
        }
    }

    /// Build a NativeClosureScope with a native parent.
    pub fn with_native_parent(captured: IndexMap<String, Value>, parent: NativeClosureScope) -> Self {
        Self::new(captured, ClosureParent::Native(parent))
    }

    /// Build a NativeClosureScope with PyGlobals as terminal parent.
    pub fn with_py_globals(captured: IndexMap<String, Value>, globals: Py<PyDict>) -> Self {
        Self::new(captured, ClosureParent::PyGlobals(globals))
    }

    /// Build a NativeClosureScope with no parent.
    pub fn without_parent(captured: IndexMap<String, Value>) -> Self {
        Self::new(captured, ClosureParent::None)
    }

    /// Build a NativeClosureScope with Globals as terminal parent.
    pub fn with_rust_globals(captured: IndexMap<String, Value>, globals: Globals) -> Self {
        Self::new(captured, ClosureParent::Globals(globals))
    }
}

// SAFETY: VM is single-threaded. RefCell is only accessed from the VM thread.
// The Rc is for shared ownership within the same thread (closures sharing captures).
// Send+Sync needed because Frame lives inside #[pyclass] PyRustVM (PyO3 requirement).
unsafe impl Send for NativeClosureScope {}
// SAFETY: same invariant as the Send impl above -- the Rc/RefCell never crosses
// threads (single VM thread); the bound exists only for the PyO3 #[pyclass] requirement.
unsafe impl Sync for NativeClosureScope {}

impl std::fmt::Debug for NativeClosureScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let captured = self.inner.captured.borrow();
        let keys: Vec<&str> = captured.keys().map(|s| s.as_str()).collect();
        write!(f, "<NativeClosureScope captured={:?}>", keys)
    }
}

/// Convert a Python ClosureScope (ClosureScope) to NativeClosureScope.
pub fn py_scope_to_native(py: Python<'_>, scope: &Py<PyAny>) -> PyResult<NativeClosureScope> {
    let scope_bound = scope.bind(py);
    if let Ok(closure) = scope_bound.cast::<ClosureScope>() {
        let cs = closure.borrow();
        let captured_dict = cs.captured.bind(py);
        let mut captured = IndexMap::new();
        for (key, value) in captured_dict.iter() {
            let name: String = key.extract()?;
            let val = Value::from_pyobject(py, &value)?;
            captured.insert(name, val);
        }
        let parent = match &cs.parent {
            Some(p) => {
                let p_bound = p.bind(py);
                if p_bound.cast::<ClosureScope>().is_ok() {
                    ClosureParent::Native(py_scope_to_native(py, p)?)
                } else if let Ok(dict) = p_bound.cast::<PyDict>() {
                    ClosureParent::PyGlobals(dict.clone().unbind())
                } else {
                    ClosureParent::None
                }
            }
            None => ClosureParent::None,
        };
        Ok(NativeClosureScope::new(captured, parent))
    } else {
        Err(pyo3::exceptions::PyTypeError::new_err(
            "Expected ClosureScope for py_scope_to_native",
        ))
    }
}

/// Convert a NativeClosureScope to a Python ClosureScope.
pub fn native_scope_to_py(py: Python<'_>, scope: &NativeClosureScope) -> PyResult<Py<PyAny>> {
    let captured = scope.inner.captured.borrow();
    let dict = PyDict::new(py);
    for (name, &val) in captured.iter() {
        // Skip letrec bindings (self-reference or mutual group sibling):
        // converting them would cycle (function -> closure -> function ->
        // ...) through pickle, which cannot memoize the fresh wrappers.
        // __setstate__ restores the self-reference after unpickle.
        if scope.is_letrec_entry(name) {
            continue;
        }
        dict.set_item(name, val.to_pyobject(py))?;
    }
    drop(captured);

    let parent: Option<Py<PyAny>> = match &scope.inner.parent {
        ClosureParent::Native(p) => Some(native_scope_to_py(py, p)?),
        // Pass the dict directly (not wrapped in ClosureScope) so that
        // py_scope_to_native detects it as PyGlobals and keeps the live reference.
        ClosureParent::PyGlobals(g) => Some(g.clone_ref(py).into_any()),
        ClosureParent::Globals(globals) => {
            // Convert to PyDict for serialization
            let d = PyDict::new(py);
            for (k, &v) in globals.borrow().iter() {
                d.set_item(k, v.to_pyobject(py))?;
            }
            Some(d.unbind().into_any())
        }
        ClosureParent::None => None,
    };

    let closure = ClosureScope::create(dict.unbind(), parent);
    Ok(Py::new(py, closure)?.into_any())
}

/// A single execution frame on the VM stack.
pub struct Frame {
    /// Operand stack
    pub stack: Vec<Value>,
    /// Local variable slots
    pub locals: Vec<Value>,
    /// Instruction pointer
    pub ip: usize,
    /// Code object being executed (Arc-shared, zero-clone on function call)
    pub code: Option<Arc<CodeObject>>,
    /// Block stack for scope isolation: (slot_start, saved_values)
    pub block_stack: Vec<(usize, Vec<Value>)>,
    /// Python scope for name resolution (fallback)
    pub py_scope: Option<Py<PyAny>>,
    /// Native closure scope for captured variables (pure Rust, no Python boundary)
    pub closure_scope: Option<NativeClosureScope>,
    /// Pending match bindings from MatchPatternVM (slot, value) pairs
    pub match_bindings: Option<Vec<(usize, Value)>>,
    /// If true, return value is discarded (used for init post-constructor)
    pub discard_return: bool,
    /// Super proxy for parent method access in extends
    pub super_proxy: Option<Py<PyAny>>,
    /// Exception handler stack (try/except/finally)
    pub handler_stack: Vec<catnip_core::exception::Handler>,
    /// Active exception stack for CheckExcMatch/LoadException.
    /// Vec (not Option) to support save/restore across nested except handlers.
    pub active_exception_stack: Vec<catnip_core::exception::ExceptionInfo>,
    /// Pending unwind state (saved signal during finally execution)
    pub pending_unwind: Option<catnip_core::exception::PendingUnwind>,
}

impl Frame {
    /// Create a new empty frame.
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(crate::constants::VM_FRAME_STACK_CAPACITY),
            locals: Vec::new(),
            ip: 0,
            code: None,
            block_stack: Vec::new(),
            py_scope: None,
            closure_scope: None,
            match_bindings: None,
            discard_return: false,
            super_proxy: None,
            handler_stack: Vec::new(),
            active_exception_stack: Vec::new(),
            pending_unwind: None,
        }
    }

    /// Create a frame for executing a CodeObject.
    pub fn with_code(code: Arc<CodeObject>) -> Self {
        let nlocals = code.nlocals;
        let mut locals = Vec::with_capacity(nlocals);
        locals.resize(
            nlocals,
            if cfg!(debug_assertions) {
                Value::INVALID
            } else {
                Value::NIL
            },
        );
        Self {
            stack: Vec::with_capacity(crate::constants::VM_FRAME_STACK_CAPACITY),
            locals,
            ip: 0,
            code: Some(code),
            block_stack: Vec::new(),
            py_scope: None,
            closure_scope: None,
            match_bindings: None,
            discard_return: false,
            super_proxy: None,
            handler_stack: Vec::new(),
            active_exception_stack: Vec::new(),
            pending_unwind: None,
        }
    }

    /// Reset frame for reuse. The sole caller (`FramePool::free`) has already
    /// released stack/locals/block_stack refs; this only clears containers.
    pub fn reset(&mut self) {
        self.stack.clear();
        self.locals.clear();
        self.ip = 0;
        self.code = None;
        self.block_stack.clear();
        self.py_scope = None;
        self.closure_scope = None;
        self.match_bindings = None;
        self.discard_return = false;
        self.super_proxy = None;
        self.handler_stack.clear();
        self.active_exception_stack.clear();
        self.pending_unwind = None;
    }

    // --- Stack operations ---

    #[inline]
    pub fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    #[inline]
    pub fn pop(&mut self) -> Value {
        if cfg!(debug_assertions) {
            self.stack.pop().expect("VM stack underflow")
        } else {
            self.stack.pop().unwrap_or(Value::NIL)
        }
    }

    #[inline]
    pub fn peek(&self) -> Value {
        if cfg!(debug_assertions) {
            *self.stack.last().expect("VM stack underflow (peek on empty)")
        } else {
            *self.stack.last().unwrap_or(&Value::NIL)
        }
    }

    // --- Local variable operations ---

    #[inline]
    pub fn set_local(&mut self, slot: usize, value: Value) {
        if slot < self.locals.len() {
            self.locals[slot] = value;
        } else {
            debug_assert!(
                false,
                "set_local: slot {slot} out of bounds (len={})",
                self.locals.len()
            );
        }
    }

    #[inline]
    pub fn get_local(&self, slot: usize) -> Value {
        if slot < self.locals.len() {
            self.locals[slot]
        } else if cfg!(debug_assertions) {
            panic!("get_local: slot {} out of bounds (nlocals={})", slot, self.locals.len())
        } else {
            Value::NIL
        }
    }

    /// Bind function arguments to local slots.
    pub fn bind_args(
        &mut self,
        py: Python<'_>,
        registry: &super::structs::StructRegistry,
        args: &[Value],
        kwargs: Option<&Bound<'_, PyDict>>,
    ) {
        use super::core::decref_discard;
        let code = match &self.code {
            Some(c) => c,
            None => return,
        };

        let nargs_given = args.len();
        let nparams = code.nargs;
        let vararg_idx = code.vararg_idx;

        if vararg_idx >= 0 {
            let vararg_idx = vararg_idx as usize;

            // Bind args before vararg
            self.locals[..nargs_given.min(vararg_idx)].copy_from_slice(&args[..nargs_given.min(vararg_idx)]);

            // Collect excess args into vararg slot
            if nargs_given > vararg_idx {
                let excess: Vec<Py<PyAny>> = args[vararg_idx..].iter().map(|v| v.to_pyobject(py)).collect();
                // Voie A: the caller moves every arg into this frame; the excess
                // ones live on only through the PyList (independent refs).
                for &a in &args[vararg_idx..] {
                    decref_discard(registry, a);
                }
                let list = PyList::new(py, excess).unwrap();
                self.locals[vararg_idx] = Value::from_pyobject(py, &list.into_any()).unwrap_or(Value::NIL);
            } else {
                let empty = PyList::empty(py);
                self.locals[vararg_idx] = Value::from_pyobject(py, &empty.into_any()).unwrap_or(Value::NIL);
            }

            // Bind kwargs
            if let Some(kw) = kwargs {
                for (key, value) in kw.iter() {
                    if let Ok(k) = key.extract::<String>() {
                        if let Some(&slot) = code.slotmap.get(&k) {
                            // A kwarg naming an already-bound slot displaces an
                            // owned value (positional arg); release it.
                            decref_discard(registry, self.locals[slot]);
                            self.locals[slot] = Value::from_pyobject(py, &value).unwrap_or(Value::NIL);
                        }
                    }
                }
            }

            // Fill defaults for params before vararg (skip if already bound)
            let ndefaults = code.defaults.len();
            if ndefaults > 0 {
                let nparams_before_vararg = vararg_idx;
                let default_start = nparams_before_vararg.saturating_sub(ndefaults);
                for i in nargs_given.max(default_start)..nparams_before_vararg {
                    if !self.locals[i].is_nil() && !self.locals[i].is_invalid() {
                        continue;
                    }
                    let default_idx = i - default_start;
                    if default_idx < ndefaults {
                        let val = code.defaults[default_idx];
                        val.clone_refcount();
                        self.locals[i] = val;
                    }
                }
            }
        } else {
            // No variadic parameter
            self.locals[..nargs_given.min(nparams)].copy_from_slice(&args[..nargs_given.min(nparams)]);
            // Voie A: excess args beyond the parameter count are accepted and
            // discarded -- release their moved refs.
            for &a in &args[nargs_given.min(nparams)..] {
                decref_discard(registry, a);
            }

            // Bind kwargs
            if let Some(kw) = kwargs {
                for (key, value) in kw.iter() {
                    if let Ok(k) = key.extract::<String>() {
                        if let Some(&slot) = code.slotmap.get(&k) {
                            // Displaced owned value (positional arg) on overwrite.
                            decref_discard(registry, self.locals[slot]);
                            self.locals[slot] = Value::from_pyobject(py, &value).unwrap_or(Value::NIL);
                        }
                    }
                }
            }

            // Fill defaults for unbound params (skip if already set by kwargs)
            let ndefaults = code.defaults.len();
            if ndefaults > 0 {
                let default_start = nparams.saturating_sub(ndefaults);
                for i in nargs_given.max(default_start)..nparams {
                    // Skip if already bound (by kwargs)
                    if !self.locals[i].is_nil() && !self.locals[i].is_invalid() {
                        continue;
                    }
                    let default_idx = i - default_start;
                    if default_idx < ndefaults {
                        let val = code.defaults[default_idx];
                        val.clone_refcount();
                        self.locals[i] = val;
                    }
                }
            }
        }
    }

    // --- Block stack operations ---

    pub fn push_block(&mut self, slot_start: usize) {
        let mut saved: Vec<Value> = self.locals[slot_start..].to_vec();
        for val in &mut saved {
            val.clone_refcount();
        }
        self.block_stack.push((slot_start, saved));
    }

    /// Restore the pre-block slot values, NILing block-local slots.
    ///
    /// Each snapshot entry holds an independent refcount (taken at push_block),
    /// so every overwritten current local and every NILed block-local slot is
    /// decref'd before the snapshot value is transferred.
    pub fn pop_block(&mut self, registry: &StructRegistry) {
        if let Some((slot_start, saved)) = self.block_stack.pop() {
            let saved_len = saved.len();
            for (i, val) in saved.into_iter().enumerate() {
                if slot_start + i < self.locals.len() {
                    let old = self.locals[slot_start + i];
                    decref_frame_value(old, registry);
                    self.locals[slot_start + i] = val;
                }
            }
            for i in (slot_start + saved_len)..self.locals.len() {
                let old = self.locals[i];
                decref_frame_value(old, registry);
                self.locals[i] = Value::NIL;
            }
        }
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.code.as_ref().map(|c| c.name.as_str()).unwrap_or("<no code>");
        write!(f, "<Frame {} ip={} stack_depth={}>", name, self.ip, self.stack.len())
    }
}

/// Frame pool for reducing allocation overhead.
pub struct FramePool {
    frames: Vec<Frame>,
    max_size: usize,
}

impl FramePool {
    pub fn new(max_size: usize) -> Self {
        Self {
            frames: Vec::with_capacity(max_size),
            max_size,
        }
    }

    #[cfg(test)]
    pub fn alloc(&mut self) -> Frame {
        self.frames.pop().unwrap_or_default()
    }

    /// Get a frame from the pool (or allocate a new one) and initialize with code.
    pub fn alloc_with_code(&mut self, code: Arc<CodeObject>) -> Frame {
        if let Some(mut frame) = self.frames.pop() {
            let nlocals = code.nlocals;
            // Reuse existing Vec capacity
            frame.locals.clear();
            let fill = if cfg!(debug_assertions) {
                Value::INVALID
            } else {
                Value::NIL
            };
            frame.locals.resize(nlocals, fill);
            frame.code = Some(code);
            frame.ip = 0;
            frame.handler_stack.clear();
            frame.active_exception_stack.clear();
            frame.pending_unwind = None;
            frame
        } else {
            Frame::with_code(code)
        }
    }

    pub fn free(&mut self, mut frame: Frame, registry: &StructRegistry) {
        decref_frame_values(&frame, registry);
        // block_stack entries hold independent refcounts (taken at push_block).
        // Release them unconditionally here, pooled or not: a frame freed with
        // a non-empty block_stack would otherwise leak its snapshot refs.
        for (_slot_start, saved) in frame.block_stack.drain(..) {
            for val in saved {
                decref_frame_value(val, registry);
            }
        }
        // Same for pending match bindings: they own independent refs (BindMatch
        // clones into slots), released here when the frame dies with an arm's
        // bindings still live.
        if let Some(bindings) = frame.match_bindings.take() {
            for (_slot, val) in bindings {
                decref_frame_value(val, registry);
            }
        }
        if self.frames.len() < self.max_size {
            frame.reset();
            self.frames.push(frame);
        }
    }
}

/// Release one owned heap value (PyObject handle, BigInt, Complex, Struct).
/// Thin alias over `core::decref_discard`: frame teardown and opcode discards
/// share one release implementation so the ownership contract cannot drift.
#[inline]
pub fn decref_frame_value(val: Value, registry: &StructRegistry) {
    super::core::decref_discard(registry, val);
}

/// Decref all heap values (PyObject, BigInt, Complex, Struct) in a frame's
/// stack and locals. block_stack entries hold independent refcounts (taken at
/// push_block) and are handled separately by reset/pop_block/truncate.
pub fn decref_frame_values(frame: &Frame, registry: &StructRegistry) {
    for &val in &frame.stack {
        decref_frame_value(val, registry);
    }
    for &val in &frame.locals {
        decref_frame_value(val, registry);
    }
}

impl Default for FramePool {
    fn default() -> Self {
        Self::new(VM_FRAME_POOL_SIZE)
    }
}

/// VM function wrapper for CodeObject.
///
/// Provides the vm_code attribute that the VM CALL handler looks for.
/// Build a `Globals` map from a Python context's globals dict.
///
/// Used by `VMFunction.__call__` when invoked from external code (e.g.
/// pandas.apply) where no parent VM globals are available.
/// Drain an EPHEMERAL `VMFunction::__call__` host, struct-aware when the
/// parent registry is reachable: `build_globals_from_context` re-interns
/// context proxies with an incref on their PARENT slot (the thread-local
/// still points at the parent when the map is built), so the plain
/// struct-blind drain leaked one registry count per re-entrant call. With no
/// parent registry, `from_pyobject` cannot restore TAG_STRUCT (the proxy
/// falls back to a pyobj), so the plain drain is complete.
fn drain_ephemeral_host(host: &super::host::VMHost, saved_registry: *const super::structs::StructRegistry) {
    if saved_registry.is_null() {
        host.drain_globals(None);
    } else {
        // SAFETY: saved_registry points to the parent VM's StructRegistry,
        // live on the stack for this synchronous, GIL-held nested call. A shared
        // `&` (never `&mut`): mutation goes through the interior RefCell, so this
        // never aliases the thread-local raw pointer to the same registry.
        let parent = unsafe { &*saved_registry };
        host.drain_globals(Some(parent));
    }
}

fn build_globals_from_context(py: Python<'_>, ctx: &Py<PyAny>) -> PyResult<Globals> {
    let globals: Globals = Rc::new(RefCell::new(IndexMap::new()));
    let ctx_globals = ctx.bind(py).getattr("globals")?;
    if let Ok(dict) = ctx_globals.cast::<PyDict>() {
        let mut g = globals.borrow_mut();
        for (key, value) in dict.iter() {
            if let Ok(name) = key.extract::<String>() {
                if let Ok(val) = super::value::Value::from_pyobject(py, &value) {
                    g.insert(name, val);
                }
            }
        }
    }
    Ok(globals)
}

/// Also captures closure scope for nested functions.
#[pyclass(module = "catnip._rs")]
pub struct VMFunction {
    /// Compiled bytecode
    #[pyo3(get)]
    pub vm_code: Py<PyCodeObject>,
    /// Native closure scope (primary, used by VM hot path)
    pub native_closure: Option<NativeClosureScope>,
    /// Python closure scope kept for backward compat (#[new] from Python, pickle)
    py_closure_scope: Option<Py<PyAny>>,
    /// Function name
    #[pyo3(get)]
    pub name: String,
    /// Context reference for direct calls
    context: Option<Py<PyAny>>,
    /// Index in the VM's FunctionTable (for TAG_VMFUNC round-trip)
    pub func_table_idx: Option<u32>,
}

impl VMFunction {
    /// Create from Rust with native closure (MakeFunction hot path).
    pub fn create_native(
        py: Python<'_>,
        code: Py<PyCodeObject>,
        native_closure: Option<NativeClosureScope>,
        context: Option<Py<PyAny>>,
    ) -> Self {
        let name = code.borrow(py).get_name().to_string();
        Self {
            vm_code: code,
            native_closure,
            py_closure_scope: None,
            name,
            context,
            func_table_idx: None,
        }
    }

    /// Create from Rust with Python closure scope (backward compat).
    pub fn create(
        py: Python<'_>,
        code: Py<PyCodeObject>,
        closure_scope: Option<Py<PyAny>>,
        context: Option<Py<PyAny>>,
    ) -> Self {
        let native_closure = closure_scope.as_ref().and_then(|cs| py_scope_to_native(py, cs).ok());
        let name = code.borrow(py).get_name().to_string();
        Self {
            vm_code: code,
            native_closure,
            py_closure_scope: closure_scope,
            name,
            context,
            func_table_idx: None,
        }
    }
}

#[pymethods]
impl VMFunction {
    #[new]
    #[pyo3(signature = (code, closure_scope=None, context=None))]
    fn new(code: Py<PyCodeObject>, closure_scope: Option<Py<PyAny>>, context: Option<Py<PyAny>>) -> PyResult<Self> {
        let name = Python::attach(|py| {
            let n = code.borrow(py).get_name().to_string();
            n
        });
        let native_closure = Python::attach(|py| closure_scope.as_ref().and_then(|cs| py_scope_to_native(py, cs).ok()));
        Ok(Self {
            vm_code: code,
            native_closure,
            py_closure_scope: closure_scope,
            name,
            context,
            func_table_idx: None,
        })
    }

    /// Participate in CPython's cyclic GC. A `VMFunction` stored in
    /// `ctx.globals` holds the context back via its `context` field, closing a
    /// `ctx.globals -> VMFunction -> ctx` cycle the collector cannot see (a Rust
    /// pyclass is opaque to it). Without this, every session that defines a
    /// function leaks its context.
    ///
    /// The captured handles of `native_closure` ARE reported (dedup by
    /// slot): a letrec self-reference injects the function's own pyobj
    /// handle into its captured map (handle -> VMFunction -> Rc -> captured
    /// -> handle), a cycle the GC cannot see without this leg -- it pinned
    /// the whole capture set for the life of the process.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        visit.call(&self.vm_code)?;
        if let Some(ref scope) = self.py_closure_scope {
            visit.call(scope)?;
        }
        if let Some(ref ctx) = self.context {
            visit.call(ctx)?;
        }
        if let Some(ref closure) = self.native_closure {
            closure.gc_traverse(&visit)?;
        }
        Ok(())
    }

    /// Break the `ctx.globals <-> VMFunction` and letrec self-reference
    /// cycles by dropping the references reported by `__traverse__`. Only
    /// called by the GC on an otherwise-unreachable function; the closure
    /// drain is idempotent with the Rc Drop (which drains what is left).
    fn __clear__(&mut self) {
        self.context = None;
        self.py_closure_scope = None;
        if let Some(ref closure) = self.native_closure {
            closure.gc_clear();
        }
    }

    /// Lazy getter: converts native closure to Python ClosureScope on demand.
    #[getter]
    fn closure_scope(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        if let Some(ref py_scope) = self.py_closure_scope {
            return Ok(Some(py_scope.clone_ref(py)));
        }
        if let Some(ref native) = self.native_closure {
            Ok(Some(native_scope_to_py(py, native)?))
        } else {
            Ok(None)
        }
    }

    fn __repr__(&self) -> String {
        format!("<VMFunction {}>", self.name)
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let cls = py.get_type::<Self>();
        let py_scope = self.closure_scope(py)?;
        let args = PyTuple::new(
            py,
            [
                self.vm_code.clone_ref(py).into_any(),
                py_scope.unwrap_or_else(|| py.None()),
                py.None(),
            ],
        )?;
        // Non-None state triggers __setstate__, which restores the let-rec
        // self-binding skipped during serialization (see native_scope_to_py)
        let state = pyo3::types::PyBool::new(py, true).to_owned().into_any();
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any(), state])?
            .into_any()
            .unbind())
    }

    /// Restore the let-rec self-binding after unpickling: serialization
    /// omits it (it would cycle), so named lambdas re-bind themselves here.
    fn __setstate__(slf: &Bound<'_, Self>, _state: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let name = slf.borrow().name.clone();
        if name == "<lambda>" || name == "<module>" || name == "<fn>" {
            return Ok(());
        }
        let val = Value::from_pyobject(py, slf.as_any())?;
        let func = slf.borrow();
        if let Some(ref native) = func.native_closure {
            native.insert_captured(&name, val);
        }
        Ok(())
    }

    #[pyo3(signature = (*args, **kwargs))]
    #[allow(unused_variables)]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        // Always use the standalone VM path. Previously, when called from
        // external code (e.g. pandas.apply), a new VMExecutor was created per
        // call -- with empty registries and a Python import per invocation.
        // This caused segfaults (invalid func_table/struct_registry indices)
        // and massive overhead on repeated callbacks.
        let parent_globals = super::host::take_vm_globals();

        {
            use super::host::VMHost;
            use super::value::{FuncSlot, Value};

            let code = std::sync::Arc::clone(&self.vm_code.borrow(py).inner);

            // Resolve globals: parent VM > Python context > fresh builtins.
            // A host built on a FRESH map (context snapshot or new builtins)
            // owns its handles and dies with this call: drain it on the way
            // out, or every re-entrant callback leaks its whole globals map
            // (no GC hook ever fires for it). The parent-globals case shares
            // the parent's Rc and must NOT be drained here.
            let ephemeral_host;
            let mut host = if let Some(globals) = parent_globals {
                ephemeral_host = false;
                VMHost::with_globals(py, globals)
            } else if let Some(ctx) = &self.context {
                ephemeral_host = true;
                let globals = build_globals_from_context(py, ctx)?;
                VMHost::with_globals(py, globals)
            } else {
                ephemeral_host = true;
                VMHost::new(py)
            }
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("{}", e)))?;

            // Inject context for @pass_context functions
            if let Some(ctx) = &self.context {
                host.set_context(ctx.clone_ref(py));
            }

            // Save parent's thread-local pointers (nested call must not clobber them)
            let saved_registry = super::value::save_struct_registry();
            let saved_func_table = super::value::save_func_table();
            let saved_symbol_table = super::value::save_symbol_table();
            let saved_enum_registry = super::value::save_enum_registry();

            let mut vm = super::core::VM::new();

            // Copy parent's func_table entries so VmFunc indices in closures remain valid
            if !saved_func_table.is_null() {
                // SAFETY: saved_func_table is the thread-local pointer captured above and
                // checked non-null; it points to the parent VM's FunctionTable, which stays
                // live on the stack for this synchronous, GIL-held nested call.
                let parent_table = unsafe { &*saved_func_table };
                for slot in &parent_table.slots {
                    vm.func_table.insert(FuncSlot {
                        code: std::sync::Arc::clone(&slot.code),
                        closure: slot.closure.clone(),
                        code_py: slot.code_py.clone_ref(py),
                        context: slot.context.as_ref().map(|c| c.clone_ref(py)),
                    });
                }
            }

            // Copy parent's struct types and instances so struct indices remain valid
            if !saved_registry.is_null() {
                // SAFETY: saved_registry is the thread-local pointer captured above and
                // checked non-null; it points to the parent VM's StructRegistry, live on
                // the stack for the duration of this synchronous, GIL-held nested call.
                let parent_registry = unsafe { &*saved_registry };
                vm.struct_registry.clone_from_parent(py, parent_registry);
            }

            // Inherit parent's symbol table + enum registry so enum/union variants
            // keep their identity inside the callback. execute_with_host points the
            // thread-local at this VM's own tables; without the copy they would be
            // empty and `resolve_symbol` would fail, demoting variants to raw ids
            // (str gives the index, method dispatch and match break).
            if !saved_symbol_table.is_null() {
                // SAFETY: saved_symbol_table is the thread-local pointer captured above and
                // checked non-null; it points to the parent VM's live SymbolTable, only read
                // (cloned) here under the GIL.
                vm.symbol_table = unsafe { (*saved_symbol_table).clone() };
            }
            if !saved_enum_registry.is_null() {
                // SAFETY: saved_enum_registry is the thread-local pointer captured above and
                // checked non-null; it points to the parent VM's live EnumRegistry, only read
                // (cloned) here under the GIL.
                vm.enum_registry = unsafe { (*saved_enum_registry).clone() };
            }
            super::value::set_struct_registry(&vm.struct_registry as *const _);
            // Set func_table AFTER copying parent entries so new VmFunc indices
            // created during execution go to this table, while inherited indices
            // remain valid.
            super::value::set_func_table(&vm.func_table as *const _);

            let mut arg_values = Vec::with_capacity(args.len());
            for item in args.iter() {
                arg_values.push(Value::from_pyobject(py, &item).map_err(pyo3::exceptions::PyRuntimeError::new_err)?);
            }

            let closure = self.native_closure.clone();
            let mut result = vm
                .execute_with_host(py, code, &arg_values, &host, closure)
                .map_err(|e| {
                    // Restore parent pointers on error
                    if ephemeral_host {
                        drain_ephemeral_host(&host, saved_registry);
                    }
                    super::value::restore_struct_registry(saved_registry);
                    super::value::restore_func_table(saved_func_table);
                    // During ND abort, skip Debug formatting to avoid quadratic
                    // string growth (each level would wrap the previous error).
                    if crate::nd::check_nd_abort() {
                        pyo3::exceptions::PyRecursionError::new_err("maximum ND recursion depth exceeded")
                    } else {
                        pyo3::exceptions::PyRuntimeError::new_err(format!("{}", e))
                    }
                })?;

            // Transplant new struct instances from child VM to parent registry
            // so they survive after the child is dropped.
            let transplanted: Vec<u32> = if !saved_registry.is_null() {
                // SAFETY: saved_registry was checked non-null; it points to the parent VM's
                // StructRegistry, still live on the stack. Shared `&` (never `&mut`): the
                // transplant mutates the parent through its interior RefCell, so it cannot
                // alias the thread-local raw pointer to the same registry, GIL-held throughout.
                let parent_registry = unsafe { &*saved_registry };
                vm.struct_registry.transplant_to_parent(py, parent_registry)
            } else {
                Vec::new()
            };

            // Drain BEFORE restoring the thread-locals: the shared borrow taken
            // on the parent registry stays on a different registry than the
            // active TL (still the child's), keeping the two access paths apart.
            if ephemeral_host {
                drain_ephemeral_host(&host, saved_registry);
            }

            // Restore parent pointers
            super::value::restore_struct_registry(saved_registry);
            super::value::restore_func_table(saved_func_table);

            // Broadcast mutation semantics: a pass-through result struct is the
            // child's snapshot (possibly mutated by the callback), whose slot
            // collides with the parent's original and is never transplanted.
            // Materialize the child's field state as a NEW parent instance --
            // shallow, mirroring the AST-side element copy -- so the returned
            // element is the callback's private copy, not the untouched parent
            // original.
            let mut materialized_idx: Option<u32> = None;
            if !saved_registry.is_null() {
                if let Some(idx) = result.as_struct_instance_idx() {
                    if !transplanted.contains(&idx) {
                        // Snapshot type + field bits under the borrow (Value is Copy,
                        // so the clone is refcount-neutral); take the field increfs
                        // AFTER the borrow drops -- clone_refcount on a struct field
                        // re-enters the registry (incref borrows), which must not nest
                        // inside with_instance.
                        let snapshot = vm
                            .struct_registry
                            .with_instance(idx, |inst| (inst.type_id, inst.fields.clone()));
                        if let Some((type_id, fields)) = snapshot {
                            for f in &fields {
                                f.clone_refcount();
                            }
                            // SAFETY: saved_registry was checked non-null and points to the
                            // parent VM's StructRegistry, live on the stack. Shared `&` (never
                            // `&mut`): create_instance mutates through the interior RefCell, so
                            // it cannot alias the thread-local raw pointer to the same registry.
                            let parent = unsafe { &*saved_registry };
                            let new_idx = parent.create_instance(type_id, fields);
                            // The child slot keeps its own counts (released with the
                            // child); the new instance owns the cloned field refs, and
                            // its create rc is transferred to the proxy below (struct
                            // decref on the old result Value is a deliberate no-op).
                            materialized_idx = Some(new_idx);
                            result = super::value::Value::from_struct_instance(new_idx);
                        }
                    }
                }
            }

            let py_result = result.to_pyobject(py);
            // Transfer the result struct's VM-internal count to its proxy: a
            // transplanted slot carries the child's copied refcount, and a
            // materialized private copy carries its create rc -- in both cases
            // the proxy above took its own ref, so the VM-side count must be
            // released here or it lingers as a phantom no proxy decref ever
            // clears (CatnipOwnershipProof::copy_leaks; the materialized case
            // leaked one instance per re-entrant call, found by the
            // intra-session ledger on a pass-through broadcast).
            if !saved_registry.is_null() {
                if let Some(idx) = result.as_struct_instance_idx() {
                    if transplanted.contains(&idx) || materialized_idx == Some(idx) {
                        // SAFETY: saved_registry was checked non-null and points to the parent
                        // VM's StructRegistry, live on the stack. Shared `&` (never `&mut`): the
                        // decref cascades through the interior RefCell, so it cannot alias the
                        // thread-local raw pointer to the same registry, GIL-held throughout.
                        let parent = unsafe { &*saved_registry };
                        super::structs::decref_slot(parent, idx);
                    }
                }
            }
            // The execute result is owned and to_pyobject above only reads it:
            // release the non-struct heap count (pyobj handle, bigint/complex
            // Arc) or it survives the callback -- one count per invocation
            // (+1 pinned Py per broadcast when the result is a pyobj). The
            // struct case is the registry decref just above; Value::decref is
            // a deliberate no-op on TAG_STRUCT.
            result.decref();
            Ok(py_result)
        }
    }
}

/// Scope wrapper for closure capture.
///
/// Provides _resolve() and _set() methods compatible with Scope chain lookup.
/// Falls back to parent scope if name not found in captured values.
#[pyclass(module = "catnip._rs")]
pub struct ClosureScope {
    /// Captured variable values
    captured: Py<PyDict>,
    /// Parent scope for chain lookup
    parent: Option<Py<PyAny>>,
}

impl ClosureScope {
    /// Create from Rust code.
    pub fn create(captured: Py<PyDict>, parent: Option<Py<PyAny>>) -> Self {
        Self { captured, parent }
    }
}

#[pymethods]
impl ClosureScope {
    #[new]
    #[pyo3(signature = (captured, parent=None))]
    fn new(captured: Py<PyDict>, parent: Option<Py<PyAny>>) -> Self {
        Self { captured, parent }
    }

    /// Resolve a name in the closure scope chain.
    fn _resolve(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let captured = self.captured.bind(py);

        if let Some(value) = captured.get_item(name)? {
            // Check if value is UNBOUND sentinel
            if value.is_none() {
                // Fall through to parent
            } else {
                return Ok(value.unbind());
            }
        }

        // Try parent scope
        if let Some(ref parent) = self.parent {
            let parent_bound = parent.bind(py);
            if parent_bound.hasattr("_resolve")? {
                return parent_bound
                    .call_method1("_resolve", (name,))?
                    .extract()
                    .map_err(Into::into);
            }
        }

        // Raise NameError
        let exc_module = py.import(PY_MOD_EXC)?;
        let name_error = exc_module.getattr("CatnipNameError")?;
        Err(PyErr::from_value(
            name_error.call1((catnip_core::constants::format_name_error(name),))?,
        ))
    }

    /// Set a name in the closure scope.
    fn _set(&self, py: Python<'_>, name: &str, value: Py<PyAny>) -> PyResult<()> {
        let captured = self.captured.bind(py);

        // If name exists in captured, set it there
        if captured.contains(name)? {
            captured.set_item(name, value)?;
            return Ok(());
        }

        // Try parent scope if it has the name
        if let Some(ref parent) = self.parent {
            let parent_bound = parent.bind(py);
            if parent_bound.hasattr("_resolve")? {
                // Check if parent has the name
                let has_name = parent_bound.call_method1("_resolve", (name,)).is_ok();
                if has_name && parent_bound.hasattr("_set")? {
                    parent_bound.call_method1("_set", (name, &value))?;
                    return Ok(());
                }
            }
        }

        // Add to captured
        captured.set_item(name, value)?;
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let captured = self.captured.bind(py);
        let keys: Vec<String> = captured.keys().iter().filter_map(|k| k.extract().ok()).collect();
        format!("<ClosureScope captured={:?}>", keys)
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        // Return (class, (captured, None))
        // Don't pickle parent - it contains builtins that aren't picklable
        let cls = py.get_type::<Self>();
        let args = PyTuple::new(
            py,
            [
                self.captured.clone_ref(py).into_any(),
                py.None(), // parent set to None - will be recreated on unpickle
            ],
        )?;
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?.into_any().unbind())
    }
}

// === VMPattern pickle serialization ===

use super::pattern::VMPatternElement;

/// Convert Vec<VMPattern> to a Python list of dicts for pickling.
fn vmpattern_vec_to_py(py: Python<'_>, patterns: &[VMPattern]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for pat in patterns {
        list.append(vmpattern_to_py(py, pat)?)?;
    }
    Ok(list.into_any().unbind())
}

fn vmpattern_to_py(py: Python<'_>, pat: &VMPattern) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    match pat {
        VMPattern::Wildcard => {
            dict.set_item("t", "w")?;
        }
        VMPattern::Literal(val) => {
            dict.set_item("t", "l")?;
            dict.set_item("v", val.to_pyobject(py))?;
        }
        VMPattern::Var(slot) => {
            dict.set_item("t", "v")?;
            dict.set_item("s", *slot)?;
        }
        VMPattern::Or(subs) => {
            dict.set_item("t", "o")?;
            dict.set_item("p", vmpattern_vec_to_py(py, subs)?)?;
        }
        VMPattern::Tuple(elems) => {
            dict.set_item("t", "tp")?;
            let list = PyList::empty(py);
            for elem in elems {
                list.append(vmpattern_elem_to_py(py, elem)?)?;
            }
            dict.set_item("e", list)?;
        }
        VMPattern::Struct {
            name,
            variant,
            field_slots,
        } => {
            dict.set_item("t", "s")?;
            dict.set_item("n", name.as_str())?;
            match variant {
                Some(v) => dict.set_item("vt", v.as_str())?,
                None => dict.set_item("vt", py.None())?,
            }
            let fields = PyList::empty(py);
            for (fname, slot) in field_slots {
                fields.append(PyTuple::new(
                    py,
                    [fname.into_pyobject(py)?.into_any(), slot.into_pyobject(py)?.into_any()],
                )?)?;
            }
            dict.set_item("f", fields)?;
        }
        VMPattern::Enum {
            enum_name,
            variant_name,
        } => {
            dict.set_item("t", "e")?;
            dict.set_item("en", enum_name.as_str())?;
            dict.set_item("vn", variant_name.as_str())?;
        }
    }
    Ok(dict.into_any().unbind())
}

fn vmpattern_elem_to_py(py: Python<'_>, elem: &VMPatternElement) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    match elem {
        VMPatternElement::Pattern(p) => {
            dict.set_item("k", "p")?;
            dict.set_item("p", vmpattern_to_py(py, p)?)?;
        }
        VMPatternElement::Star(slot) => {
            dict.set_item("k", "s")?;
            dict.set_item("s", *slot)?;
        }
    }
    Ok(dict.into_any().unbind())
}

/// Reconstruct Vec<VMPattern> from Python list of dicts.
fn vmpattern_vec_from_py(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Vec<VMPattern>> {
    let list = obj.cast::<PyList>()?;
    let mut patterns = Vec::with_capacity(list.len());
    for item in list.iter() {
        patterns.push(vmpattern_from_py(py, &item)?);
    }
    Ok(patterns)
}

fn vmpattern_from_py(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<VMPattern> {
    let dict = obj.cast::<PyDict>()?;
    let tag: String = dict.get_item("t")?.unwrap().extract()?;
    match tag.as_str() {
        "w" => Ok(VMPattern::Wildcard),
        "l" => {
            let val = dict.get_item("v")?.unwrap();
            Ok(VMPattern::Literal(Value::from_pyobject(py, &val)?))
        }
        "v" => {
            let slot: usize = dict.get_item("s")?.unwrap().extract()?;
            Ok(VMPattern::Var(slot))
        }
        "o" => {
            let subs = dict.get_item("p")?.unwrap();
            Ok(VMPattern::Or(vmpattern_vec_from_py(py, &subs)?))
        }
        "tp" => {
            let elems_list = dict.get_item("e")?.unwrap();
            let list = elems_list.cast::<PyList>()?;
            let mut elems = Vec::with_capacity(list.len());
            for item in list.iter() {
                elems.push(vmpattern_elem_from_py(py, &item)?);
            }
            Ok(VMPattern::Tuple(elems))
        }
        "s" => {
            let name: String = dict.get_item("n")?.unwrap().extract()?;
            let variant: Option<String> = match dict.get_item("vt")? {
                Some(v) if !v.is_none() => Some(v.extract()?),
                _ => None,
            };
            let fields_list = dict.get_item("f")?.unwrap().cast::<PyList>()?.clone();
            let mut field_slots = Vec::new();
            for item in fields_list.iter() {
                let pair = item.cast::<PyTuple>()?;
                let fname: String = pair.get_item(0)?.extract()?;
                let slot: usize = pair.get_item(1)?.extract()?;
                field_slots.push((fname, slot));
            }
            Ok(VMPattern::Struct {
                name,
                variant,
                field_slots,
            })
        }
        "e" => {
            let enum_name: String = dict.get_item("en")?.unwrap().extract()?;
            let variant_name: String = dict.get_item("vn")?.unwrap().extract()?;
            Ok(VMPattern::Enum {
                enum_name,
                variant_name,
            })
        }
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Unknown pattern tag: {}",
            tag
        ))),
    }
}

fn vmpattern_elem_from_py(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<VMPatternElement> {
    let dict = obj.cast::<PyDict>()?;
    let kind: String = dict.get_item("k")?.unwrap().extract()?;
    match kind.as_str() {
        "p" => {
            let p = dict.get_item("p")?.unwrap();
            Ok(VMPatternElement::Pattern(vmpattern_from_py(py, &p)?))
        }
        "s" => {
            let slot: usize = dict.get_item("s")?.unwrap().extract()?;
            Ok(VMPatternElement::Star(slot))
        }
        _ => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Unknown element kind: {}",
            kind
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_stack() {
        let mut frame = Frame::new();
        frame.push(Value::from_int(1));
        frame.push(Value::from_int(2));
        frame.push(Value::from_int(3));

        assert_eq!(frame.peek().as_int(), Some(3));
        assert_eq!(frame.pop().as_int(), Some(3));
        assert_eq!(frame.pop().as_int(), Some(2));
        assert_eq!(frame.pop().as_int(), Some(1));
    }

    #[test]
    fn test_frame_locals() {
        let mut code = CodeObject::new("test");
        code.nlocals = 3;
        let mut frame = Frame::with_code(Arc::new(code));

        frame.set_local(0, Value::from_int(10));
        frame.set_local(1, Value::from_int(20));

        assert_eq!(frame.get_local(0).as_int(), Some(10));
        assert_eq!(frame.get_local(1).as_int(), Some(20));
    }

    #[test]
    fn test_frame_pool() {
        let mut pool = FramePool::new(2);
        let registry = StructRegistry::new();

        let frame1 = pool.alloc();
        let frame2 = pool.alloc();

        pool.free(frame1, &registry);
        pool.free(frame2, &registry);

        assert_eq!(pool.frames.len(), 2);
    }

    // --- Resolver ownership contract (owned in every return path) ---
    //
    // resolve_captured_chain / resolve_with_py must hand back a fully owned
    // value on the borrow paths (captured map, native Globals), matching the
    // from_pyobject paths that are owned by construction. The LoadScope
    // callers push the result without adding a reference, so a borrowed
    // return here would be an under-count (double release downstream).

    #[test]
    fn resolve_captured_chain_returns_owned_bigint() {
        use rug::Integer;
        use rug::ops::Pow;

        let big = Value::from_bigint(Integer::from(10).pow(40));
        assert_eq!(big.bigint_strong_count(), 1);

        let mut captured = IndexMap::new();
        captured.insert("x".to_string(), big);
        let scope = NativeClosureScope::without_parent(captured);

        let resolved = scope.resolve_captured_chain("x").expect("captured name resolves");
        assert_eq!(resolved.bits(), big.bits());
        assert_eq!(
            big.bigint_strong_count(),
            2,
            "resolver must return an owned bigint (captured map ref + returned ref)"
        );

        resolved.decref_bigint();
        assert_eq!(big.bigint_strong_count(), 1, "release balances the resolver incref");
    }

    #[test]
    fn release_captured_structs_reclaims_struct_slots() {
        use crate::vm::structs::StructField;
        use catnip_core::vm::opcode::ParamCheck;

        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "P".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
                check: ParamCheck::None,
            }],
            IndexMap::new(),
            vec![],
            vec!["P".into()],
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(1)]);
        assert_eq!(reg.live_count(), 1);

        let mut captured = IndexMap::new();
        captured.insert("p".to_string(), Value::from_struct_instance(idx));
        let scope = NativeClosureScope::without_parent(captured);
        assert_eq!(reg.live_count(), 1, "scope holds the struct capture");

        // ClosureScopeInner::Drop is a no-op on struct captures (no registry in
        // hand); the explicit release reclaims the slot -- without it the count
        // would stay 1. This is the worker's per-task struct-capture leak.
        scope.release_captured_structs(&reg);
        assert_eq!(reg.live_count(), 0, "struct capture reclaimed");

        // Idempotent: a second call and the eventual Drop are no-ops (slot NILed).
        scope.release_captured_structs(&reg);
        drop(scope);
        assert_eq!(reg.live_count(), 0);
    }
}

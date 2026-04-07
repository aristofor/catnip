// FILE: catnip_rs/src/vm/frame.rs
//! Frame and CodeObject for the Catnip Rust VM.
//!
//! Mirrors catnip/vm/frame.pyx structure for compatibility.

use super::opcode::{Instruction, VMOpCode};
use super::pattern::VMPattern;
use super::structs::{StructRegistry, cascade_decref_fields};
use super::value::Value;
use crate::constants::*;
use indexmap::IndexMap;
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
    /// Cached bytecode hash for JIT trace cache (computed once on demand)
    pub(crate) bytecode_hash: std::sync::OnceLock<u64>,
    /// Bincode-encoded IR body for ND worker IPC transport.
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

impl CodeObject {
    /// Clone with Python GIL for PyObject fields.
    pub fn clone_with_py(&self, _py: Python<'_>) -> Self {
        Self {
            instructions: self.instructions.clone(),
            constants: self.constants.clone(),
            names: self.names.clone(),
            nlocals: self.nlocals,
            varnames: self.varnames.clone(),
            slotmap: self.slotmap.clone(),
            nargs: self.nargs,
            defaults: self.defaults.clone(),
            name: self.name.clone(),
            freevars: self.freevars.clone(),
            vararg_idx: self.vararg_idx,
            is_pure: self.is_pure,
            complexity: self.complexity,
            line_table: self.line_table.clone(),
            patterns: self.patterns.clone(),
            bytecode_hash: std::sync::OnceLock::new(),
            encoded_ir: self.encoded_ir.clone(),
        }
    }

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
}

/// Pure-Rust closure scope eliminating Python boundary crossings for
/// captured variable access. Uses `Rc` because sharing is single-threaded
/// only, and `RefCell` for interior mutability.
#[derive(Clone)]
pub struct NativeClosureScope {
    inner: Rc<ClosureScopeInner>,
}

impl NativeClosureScope {
    pub(crate) fn new(captured: IndexMap<String, Value>, parent: ClosureParent) -> Self {
        Self {
            inner: Rc::new(ClosureScopeInner {
                captured: RefCell::new(captured),
                parent,
            }),
        }
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

    /// Resolve only from captured vars (no parent chain). O(1) HashMap lookup.
    #[inline]
    pub fn resolve_captured_only(&self, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                return Some(val);
            }
        }
        None
    }

    /// Resolve with PyGlobals fallback (needs GIL).
    pub fn resolve_with_py(&self, py: Python<'_>, name: &str) -> Option<Value> {
        let captured = self.inner.captured.borrow();
        if let Some(&val) = captured.get(name) {
            if !val.is_nil() {
                return Some(val);
            }
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.resolve_with_py(py, name),
            ClosureParent::PyGlobals(globals) => globals
                .bind(py)
                .get_item(name)
                .ok()
                .flatten()
                .and_then(|v| Value::from_pyobject(py, &v).ok()),
            ClosureParent::Globals(globals) => globals.borrow().get(name).copied(),
            ClosureParent::None => None,
        }
    }

    /// Pure Rust set. Returns `false` when the name lives in PyGlobals.
    pub fn set(&self, name: &str, value: Value) -> bool {
        let mut captured = self.inner.captured.borrow_mut();
        if captured.contains_key(name) {
            captured.insert(name.to_string(), value);
            return true;
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.set(name, value),
            ClosureParent::Globals(globals) => {
                let mut g = globals.borrow_mut();
                if g.contains_key(name) {
                    g.insert(name.to_string(), value);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Set with PyGlobals fallback (needs GIL).
    pub fn set_with_py(&self, py: Python<'_>, name: &str, value: Value) -> bool {
        let mut captured = self.inner.captured.borrow_mut();
        if captured.contains_key(name) {
            captured.insert(name.to_string(), value);
            return true;
        }
        drop(captured);
        match &self.inner.parent {
            ClosureParent::Native(parent) => parent.set_with_py(py, name, value),
            ClosureParent::PyGlobals(globals) => {
                if let Ok(Some(_)) = globals.bind(py).get_item(name) {
                    let py_value = value.to_pyobject(py);
                    globals.bind(py).set_item(name, py_value).is_ok()
                } else {
                    false
                }
            }
            ClosureParent::Globals(globals) => {
                let mut g = globals.borrow_mut();
                if g.contains_key(name) {
                    g.insert(name.to_string(), value);
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

    /// Reset frame for reuse.
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
    pub fn bind_args(&mut self, py: Python<'_>, args: &[Value], kwargs: Option<&Bound<'_, PyDict>>) {
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

            // Bind kwargs
            if let Some(kw) = kwargs {
                for (key, value) in kw.iter() {
                    if let Ok(k) = key.extract::<String>() {
                        if let Some(&slot) = code.slotmap.get(&k) {
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
        let saved: Vec<Value> = self.locals[slot_start..].to_vec();
        self.block_stack.push((slot_start, saved));
    }

    pub fn pop_block(&mut self) {
        if let Some((slot_start, saved)) = self.block_stack.pop() {
            let saved_len = saved.len();
            for (i, val) in saved.into_iter().enumerate() {
                if slot_start + i < self.locals.len() {
                    self.locals[slot_start + i] = val;
                }
            }
            // Reset remaining slots to NIL
            for i in (slot_start + saved_len)..self.locals.len() {
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

    pub fn free(&mut self, mut frame: Frame, registry: &mut StructRegistry) {
        decref_frame_values(&frame, registry);
        if self.frames.len() < self.max_size {
            frame.reset();
            self.frames.push(frame);
        }
    }
}

/// Decref all heap values (BigInt, Complex, Struct) in a frame's stack and locals.
pub fn decref_frame_values(frame: &Frame, registry: &mut StructRegistry) {
    for &val in &frame.stack {
        if val.is_bigint() {
            val.decref_bigint();
        } else if val.is_complex() {
            val.decref();
        } else if val.is_struct_instance() {
            let idx = val.as_struct_instance_idx().unwrap();
            if let Some(fields) = registry.decref(idx) {
                cascade_decref_fields(registry, fields);
            }
        }
    }
    for &val in &frame.locals {
        if val.is_bigint() {
            val.decref_bigint();
        } else if val.is_complex() {
            val.decref();
        } else if val.is_struct_instance() {
            let idx = val.as_struct_instance_idx().unwrap();
            if let Some(fields) = registry.decref(idx) {
                cascade_decref_fields(registry, fields);
            }
        }
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
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?.into_any().unbind())
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

            // Resolve globals: parent VM > Python context > fresh builtins
            let mut host = if let Some(globals) = parent_globals {
                VMHost::with_globals(py, globals)
            } else if let Some(ctx) = &self.context {
                let globals = build_globals_from_context(py, ctx)?;
                VMHost::with_globals(py, globals)
            } else {
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

            let mut vm = super::core::VM::new();

            // Copy parent's func_table entries so VmFunc indices in closures remain valid
            if !saved_func_table.is_null() {
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
                let parent_registry = unsafe { &*saved_registry };
                vm.struct_registry.clone_from_parent(py, parent_registry);
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
            let result = vm
                .execute_with_host(py, code, &arg_values, &host, closure)
                .map_err(|e| {
                    // Restore parent pointers on error
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
            if !saved_registry.is_null() {
                let parent_registry = unsafe { &mut *(saved_registry as *mut super::structs::StructRegistry) };
                vm.struct_registry.transplant_to_parent(parent_registry);
            }

            // Restore parent pointers
            super::value::restore_struct_registry(saved_registry);
            super::value::restore_func_table(saved_func_table);

            Ok(result.to_pyobject(py))
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
        VMPattern::Struct { name, field_slots } => {
            dict.set_item("t", "s")?;
            dict.set_item("n", name.as_str())?;
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
            let fields_list = dict.get_item("f")?.unwrap().cast::<PyList>()?.clone();
            let mut field_slots = Vec::new();
            for item in fields_list.iter() {
                let pair = item.cast::<PyTuple>()?;
                let fname: String = pair.get_item(0)?.extract()?;
                let slot: usize = pair.get_item(1)?.extract()?;
                field_slots.push((fname, slot));
            }
            Ok(VMPattern::Struct { name, field_slots })
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
        let mut registry = StructRegistry::new();

        let frame1 = pool.alloc();
        let frame2 = pool.alloc();

        pool.free(frame1, &mut registry);
        pool.free(frame2, &mut registry);

        assert_eq!(pool.frames.len(), 2);
    }
}

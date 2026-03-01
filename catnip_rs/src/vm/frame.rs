// FILE: catnip_rs/src/vm/frame.rs
//! Frame and CodeObject for the Catnip Rust VM.
//!
//! Mirrors catnip/vm/frame.pyx structure for compatibility.

use super::opcode::{Instruction, VMOpCode};
use super::pattern::VMPattern;
use super::value::Value;
use crate::constants::VM_FRAME_POOL_SIZE;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use std::cell::RefCell;
use std::collections::HashMap;
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
    pub defaults: Vec<Py<PyAny>>,
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
        }
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
    pub fn clone_with_py(&self, py: Python<'_>) -> Self {
        Self {
            instructions: self.instructions.clone(),
            constants: self.constants.clone(),
            names: self.names.clone(),
            nlocals: self.nlocals,
            varnames: self.varnames.clone(),
            slotmap: self.slotmap.clone(),
            nargs: self.nargs,
            defaults: self.defaults.iter().map(|d| d.clone_ref(py)).collect(),
            name: self.name.clone(),
            freevars: self.freevars.clone(),
            vararg_idx: self.vararg_idx,
            is_pure: self.is_pure,
            complexity: self.complexity,
            line_table: self.line_table.clone(),
            patterns: self.patterns.clone(),
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

#[pymethods]
impl PyCodeObject {
    #[new]
    #[pyo3(signature = (bytecode, constants, names, nlocals, varnames, slotmap, nargs, defaults, name, freevars, vararg_idx))]
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
    ) -> PyResult<Self> {
        // Parse bytecode (tuple of (opcode, arg) pairs)
        let bytecode_seq = bytecode.cast::<PyTuple>()?;
        let mut instructions = Vec::new();
        for item in bytecode_seq.iter() {
            let pair = item.cast::<PyTuple>()?;
            let op = pair.get_item(0)?.extract::<u8>()?;
            let arg = pair.get_item(1)?.extract::<u32>()?;
            let opcode = VMOpCode::from_u8(op).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!("Invalid opcode: {}", op))
            })?;
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
        let defaults_vec: Vec<Py<PyAny>> =
            defaults_seq.iter().map(|d| d.clone().unbind()).collect();

        // Parse freevars
        let freevars_seq = freevars.cast::<PyTuple>()?;
        let freevars_vec: Vec<String> = freevars_seq
            .iter()
            .map(|s| s.extract::<String>())
            .collect::<Result<Vec<_>, _>>()?;

        // Calculate complexity as instruction count
        let complexity = instructions.len();

        Ok(Self {
            inner: Arc::new(CodeObject {
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
                is_pure: false,
                complexity,
                line_table: Vec::new(),
                patterns: Vec::new(),
            }),
        })
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
        let items: Vec<Py<PyAny>> = self
            .inner
            .constants
            .iter()
            .map(|v| v.to_pyobject(py))
            .collect();
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
        let items: Vec<Py<PyAny>> = self
            .inner
            .defaults
            .iter()
            .map(|d| d.clone_ref(py))
            .collect();
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
        // Return (class, (bytecode, constants, names, nlocals, varnames, slotmap, nargs, defaults, name, freevars, vararg_idx))
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
            ],
        )?;
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?
            .into_any()
            .unbind())
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
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Get function name.
    pub fn get_name(&self) -> &str {
        &self.inner.name
    }
}

// ---------------------------------------------------------------------------
// NativeClosureScope - pure-Rust closure chain for captured variables
// ---------------------------------------------------------------------------

/// Closure parent in the scope chain.
pub(crate) enum ClosureParent {
    /// No parent (top-level function without context)
    None,
    /// Parent is another native closure scope (nested closures)
    Native(NativeClosureScope),
    /// Terminal: module-level globals (only crossing left)
    PyGlobals(Py<PyDict>),
}

struct ClosureScopeInner {
    captured: RefCell<HashMap<String, Value>>,
    parent: ClosureParent,
}

/// Pure-Rust closure scope eliminating Python boundary crossings for
/// captured variable access. Uses `Arc` so closures from the same scope
/// share captures (e.g. counter pattern), and `RefCell` for interior
/// mutability (safe: VM is single-threaded).
#[derive(Clone)]
pub struct NativeClosureScope {
    inner: Arc<ClosureScopeInner>,
}

impl NativeClosureScope {
    pub(crate) fn new(captured: HashMap<String, Value>, parent: ClosureParent) -> Self {
        Self {
            inner: Arc::new(ClosureScopeInner {
                captured: RefCell::new(captured),
                parent,
            }),
        }
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
            _ => None,
        }
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
            ClosureParent::None => false,
        }
    }

    /// Build a NativeClosureScope with a native parent.
    pub fn with_native_parent(
        captured: HashMap<String, Value>,
        parent: NativeClosureScope,
    ) -> Self {
        Self::new(captured, ClosureParent::Native(parent))
    }

    /// Build a NativeClosureScope with PyGlobals as terminal parent.
    pub fn with_py_globals(captured: HashMap<String, Value>, globals: Py<PyDict>) -> Self {
        Self::new(captured, ClosureParent::PyGlobals(globals))
    }

    /// Build a NativeClosureScope with no parent.
    pub fn without_parent(captured: HashMap<String, Value>) -> Self {
        Self::new(captured, ClosureParent::None)
    }
}

// SAFETY: VM is single-threaded. RefCell is only accessed from the VM thread.
// The Arc is for shared ownership within the same thread (closures sharing captures).
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

/// Convert a Python ClosureScope (RustClosureScope) to NativeClosureScope.
pub fn py_scope_to_native(py: Python<'_>, scope: &Py<PyAny>) -> PyResult<NativeClosureScope> {
    let scope_bound = scope.bind(py);
    if let Ok(closure) = scope_bound.cast::<RustClosureScope>() {
        let cs = closure.borrow();
        let captured_dict = cs.captured.bind(py);
        let mut captured = HashMap::new();
        for (key, value) in captured_dict.iter() {
            let name: String = key.extract()?;
            let val = Value::from_pyobject(py, &value)?;
            captured.insert(name, val);
        }
        let parent = match &cs.parent {
            Some(p) => {
                let p_bound = p.bind(py);
                if p_bound.cast::<RustClosureScope>().is_ok() {
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

/// Convert a NativeClosureScope to a Python RustClosureScope.
pub fn native_scope_to_py(py: Python<'_>, scope: &NativeClosureScope) -> PyResult<Py<PyAny>> {
    let captured = scope.inner.captured.borrow();
    let dict = PyDict::new(py);
    for (name, &val) in captured.iter() {
        dict.set_item(name, val.to_pyobject(py))?;
    }
    drop(captured);

    let parent: Option<Py<PyAny>> = match &scope.inner.parent {
        ClosureParent::Native(p) => Some(native_scope_to_py(py, p)?),
        // Pass the dict directly (not wrapped in RustClosureScope) so that
        // py_scope_to_native detects it as PyGlobals and keeps the live reference.
        ClosureParent::PyGlobals(g) => Some(g.clone_ref(py).into_any()),
        ClosureParent::None => None,
    };

    let closure = RustClosureScope::create(dict.unbind(), parent);
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
}

impl Frame {
    /// Create a new empty frame.
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(32),
            locals: Vec::new(),
            ip: 0,
            code: None,
            block_stack: Vec::new(),
            py_scope: None,
            closure_scope: None,
            match_bindings: None,
            discard_return: false,
            super_proxy: None,
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
            stack: Vec::with_capacity(32),
            locals,
            ip: 0,
            code: Some(code),
            block_stack: Vec::new(),
            py_scope: None,
            closure_scope: None,
            match_bindings: None,
            discard_return: false,
            super_proxy: None,
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
            *self
                .stack
                .last()
                .expect("VM stack underflow (peek on empty)")
        } else {
            *self.stack.last().unwrap_or(&Value::NIL)
        }
    }

    // --- Local variable operations ---

    #[inline]
    pub fn set_local(&mut self, slot: usize, value: Value) {
        if slot < self.locals.len() {
            self.locals[slot] = value;
        }
    }

    #[inline]
    pub fn get_local(&self, slot: usize) -> Value {
        if slot < self.locals.len() {
            self.locals[slot]
        } else if cfg!(debug_assertions) {
            panic!(
                "get_local: slot {} out of bounds (nlocals={})",
                slot,
                self.locals.len()
            )
        } else {
            Value::NIL
        }
    }

    /// Bind function arguments to local slots.
    pub fn bind_args(
        &mut self,
        py: Python<'_>,
        args: &[Value],
        kwargs: Option<&Bound<'_, PyDict>>,
    ) {
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
            for i in 0..nargs_given.min(vararg_idx) {
                self.locals[i] = args[i];
            }

            // Collect excess args into vararg slot
            if nargs_given > vararg_idx {
                let excess: Vec<Py<PyAny>> = args[vararg_idx..]
                    .iter()
                    .map(|v| v.to_pyobject(py))
                    .collect();
                let list = PyList::new(py, excess).unwrap();
                self.locals[vararg_idx] =
                    Value::from_pyobject(py, &list.into_any()).unwrap_or(Value::NIL);
            } else {
                let empty = PyList::empty(py);
                self.locals[vararg_idx] =
                    Value::from_pyobject(py, &empty.into_any()).unwrap_or(Value::NIL);
            }

            // Bind kwargs
            if let Some(kw) = kwargs {
                for (key, value) in kw.iter() {
                    if let Ok(k) = key.extract::<String>() {
                        if let Some(&slot) = code.slotmap.get(&k) {
                            self.locals[slot] =
                                Value::from_pyobject(py, &value).unwrap_or(Value::NIL);
                        }
                    }
                }
            }
        } else {
            // No variadic parameter
            for i in 0..nargs_given.min(nparams) {
                self.locals[i] = args[i];
            }

            // Bind kwargs
            if let Some(kw) = kwargs {
                for (key, value) in kw.iter() {
                    if let Ok(k) = key.extract::<String>() {
                        if let Some(&slot) = code.slotmap.get(&k) {
                            self.locals[slot] =
                                Value::from_pyobject(py, &value).unwrap_or(Value::NIL);
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
                        let default_obj = code.defaults[default_idx].bind(py);
                        self.locals[i] =
                            Value::from_pyobject(py, default_obj).unwrap_or(Value::NIL);
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
        let name = self
            .code
            .as_ref()
            .map(|c| c.name.as_str())
            .unwrap_or("<no code>");
        write!(
            f,
            "<Frame {} ip={} stack_depth={}>",
            name,
            self.ip,
            self.stack.len()
        )
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
        self.frames.pop().unwrap_or_else(Frame::new)
    }

    pub fn free(&mut self, mut frame: Frame) {
        if self.frames.len() < self.max_size {
            frame.reset();
            self.frames.push(frame);
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
/// Also captures closure scope for nested functions.
#[pyclass(name = "VMFunction", module = "catnip._rs")]
pub struct RustVMFunction {
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
}

impl RustVMFunction {
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
        }
    }

    /// Create from Rust with Python closure scope (backward compat).
    pub fn create(
        py: Python<'_>,
        code: Py<PyCodeObject>,
        closure_scope: Option<Py<PyAny>>,
        context: Option<Py<PyAny>>,
    ) -> Self {
        let native_closure = closure_scope
            .as_ref()
            .and_then(|cs| py_scope_to_native(py, cs).ok());
        let name = code.borrow(py).get_name().to_string();
        Self {
            vm_code: code,
            native_closure,
            py_closure_scope: closure_scope,
            name,
            context,
        }
    }
}

#[pymethods]
impl RustVMFunction {
    #[new]
    #[pyo3(signature = (code, closure_scope=None, context=None))]
    fn new(
        code: Py<PyCodeObject>,
        closure_scope: Option<Py<PyAny>>,
        context: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let name = Python::attach(|py| {
            let n = code.borrow(py).get_name().to_string();
            n
        });
        let native_closure = Python::attach(|py| {
            closure_scope
                .as_ref()
                .and_then(|cs| py_scope_to_native(py, cs).ok())
        });
        Ok(Self {
            vm_code: code,
            native_closure,
            py_closure_scope: closure_scope,
            name,
            context,
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
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?
            .into_any()
            .unbind())
    }

    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let ctx = self.context.as_ref().ok_or_else(|| {
            pyo3::exceptions::PyTypeError::new_err(
                "VMFunction cannot be called directly without context",
            )
        })?;

        let registry = ctx.bind(py).getattr("_registry")?;
        let rust_bridge = py.import("catnip.vm.rust_bridge")?;
        let executor_class = rust_bridge.getattr("VMExecutor")?;
        let executor = executor_class.call1((registry, ctx.bind(py)))?;

        let execute_kwargs = PyDict::new(py);
        execute_kwargs.set_item("sync_globals", false)?;
        if let Some(cs) = self.closure_scope(py)? {
            execute_kwargs.set_item("closure_scope", cs.bind(py))?;
        }

        let result = executor.call_method(
            "execute",
            (self.vm_code.bind(py), args, kwargs),
            Some(&execute_kwargs),
        )?;

        Ok(result.unbind())
    }
}

/// Scope wrapper for closure capture.
///
/// Provides _resolve() and _set() methods compatible with Scope chain lookup.
/// Falls back to parent scope if name not found in captured values.
#[pyclass(name = "ClosureScope", module = "catnip._rs")]
pub struct RustClosureScope {
    /// Captured variable values
    captured: Py<PyDict>,
    /// Parent scope for chain lookup
    parent: Option<Py<PyAny>>,
}

impl RustClosureScope {
    /// Create from Rust code.
    pub fn create(captured: Py<PyDict>, parent: Option<Py<PyAny>>) -> Self {
        Self { captured, parent }
    }
}

#[pymethods]
impl RustClosureScope {
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
        let exc_module = py.import("catnip.exc")?;
        let name_error = exc_module.getattr("CatnipNameError")?;
        Err(PyErr::from_value(
            name_error.call1((format!("name '{name}' is not defined"),))?,
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
        let keys: Vec<String> = captured
            .keys()
            .iter()
            .filter_map(|k| k.extract().ok())
            .collect();
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
        Ok(PyTuple::new(py, [cls.into_any(), args.into_any()])?
            .into_any()
            .unbind())
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

        let frame1 = pool.alloc();
        let frame2 = pool.alloc();

        pool.free(frame1);
        pool.free(frame2);

        assert_eq!(pool.frames.len(), 2);
    }
}

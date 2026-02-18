// FILE: catnip_rs/src/core/registry/mod.rs
//! Registry module - Core execution engine for Catnip operations
//!
//! This module provides the Registry struct which is the main execution engine
//! for Catnip. It handles:
//! - Operation dispatch (exec_stmt)
//! - Identifier resolution
//! - Operation caching
//! - All 52 built-in operations

mod access;
mod args;
mod arithmetic;
mod bitwise;
mod broadcast;
mod control_flow;
mod execution;
mod functions;
mod literals;
mod logical;
mod nd;
mod patterns;
mod stack;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

const BUILTINS_MODULE: &str = "builtins";
const OPERATOR_MODULE: &str = "operator";

/// Cached OpCode values for fast dispatch (avoids Python lookups)
#[derive(Debug, Clone)]
pub(crate) struct OpCodeCache {
    // Arithmetic
    pub add: i32,
    pub sub: i32,
    pub mul: i32,
    pub truediv: i32,
    pub floordiv: i32,
    pub mod_: i32,
    pub pow: i32,
    pub neg: i32,
    pub pos: i32,

    // Comparison
    pub lt: i32,
    pub le: i32,
    pub gt: i32,
    pub ge: i32,
    pub eq: i32,
    pub ne: i32,

    // Logical
    pub and: i32,
    pub or: i32,
    pub not: i32,

    // Bitwise
    pub bor: i32,
    pub bxor: i32,
    pub band: i32,
    pub bnot: i32,
    pub lshift: i32,
    pub rshift: i32,

    // Control flow
    pub op_for: i32,
    pub op_while: i32,
    pub op_if: i32,
    pub op_block: i32,
    pub op_return: i32,
    pub op_break: i32,
    pub op_continue: i32,
    pub set_locals: i32,
    pub call: i32,

    // Access
    pub getattr: i32,
    pub getitem: i32,
    pub setattr: i32,
    pub setitem: i32,
    pub slice: i32,

    // Literals
    pub list_literal: i32,
    pub tuple_literal: i32,
    pub set_literal: i32,
    pub dict_literal: i32,
    pub fstring: i32,

    // Stack
    pub push: i32,
    pub push_peek: i32,
    pub pop: i32,

    // Patterns
    pub op_match: i32,

    // ND
    pub nd_empty_topos: i32,
    pub nd_recursion: i32,
    pub nd_map: i32,

    // Struct
    pub op_struct: i32,

    // Trait
    pub trait_def: i32,
}

impl OpCodeCache {
    /// Initialize OpCode cache from Python OpCode enum
    fn new(py: Python<'_>) -> PyResult<Self> {
        let opcode_module = py.import("catnip.semantic.opcode")?;
        let opcode_class = opcode_module.getattr("OpCode")?;

        let get = |name: &str| -> PyResult<i32> { opcode_class.getattr(name)?.extract() };

        Ok(Self {
            add: get("ADD")?,
            sub: get("SUB")?,
            mul: get("MUL")?,
            truediv: get("TRUEDIV")?,
            floordiv: get("FLOORDIV")?,
            mod_: get("MOD")?,
            pow: get("POW")?,
            neg: get("NEG")?,
            pos: get("POS")?,

            lt: get("LT")?,
            le: get("LE")?,
            gt: get("GT")?,
            ge: get("GE")?,
            eq: get("EQ")?,
            ne: get("NE")?,

            and: get("AND")?,
            or: get("OR")?,
            not: get("NOT")?,

            bor: get("BOR")?,
            bxor: get("BXOR")?,
            band: get("BAND")?,
            bnot: get("BNOT")?,
            lshift: get("LSHIFT")?,
            rshift: get("RSHIFT")?,

            op_for: get("OP_FOR")?,
            op_while: get("OP_WHILE")?,
            op_if: get("OP_IF")?,
            op_block: get("OP_BLOCK")?,
            op_return: get("OP_RETURN")?,
            op_break: get("OP_BREAK")?,
            op_continue: get("OP_CONTINUE")?,
            set_locals: get("SET_LOCALS")?,
            call: get("CALL")?,

            getattr: get("GETATTR")?,
            getitem: get("GETITEM")?,
            setattr: get("SETATTR")?,
            setitem: get("SETITEM")?,
            slice: get("SLICE")?,

            list_literal: get("LIST_LITERAL")?,
            tuple_literal: get("TUPLE_LITERAL")?,
            set_literal: get("SET_LITERAL")?,
            dict_literal: get("DICT_LITERAL")?,
            fstring: get("FSTRING")?,

            push: get("PUSH")?,
            push_peek: get("PUSH_PEEK")?,
            pop: get("POP")?,

            op_match: get("OP_MATCH")?,

            nd_empty_topos: get("ND_EMPTY_TOPOS")?,
            nd_recursion: get("ND_RECURSION")?,
            nd_map: get("ND_MAP")?,

            op_struct: get("OP_STRUCT")?,

            trait_def: get("TRAIT_DEF")?,
        })
    }
}

/// Cached operator functions for fast arithmetic/logical dispatch.
#[derive(Debug)]
pub(crate) struct OperatorCache {
    pub add: Py<PyAny>,
    pub sub: Py<PyAny>,
    pub mul: Py<PyAny>,
    pub truediv: Py<PyAny>,
    pub floordiv: Py<PyAny>,
    pub mod_: Py<PyAny>,
    pub pow: Py<PyAny>,
    pub lt: Py<PyAny>,
    pub le: Py<PyAny>,
    pub gt: Py<PyAny>,
    pub ge: Py<PyAny>,
    pub eq: Py<PyAny>,
    pub ne: Py<PyAny>,
    pub and_: Py<PyAny>,
    pub or_: Py<PyAny>,
    pub xor: Py<PyAny>,
    pub lshift: Py<PyAny>,
    pub rshift: Py<PyAny>,
    pub neg: Py<PyAny>,
    pub pos: Py<PyAny>,
    pub invert: Py<PyAny>,
    pub not_: Py<PyAny>,
    pub abs: Py<PyAny>,
}

impl OperatorCache {
    fn new(py: Python<'_>) -> PyResult<Self> {
        let operator = py.import(OPERATOR_MODULE)?;
        let builtins = py.import(BUILTINS_MODULE)?;
        Ok(Self {
            add: operator.getattr("add")?.unbind(),
            sub: operator.getattr("sub")?.unbind(),
            mul: operator.getattr("mul")?.unbind(),
            truediv: operator.getattr("truediv")?.unbind(),
            floordiv: operator.getattr("floordiv")?.unbind(),
            mod_: operator.getattr("mod")?.unbind(),
            pow: operator.getattr("pow")?.unbind(),
            lt: operator.getattr("lt")?.unbind(),
            le: operator.getattr("le")?.unbind(),
            gt: operator.getattr("gt")?.unbind(),
            ge: operator.getattr("ge")?.unbind(),
            eq: operator.getattr("eq")?.unbind(),
            ne: operator.getattr("ne")?.unbind(),
            and_: operator.getattr("and_")?.unbind(),
            or_: operator.getattr("or_")?.unbind(),
            xor: operator.getattr("xor")?.unbind(),
            lshift: operator.getattr("lshift")?.unbind(),
            rshift: operator.getattr("rshift")?.unbind(),
            neg: operator.getattr("neg")?.unbind(),
            pos: operator.getattr("pos")?.unbind(),
            invert: operator.getattr("invert")?.unbind(),
            not_: operator.getattr("not_")?.unbind(),
            abs: builtins.getattr("abs")?.unbind(),
        })
    }
}

/// Core Registry struct exposed to Python
///
/// This is the main execution engine that handles all Catnip operations.
/// Migrated from Cython registry_core.pyx to Rust for better performance.
///
/// Uses RefCell for interior mutability to allow recursive exec_stmt calls
/// (e.g., when a Lambda calls exec_stmt during its execution).
///
/// `unsendable` is used because RefCell is not Sync. This is fine because
/// each Python process/interpreter has its own Registry instance.
#[pyclass(name = "Registry", subclass, unsendable)]
pub struct Registry {
    /// Execution context (catnip.context.Context)
    #[pyo3(get)]
    pub(crate) ctx: Py<PyAny>,

    /// Internal operations map (Python dict, exposed to allow Python subclass to modify)
    /// Keys can be OpCode ints or strings for backward compatibility
    /// Type is PyObject to allow PyDict
    #[pyo3(get)]
    pub(crate) internals: Py<PyAny>,

    /// Operation cache for faster lookup (opcode int → callable)
    /// Wrapped in RefCell for interior mutability during recursive calls
    pub(crate) op_cache: RefCell<HashMap<i32, Py<PyAny>>>,

    /// Cache enabled flag
    #[pyo3(get, set)]
    pub(crate) cache_enabled: bool,

    /// Stack for stack operations (push, pop, push_peek)
    /// Wrapped in RefCell for interior mutability during recursive calls
    pub(crate) stack: RefCell<Vec<Py<PyAny>>>,

    /// Control flow opcodes (for checking if args should be evaluated)
    pub(crate) control_flow_ops: HashSet<i32>,

    /// Cached OpCode values for fast dispatch
    pub(crate) opcodes: OpCodeCache,

    /// Cached operator functions for fast op dispatch
    pub(crate) operator_cache: OperatorCache,
}

#[pymethods]
impl Registry {
    /// Create a new Registry with the given context
    #[new]
    fn new(py: Python<'_>, context: Py<PyAny>) -> PyResult<Self> {
        // Initialize control flow ops from Python BEFORE creating the registry
        let opcode_module = py.import("catnip.semantic.opcode")?;
        let control_flow_ops_py = opcode_module.getattr("CONTROL_FLOW_OPS")?;
        let mut control_flow_ops = HashSet::new();
        for op in control_flow_ops_py.try_iter()? {
            let op_value: i32 = op?.extract()?;
            control_flow_ops.insert(op_value);
        }

        // Initialize caches
        let opcodes = OpCodeCache::new(py)?;
        let operator_cache = OperatorCache::new(py)?;

        let registry = Self {
            ctx: context.clone_ref(py),
            internals: PyDict::new(py).unbind().into(),
            op_cache: RefCell::new(HashMap::new()),
            cache_enabled: true,
            stack: RefCell::new(Vec::new()),
            control_flow_ops,
            opcodes,
            operator_cache,
        };

        // Register operations in internals for semantic analysis
        // The semantic analyzer checks if opcode in registry.internals
        // We put True as a marker - actual dispatch happens via try_rust_dispatch()
        registry.register_operations(py)?;

        // Update context globals with utility functions
        registry.update_context_globals(py)?;

        Ok(registry)
    }

    /// Execute a statement (main dispatch function)
    ///
    /// This is the core of the Catnip execution engine. It handles:
    /// - Op nodes: dispatch to registered operations
    /// - Ref nodes: resolve identifiers
    /// - Broadcast nodes: handle broadcasting
    /// - Literals: return as-is
    fn exec_stmt(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.exec_stmt_impl(py, stmt)
    }

    /// Resolve and execute a statement (alias for exec_stmt)
    ///
    /// This is an alias for backward compatibility with tests and older code.
    /// Use exec_stmt() for new code.
    fn resolve_stmt(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.exec_stmt_impl(py, stmt)
    }

    /// Resolve an identifier from locals/globals
    ///
    /// Args:
    ///     ident: The identifier name
    ///     check: If True, raise error if not found. If False, return None.
    ///
    /// Returns:
    ///     The resolved value or None
    #[pyo3(signature = (ident, check=true))]
    fn resolve_ident(
        &self,
        py: Python<'_>,
        ident: &str,
        check: bool,
    ) -> PyResult<Option<Py<PyAny>>> {
        self.resolve_ident_impl(py, ident, check)
    }

    /// Clear the operation cache
    fn clear_cache(&self) {
        self.op_cache.borrow_mut().clear();
    }

    /// Enable or disable operation caching
    #[pyo3(signature = (enabled=true))]
    fn enable_cache(&self, _py: Python<'_>, enabled: bool) {
        // Use PyCell to modify cache_enabled since it's not in RefCell
        // For now, just clear cache if disabling (cache_enabled is set via pyo3 setter)
        if !enabled {
            self.clear_cache();
        }
    }

    // ========================================
    // Arithmetic Operations (exposed to Python)
    // ========================================

    /// Unary negation: -value
    fn _neg(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_neg(py, stmt)
    }

    /// Unary positive: +value
    fn _pos(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_pos(py, stmt)
    }

    /// Bitwise inversion: ~value (Note: also called "inv" in some contexts)
    fn _inv(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_inv(py, stmt)
    }

    /// Addition: fold left with __add__
    #[pyo3(signature = (*items))]
    fn _add(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_add(py, items)
    }

    /// Subtraction: fold left with __sub__
    #[pyo3(signature = (*items))]
    fn _sub(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_sub(py, items)
    }

    /// Multiplication: fold left with __mul__
    #[pyo3(signature = (*items))]
    fn _mul(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_mul(py, items)
    }

    /// True division: fold left with __truediv__
    #[pyo3(signature = (*items))]
    fn _truediv(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_truediv(py, items)
    }

    /// Floor division: fold left with __floordiv__
    #[pyo3(signature = (*items))]
    fn _floordiv(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_floordiv(py, items)
    }

    /// Modulo: fold left with __mod__
    #[pyo3(signature = (*items))]
    fn _mod(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_mod(py, items)
    }

    /// Power: fold left with __pow__
    #[pyo3(signature = (*items))]
    fn _pow(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_pow(py, items)
    }

    // ========================================
    // Logical Operations (exposed to Python)
    // ========================================

    /// Boolean NOT: not value
    fn _bool_not(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_bool_not(py, stmt)
    }

    /// Boolean OR: short-circuit evaluation
    #[pyo3(signature = (*items))]
    fn _bool_or(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_bool_or(py, items)
    }

    /// Boolean AND: short-circuit evaluation
    #[pyo3(signature = (*items))]
    fn _bool_and(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_bool_and(py, items)
    }

    /// Less than: a < b < c < ...
    #[pyo3(signature = (*items))]
    fn _lt(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_lt(py, items)
    }

    /// Less than or equal: a <= b <= c <= ...
    #[pyo3(signature = (*items))]
    fn _le(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_le(py, items)
    }

    /// Greater than: a > b > c > ...
    #[pyo3(signature = (*items))]
    fn _gt(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_gt(py, items)
    }

    /// Greater than or equal: a >= b >= c >= ...
    #[pyo3(signature = (*items))]
    fn _ge(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_ge(py, items)
    }

    /// Equality: a == b == c == ...
    #[pyo3(signature = (*items))]
    fn _eq(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_eq(py, items)
    }

    /// Not equal: a != b != c != ...
    #[pyo3(signature = (*items))]
    fn _ne(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_ne(py, items)
    }

    // ========================================
    // Bitwise Operations (exposed to Python)
    // ========================================

    /// Bitwise OR: fold left with |
    #[pyo3(signature = (*items))]
    fn _bit_or(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_bit_or(py, items)
    }

    /// Bitwise XOR: fold left with ^
    #[pyo3(signature = (*items))]
    fn _bit_xor(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_bit_xor(py, items)
    }

    /// Bitwise AND: fold left with &
    #[pyo3(signature = (*items))]
    fn _bit_and(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_bit_and(py, items)
    }

    /// Left shift: fold left with <<
    #[pyo3(signature = (*items))]
    fn _lshift(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_lshift(py, items)
    }

    /// Right shift: fold left with >>
    #[pyo3(signature = (*items))]
    fn _rshift(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_rshift(py, items)
    }

    // ========================================
    // Stack Operations (exposed to Python)
    // ========================================

    /// Push a value onto the stack (no return value)
    fn _push(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_push(py, stmt)?;
        Ok(py.None())
    }

    /// Push a value onto the stack and return it
    fn _push_peek(&self, py: Python<'_>, stmt: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.op_push_peek(py, stmt)
    }

    /// Pop a value from the stack
    fn _pop(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.op_pop(py)
    }

    // ========================================
    // Literal Operations (exposed to Python)
    // ========================================

    /// Create a list literal from items
    #[pyo3(signature = (*items))]
    fn _list_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_list_literal(py, items)
    }

    /// Create a tuple literal from items
    #[pyo3(signature = (*items))]
    fn _tuple_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_tuple_literal(py, items)
    }

    /// Create a set literal from items
    #[pyo3(signature = (*items))]
    fn _set_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_set_literal(py, items)
    }

    /// Create a dict literal from key-value pairs
    #[pyo3(signature = (*pairs))]
    fn _dict_literal(&self, py: Python<'_>, pairs: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_dict_literal(py, pairs)
    }

    /// Evaluate an f-string template
    #[pyo3(signature = (*parts))]
    fn _fstring(&self, py: Python<'_>, parts: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_fstring(py, parts)
    }

    // ========================================
    // Access Operations (exposed to Python)
    // ========================================

    /// Get an attribute from an object: getattr(parent, ident)
    #[pyo3(signature = (*args))]
    fn _getattr(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_getattr(py, args)
    }

    /// Get an item from an object: obj[index]
    #[pyo3(signature = (*args))]
    fn _getitem(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_getitem(py, args)
    }

    /// Set an attribute on an object: setattr(obj, attr, value)
    #[pyo3(signature = (*args))]
    fn _setattr(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_setattr(py, args)
    }

    /// Set an item in an object: obj[index] = value
    #[pyo3(signature = (*args))]
    fn _setitem(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_setitem(py, args)
    }

    /// Create a slice object: slice(start, stop, step)
    #[pyo3(signature = (*args))]
    fn _slice(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_slice(py, args)
    }

    // ========================================
    // Control Flow Operations (exposed to Python)
    // ========================================

    /// Set local variables: set_locals((names,), value)
    #[pyo3(signature = (*args))]
    fn _set_locals(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_set_locals(py, args)
    }

    /// Execute a for loop: for identifier in iterable { block }
    #[pyo3(signature = (*args))]
    fn _for(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_for(py, args)
    }

    /// Execute a while loop: while condition { block }
    #[pyo3(signature = (*args))]
    fn _while(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_while(py, args)
    }

    /// Execute a block of statements: { stmt1; stmt2; ... }
    #[pyo3(signature = (*args))]
    fn _block(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_block(py, args)
    }

    /// Execute conditional branches: if/elif/else
    #[pyo3(signature = (*args))]
    fn _if(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_if(py, args)
    }

    /// Return from a function or lambda
    #[pyo3(signature = (*args))]
    fn _return(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_return(py, args)
    }

    /// Break out of the current loop
    #[pyo3(signature = (*args))]
    fn _break(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_break(py, args)
    }

    /// Continue to the next iteration of the current loop
    #[pyo3(signature = (*args))]
    fn _continue(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        self.op_continue(py, args)
    }

    // ========================================
    // Pattern Matching Operations (exposed to Python)
    // ========================================

    /// Match a pattern against a value (for VM bytecode execution).
    ///
    /// Args:
    ///     pattern: Pattern object (PatternWildcard, PatternLiteral, PatternVar, PatternOr, PatternTuple)
    ///     value: Value to match
    ///
    /// Returns:
    ///     Dict with variable bindings if match succeeds, None if it fails
    fn _match_pattern(
        &self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        use pyo3::types::PyDict;

        let value_obj = value.clone().unbind();
        match self.match_pattern(py, pattern, &value_obj)? {
            Some(bindings) => {
                // Convert Vec to PyDict
                let dict = PyDict::new(py);
                for (key, val) in bindings {
                    dict.set_item(key, val)?;
                }
                Ok(dict.into())
            }
            None => Ok(py.None()),
        }
    }

    // ========================================
    // Function Operations (exposed to Python)
    // ========================================

    /// Create a lambda (anonymous function)
    #[pyo3(signature = (*args))]
    fn _lambda(slf: &Bound<'_, Self>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        Registry::op_lambda(slf, args)
    }

    // ========================================
    // Broadcasting Operations (exposed to Python)
    // ========================================

    /// Handle Broadcast nodes (internal method exposed for Python wrapper)
    ///
    /// This method is called by the Python Registry wrapper when handling
    /// explicit broadcast() calls. Most broadcast operations go directly
    /// through exec_stmt which internally calls handle_broadcast.
    fn _handle_broadcast(&self, py: Python<'_>, node: Py<PyAny>) -> PyResult<Py<PyAny>> {
        self.handle_broadcast(py, node)
    }

    /// Helper method to call broadcast_map from Python
    ///
    /// This is used as a callback in apply_broadcast to map functions over collections.
    fn _broadcast_map_helper(
        &self,
        py: Python<'_>,
        target: Py<PyAny>,
        func: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        use crate::core::registry::broadcast;
        let target = target.bind(py);
        let func = func.bind(py);
        broadcast::broadcast_map(py, target, func)
    }

    /// Apply broadcasting operation (exposed for VM bytecode execution).
    ///
    /// Args:
    ///     target: Target collection to broadcast over
    ///     operator: Operator to apply (string, callable, or expression)
    ///     operand: Optional operand for binary operations (defaults to None)
    ///     is_filter: If True, treat as filter operation (defaults to False)
    ///
    /// Returns:
    ///     Result of broadcasting operation
    #[pyo3(signature = (target, operator, operand=None, is_filter=false))]
    fn _apply_broadcast(
        &self,
        py: Python<'_>,
        target: &Bound<'_, PyAny>,
        operator: &Bound<'_, PyAny>,
        operand: Option<&Bound<'_, PyAny>>,
        is_filter: bool,
    ) -> PyResult<Py<PyAny>> {
        let target_obj = target.clone().unbind();
        let operator_obj = operator.clone().unbind();
        let operand_obj = operand
            .map(|o| o.clone().unbind())
            .unwrap_or_else(|| py.None());

        self.apply_broadcast(py, target_obj, operator_obj, operand_obj, is_filter)
    }

    // ========================================
    // ND Operations (exposed to Python)
    // ========================================

    /// Execute ND-recursion starting from seed (Python-exposed wrapper)
    pub fn execute_nd_recursion_py(
        &self,
        py: Python<'_>,
        seed: Py<PyAny>,
        nd_lambda: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        self.execute_nd_recursion(py, &seed, &nd_lambda)
    }

    /// Execute ND-map on data (Python-exposed wrapper)
    pub fn execute_nd_map_py(
        &self,
        py: Python<'_>,
        data: Py<PyAny>,
        func: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        self.execute_nd_map(py, &data, &func)
    }
}

impl Registry {
    /// Generic fold-left operation using operator.* functions
    ///
    /// Reduces a sequence of values using an operator function from the `operator` module.
    ///
    /// # Arguments
    /// * `items` - Tuple of arguments to reduce
    /// * `op_func` - Bound operator function (e.g., operator.add)
    /// * `default_value` - Default value to return if items is empty (None = error)
    pub(crate) fn fold_left_operator(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
        op_func: &Bound<'_, PyAny>,
        default_value: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.is_empty() {
            return default_value.map_or_else(
                || {
                    Err(pyo3::exceptions::PyTypeError::new_err(
                        "requires at least one argument",
                    ))
                },
                Ok,
            );
        }
        let mut result = args[0].clone_ref(py);
        for val in &args[1..] {
            result = op_func.call1((result.bind(py), val))?.unbind();
        }
        Ok(result)
    }

    /// Generic fold-left with magic methods (__or__, __xor__, etc.)
    ///
    /// Reduces a sequence of values using a magic method.
    ///
    /// # Arguments
    /// * `items` - Tuple of arguments to reduce
    /// * `method_name` - Name of the magic method (e.g., "__or__")
    pub(crate) fn fold_left_magic(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
        method_name: &str,
    ) -> PyResult<Py<PyAny>> {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.is_empty() {
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "{} requires at least one argument",
                method_name
            )));
        }
        let mut result = args[0].clone_ref(py);
        for val in &args[1..] {
            result = result.call_method1(py, method_name, (val,))?;
        }
        Ok(result)
    }

    /// Generic chained comparison (a < b < c)
    ///
    /// Evaluates a chain of comparisons, short-circuiting on first failure.
    ///
    /// # Arguments
    /// * `items` - Tuple of values to compare
    /// * `compare_fn` - Comparison function returning bool
    pub(crate) fn chained_comparison<F>(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
        compare_fn: F,
    ) -> PyResult<Py<PyAny>>
    where
        F: Fn(&Bound<'_, PyAny>, &Bound<'_, PyAny>) -> PyResult<bool>,
    {
        let args = self.unwrap_and_eval_args(py, items)?;
        if args.is_empty() {
            return Ok(true.into_pyobject(py).unwrap().to_owned().unbind().into());
        }
        let mut left = args[0].clone_ref(py);
        for val in &args[1..] {
            if !compare_fn(left.bind(py), val.bind(py))? {
                return Ok(false.into_pyobject(py).unwrap().to_owned().unbind().into());
            }
            left = val.clone_ref(py);
        }
        Ok(true.into_pyobject(py).unwrap().to_owned().unbind().into())
    }

    /// Register all operations in the internals dict
    fn register_operations(&self, py: Python<'_>) -> PyResult<()> {
        // Import OpCode enum
        let opcode_module = py.import("catnip.semantic.opcode")?;
        let opcode_class = opcode_module.getattr("OpCode")?;

        // Helper to get OpCode value
        let get_opcode = |name: &str| -> PyResult<i32> {
            let op = opcode_class.getattr(name)?;
            op.extract()
        };

        // Register all operations (stubs for now)
        // Stack operations
        self.register_op(py, "push", get_opcode("PUSH")?)?;
        self.register_op(py, "pop", get_opcode("POP")?)?;
        self.register_op(py, "push_peek", get_opcode("PUSH_PEEK")?)?;

        // Arithmetic operations
        self.register_op(py, "add", get_opcode("ADD")?)?;
        self.register_op(py, "sub", get_opcode("SUB")?)?;
        self.register_op(py, "mul", get_opcode("MUL")?)?;
        self.register_op(py, "truediv", get_opcode("TRUEDIV")?)?;
        self.register_op(py, "floordiv", get_opcode("FLOORDIV")?)?;
        self.register_op(py, "mod", get_opcode("MOD")?)?;
        self.register_op(py, "pow", get_opcode("POW")?)?;
        self.register_op(py, "neg", get_opcode("NEG")?)?;
        self.register_op(py, "pos", get_opcode("POS")?)?;

        // Register DIV as alias for TRUEDIV
        let div_opcode = get_opcode("DIV")?;
        let truediv_opcode = get_opcode("TRUEDIV")?;
        let dict = self.internals.bind(py);
        let truediv_func = dict.call_method1("get", (truediv_opcode,))?;
        if !truediv_func.is_none() {
            dict.call_method1("__setitem__", (div_opcode, truediv_func))?;
        }

        // Logical operations
        self.register_op(py, "bool_not", get_opcode("NOT")?)?;
        self.register_op(py, "bool_or", get_opcode("OR")?)?;
        self.register_op(py, "bool_and", get_opcode("AND")?)?;
        self.register_op(py, "lt", get_opcode("LT")?)?;
        self.register_op(py, "le", get_opcode("LE")?)?;
        self.register_op(py, "gt", get_opcode("GT")?)?;
        self.register_op(py, "ge", get_opcode("GE")?)?;
        self.register_op(py, "eq", get_opcode("EQ")?)?;
        self.register_op(py, "ne", get_opcode("NE")?)?;

        // Bitwise operations (IMPLEMENTED in Rust)
        self.register_op(py, "bit_not", get_opcode("BNOT")?)?; // inv/~
        self.register_op(py, "bit_or", get_opcode("BOR")?)?;
        self.register_op(py, "bit_xor", get_opcode("BXOR")?)?;
        self.register_op(py, "bit_and", get_opcode("BAND")?)?;
        self.register_op(py, "lshift", get_opcode("LSHIFT")?)?;
        self.register_op(py, "rshift", get_opcode("RSHIFT")?)?;

        // Access operations (IMPLEMENTED in Rust)
        self.register_op(py, "getattr", get_opcode("GETATTR")?)?;
        self.register_op(py, "getitem", get_opcode("GETITEM")?)?;
        self.register_op(py, "setattr", get_opcode("SETATTR")?)?;
        self.register_op(py, "setitem", get_opcode("SETITEM")?)?;
        self.register_op(py, "slice", get_opcode("SLICE")?)?;

        // Control flow operations (IMPLEMENTED in Rust)
        self.register_op(py, "set_locals", get_opcode("SET_LOCALS")?)?;
        self.register_op(py, "for", get_opcode("OP_FOR")?)?;
        self.register_op(py, "while", get_opcode("OP_WHILE")?)?;
        self.register_op(py, "block", get_opcode("OP_BLOCK")?)?;
        self.register_op(py, "if", get_opcode("OP_IF")?)?;
        self.register_op(py, "return", get_opcode("OP_RETURN")?)?;
        self.register_op(py, "break", get_opcode("OP_BREAK")?)?;
        self.register_op(py, "continue", get_opcode("OP_CONTINUE")?)?;

        // Function operations (100% Rust: lambda factory + call with TCO detection)
        self.register_op(py, "lambda", get_opcode("OP_LAMBDA")?)?;
        self.register_op(py, "call", get_opcode("CALL")?)?;

        // Pattern matching (IMPLEMENTED in Rust)
        self.register_op(py, "match", get_opcode("OP_MATCH")?)?;

        // ND operations (IMPLEMENTED in Rust)
        self.register_op(py, "nd_empty_topos", get_opcode("ND_EMPTY_TOPOS")?)?;
        self.register_op(py, "nd_recursion", get_opcode("ND_RECURSION")?)?;
        self.register_op(py, "nd_map", get_opcode("ND_MAP")?)?;

        // Literals (IMPLEMENTED in Rust)
        self.register_op(py, "list_literal", get_opcode("LIST_LITERAL")?)?;
        self.register_op(py, "tuple_literal", get_opcode("TUPLE_LITERAL")?)?;
        self.register_op(py, "set_literal", get_opcode("SET_LITERAL")?)?;
        self.register_op(py, "dict_literal", get_opcode("DICT_LITERAL")?)?;
        self.register_op(py, "fstring", get_opcode("FSTRING")?)?;

        // Struct (dataclass-based)
        self.register_op(py, "struct", get_opcode("OP_STRUCT")?)?;

        // Trait
        self.register_op(py, "trait_def", get_opcode("TRAIT_DEF")?)?;

        // Broadcasting (NOT implemented, use Cython fallback)
        // NOT registering: broadcast, nd_recursion, nd_map, nd_empty_topos

        Ok(())
    }

    /// Register a single operation
    fn register_op(&self, py: Python<'_>, _name: &str, opcode: i32) -> PyResult<()> {
        // Store marker (True) so semantic analyzer knows this opcode is valid
        // Actual dispatch happens via try_rust_dispatch() in exec_op
        let dict = self.internals.bind(py);
        // Use __setitem__ instead of set_item to work with PyDict
        dict.call_method1("__setitem__", (opcode, true))?;

        Ok(())
    }

    /// Update context globals with utility functions
    fn update_context_globals(&self, py: Python<'_>) -> PyResult<()> {
        let ctx = self.ctx.bind(py);
        let globals = ctx.getattr("globals")?;
        let globals_dict = globals.cast::<PyDict>()?;

        // Add globals() and locals() utility functions (stubs for now)
        let globals_fn = py.eval(c"lambda: {}", None, None)?;
        let locals_fn = py.eval(c"lambda: {}", None, None)?;

        globals_dict.set_item("globals", globals_fn)?;
        globals_dict.set_item("locals", locals_fn)?;

        Ok(())
    }
}

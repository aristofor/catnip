// FILE: catnip_rs/src/vm/compiler_input.rs
//! Abstraction layer over Op (PyObject) and IR input formats.
//!
//! `CompilerNode` wraps either a Python object or an IR reference,
//! providing uniform accessors for the unified bytecode compiler.

use crate::core::Op;
use crate::ir::{IR, IROpCode};
use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// A compiler input node: either a Python object (Op pipeline) or an IR reference (standalone pipeline).
#[derive(Clone)]
pub enum CompilerNode<'py> {
    /// Python object from the Op-based pipeline (GIL-pinned)
    PyObj(Bound<'py, PyAny>),
    /// Reference to an IR node from the standalone pipeline
    Pure(&'py IR),
}

/// Keyword arguments for a compiler operation.
pub enum CompilerKwargs<'py> {
    /// Python dict from the Op pipeline
    Py(Bound<'py, PyDict>),
    /// HashMap reference from the IR pipeline
    Pure(&'py IndexMap<String, IR>),
    /// No kwargs
    Empty,
}

// ========== CompilerNode ==========

impl<'py> CompilerNode<'py> {
    // ========== Child access ==========

    /// For PyObj: get the indexable sequence (Op.args for Op nodes, else self).
    fn py_seq<'a>(obj: &'a Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        if let Ok(op) = obj.extract::<pyo3::PyRef<'_, Op>>() {
            Ok(op.args.bind(obj.py()).clone().into_any())
        } else {
            Ok(obj.clone())
        }
    }

    /// Get child at index (0-based). Works on sequences and Op args.
    pub fn child(&self, _py: Python<'py>, idx: usize) -> PyResult<CompilerNode<'py>> {
        match self {
            CompilerNode::PyObj(obj) => {
                let seq = Self::py_seq(obj)?;
                Ok(CompilerNode::PyObj(seq.get_item(idx)?))
            }
            CompilerNode::Pure(ir) => {
                let items: &[IR] = match ir {
                    IR::Tuple(v) | IR::List(v) | IR::Program(v) | IR::Set(v) => v,
                    IR::Op { args, .. } => args,
                    _ => {
                        return Err(pyo3::exceptions::PyTypeError::new_err("node is not indexable"));
                    }
                };
                items.get(idx).map(CompilerNode::Pure).ok_or_else(|| {
                    pyo3::exceptions::PyIndexError::new_err(format!("index {} out of range (len {})", idx, items.len()))
                })
            }
        }
    }

    /// Number of children (sequence length or Op args count).
    pub fn children_len(&self, _py: Python<'py>) -> PyResult<usize> {
        match self {
            CompilerNode::PyObj(obj) => {
                let seq = Self::py_seq(obj)?;
                seq.len()
            }
            CompilerNode::Pure(ir) => Ok(match ir {
                IR::Tuple(v) | IR::List(v) | IR::Program(v) | IR::Set(v) => v.len(),
                IR::Op { args, .. } => args.len(),
                _ => 0,
            }),
        }
    }

    /// All children as a vec of CompilerNode.
    pub fn children(&self, _py: Python<'py>) -> PyResult<Vec<CompilerNode<'py>>> {
        match self {
            CompilerNode::PyObj(obj) => {
                let seq = Self::py_seq(obj)?;
                let len = seq.len()?;
                (0..len).map(|i| Ok(CompilerNode::PyObj(seq.get_item(i)?))).collect()
            }
            CompilerNode::Pure(ir) => {
                let items: &[IR] = match ir {
                    IR::Tuple(v) | IR::List(v) | IR::Program(v) | IR::Set(v) => v,
                    IR::Op { args, .. } => args,
                    _ => &[],
                };
                Ok(items.iter().map(CompilerNode::Pure).collect())
            }
        }
    }

    /// Check if node is a list or tuple (for unwrapping binary args).
    pub fn is_list_or_tuple(&self) -> bool {
        match self {
            CompilerNode::PyObj(obj) => obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>(),
            CompilerNode::Pure(ir) => matches!(ir, IR::List(_) | IR::Tuple(_)),
        }
    }

    /// Check if this is a tuple.
    pub fn is_tuple(&self) -> bool {
        match self {
            CompilerNode::PyObj(obj) => obj.is_instance_of::<PyTuple>(),
            CompilerNode::Pure(ir) => matches!(ir, IR::Tuple(_)),
        }
    }

    // ========== Typed extraction ==========

    /// Extract as string (from String, Identifier, Ref, or PyString).
    pub fn as_string(&self) -> PyResult<String> {
        match self {
            CompilerNode::PyObj(obj) => obj.extract(),
            CompilerNode::Pure(ir) => match ir {
                IR::String(s) | IR::Identifier(s) | IR::Ref(s, _, _) => Ok(s.clone()),
                _ => Err(pyo3::exceptions::PyTypeError::new_err("expected string")),
            },
        }
    }

    /// Extract as i64.
    pub fn as_int(&self) -> PyResult<i64> {
        match self {
            CompilerNode::PyObj(obj) => obj.extract(),
            CompilerNode::Pure(ir) => match ir {
                IR::Int(n) => Ok(*n),
                _ => Err(pyo3::exceptions::PyTypeError::new_err("expected int")),
            },
        }
    }

    /// Extract as bool.
    pub fn as_bool(&self) -> PyResult<bool> {
        match self {
            CompilerNode::PyObj(obj) => obj.extract(),
            CompilerNode::Pure(ir) => match ir {
                IR::Bool(b) => Ok(*b),
                _ => Err(pyo3::exceptions::PyTypeError::new_err("expected bool")),
            },
        }
    }

    /// Try to extract a variable name (from Ref, Identifier, String, Lvalue).
    /// Returns None if the node isn't a name-like type.
    pub fn as_name(&self, _py: Python<'py>) -> Option<String> {
        match self {
            CompilerNode::PyObj(obj) => {
                if let Ok(s) = obj.extract::<String>() {
                    return Some(s);
                }
                let type_name = obj.get_type().name().ok()?;
                match type_name.to_str().unwrap_or("") {
                    "Ref" => obj.getattr("ident").ok()?.extract().ok(),
                    "Lvalue" => obj.getattr("value").ok()?.extract().ok(),
                    "Identifier" => obj.getattr("name").ok()?.extract().ok(),
                    _ => None,
                }
            }
            CompilerNode::Pure(ir) => match ir {
                IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => Some(name.clone()),
                _ => None,
            },
        }
    }

    /// Check if this represents None/nil.
    pub fn is_none_value(&self) -> bool {
        match self {
            CompilerNode::PyObj(obj) => obj.is_none(),
            CompilerNode::Pure(ir) => matches!(ir, IR::None),
        }
    }

    /// Check if this is a Bool node.
    pub fn is_bool(&self) -> bool {
        match self {
            CompilerNode::PyObj(obj) => obj.is_instance_of::<pyo3::types::PyBool>(),
            CompilerNode::Pure(ir) => matches!(ir, IR::Bool(_)),
        }
    }

    // ========== Op detection ==========

    /// Check if this is an Op with the given opcode.
    pub fn is_op(&self, _py: Python<'py>, opcode: IROpCode) -> bool {
        match self {
            CompilerNode::PyObj(obj) => obj
                .extract::<PyRef<Op>>()
                .map(|op| op.ident == opcode as i32)
                .unwrap_or(false),
            CompilerNode::Pure(ir) => matches!(ir, IR::Op { opcode: op, .. } if *op == opcode),
        }
    }

    /// Check if this is a SetLocals op.
    #[inline]
    pub fn is_set_locals(&self, py: Python<'py>) -> bool {
        self.is_op(py, IROpCode::SetLocals)
    }

    /// Check if this is a void mutation op (SetItem or SetAttr - pushes nothing).
    #[inline]
    pub fn is_void_op(&self, py: Python<'py>) -> bool {
        self.is_op(py, IROpCode::SetItem) || self.is_op(py, IROpCode::SetAttr)
    }

    /// If this is an OpBlock, return its inner statements. Otherwise None.
    pub fn as_block_contents(&self, py: Python<'py>) -> Option<Vec<CompilerNode<'py>>> {
        match self {
            CompilerNode::PyObj(obj) => {
                let op = obj.extract::<PyRef<Op>>().ok()?;
                if op.ident != IROpCode::OpBlock as i32 {
                    return None;
                }
                let args = op.args.bind(py);
                let len = args.len().ok()?;
                (0..len)
                    .map(|i| args.get_item(i).ok().map(CompilerNode::PyObj))
                    .collect()
            }
            CompilerNode::Pure(ir) => {
                if let IR::Op {
                    opcode: IROpCode::OpBlock,
                    args,
                    ..
                } = ir
                {
                    Some(args.iter().map(CompilerNode::Pure).collect())
                } else {
                    None
                }
            }
        }
    }

    /// For GetAttr Op nodes: extract (object, method_name). Used for method call detection.
    pub fn as_getattr_parts(&self, py: Python<'py>) -> Option<(CompilerNode<'py>, String)> {
        match self {
            CompilerNode::PyObj(obj) => {
                let op = obj.extract::<PyRef<Op>>().ok()?;
                if op.ident != IROpCode::GetAttr as i32 {
                    return None;
                }
                let args = op.args.bind(py);
                if args.len().ok()? < 2 {
                    return None;
                }
                let obj_node = CompilerNode::PyObj(args.get_item(0).ok()?);
                let method_name: String = args.get_item(1).ok()?.extract().ok()?;
                Some((obj_node, method_name))
            }
            CompilerNode::Pure(ir) => {
                if let IR::Op {
                    opcode: IROpCode::GetAttr,
                    args,
                    ..
                } = ir
                {
                    if args.len() < 2 {
                        return None;
                    }
                    let method_name = match &args[1] {
                        IR::String(s) | IR::Identifier(s) | IR::Ref(s, _, _) => s.clone(),
                        _ => return None,
                    };
                    Some((CompilerNode::Pure(&args[0]), method_name))
                } else {
                    None
                }
            }
        }
    }

    /// Check if this is a call to `range()`. Used for for-range optimization.
    pub fn is_range_call(&self, py: Python<'py>) -> bool {
        match self {
            CompilerNode::PyObj(obj) => {
                if let Ok(op) = obj.extract::<PyRef<Op>>() {
                    if op.ident != IROpCode::Call as i32 {
                        return false;
                    }
                    let args = op.args.bind(py);
                    if args.len().unwrap_or(0) < 2 {
                        return false;
                    }
                    if let Ok(func_ref) = args.get_item(0) {
                        if let Ok(type_name) = func_ref.get_type().name() {
                            if type_name.to_str().unwrap_or("") == "Ref" {
                                if let Ok(ident) = func_ref.getattr("ident").and_then(|v| v.extract::<String>()) {
                                    return ident == "range";
                                }
                            }
                        }
                    }
                }
                false
            }
            CompilerNode::Pure(ir) => {
                if let IR::Op {
                    opcode: IROpCode::Call,
                    args,
                    ..
                } = ir
                {
                    if args.len() >= 2 {
                        if let IR::Ref(name, _, _) | IR::Identifier(name) = &args[0] {
                            return name == "range";
                        }
                    }
                }
                if let IR::Call { func, .. } = ir {
                    if let IR::Ref(name, _, _) | IR::Identifier(name) = func.as_ref() {
                        return name == "range";
                    }
                }
                false
            }
        }
    }

    /// Extract range call arguments (everything after the func ref).
    pub fn range_call_args(&self, py: Python<'py>) -> PyResult<Vec<CompilerNode<'py>>> {
        match self {
            CompilerNode::PyObj(obj) => {
                let op: PyRef<Op> = obj.extract()?;
                let args = op.args.bind(py);
                let len = args.len()?;
                (1..len).map(|i| Ok(CompilerNode::PyObj(args.get_item(i)?))).collect()
            }
            CompilerNode::Pure(ir) => match ir {
                IR::Op { args, .. } => Ok(args[1..].iter().map(CompilerNode::Pure).collect()),
                IR::Call { args, .. } => Ok(args.iter().map(CompilerNode::Pure).collect()),
                _ => Err(pyo3::exceptions::PyValueError::new_err("not a range call")),
            },
        }
    }

    /// Try to extract a negated literal: Op(NEG, [int]) -> -int.
    pub fn try_extract_neg_literal(&self, py: Python<'py>) -> Option<i64> {
        match self {
            CompilerNode::PyObj(obj) => {
                let op: PyRef<Op> = obj.extract().ok()?;
                if op.ident != IROpCode::Neg as i32 {
                    return None;
                }
                let args = op.args.bind(py);
                if args.len().ok()? != 1 {
                    return None;
                }
                let val: i64 = args.get_item(0).ok()?.extract().ok()?;
                Some(-val)
            }
            CompilerNode::Pure(ir) => {
                if let IR::Op {
                    opcode: IROpCode::Neg,
                    args,
                    ..
                } = ir
                {
                    if args.len() == 1 {
                        if let IR::Int(n) = &args[0] {
                            return Some(-n);
                        }
                    }
                }
                None
            }
        }
    }

    /// Check if pattern contains star (*rest) or nested patterns.
    /// Used by compile_set_locals to decide between flat unpacking and VM pattern matching.
    pub fn has_complex_pattern(&self, py: Python<'py>) -> bool {
        match self {
            CompilerNode::PyObj(obj) => has_complex_pattern_py(py, obj),
            CompilerNode::Pure(ir) => has_complex_pattern_ir(ir),
        }
    }

    /// Recursively extract flat variable names from a pattern.
    pub fn extract_names(&self, py: Python<'py>) -> Vec<String> {
        let mut names = Vec::new();
        match self {
            CompilerNode::PyObj(obj) => extract_names_recursive_py(py, obj, &mut names),
            CompilerNode::Pure(ir) => extract_names_recursive_ir(ir, &mut names),
        }
        names
    }

    /// Unwrap a single-element tuple wrapper: ((items...),) -> (items...)
    pub fn unwrap_single_tuple(&self, py: Python<'py>) -> PyResult<CompilerNode<'py>> {
        if self.is_tuple() && self.children_len(py)? == 1 {
            let inner = self.child(py, 0)?;
            if inner.is_tuple() {
                return Ok(inner);
            }
        }
        Ok(self.clone())
    }
}

// ========== CompilerKwargs ==========

impl<'py> CompilerKwargs<'py> {
    /// Get a kwarg value by key.
    pub fn get(&self, _py: Python<'py>, key: &str) -> PyResult<Option<CompilerNode<'py>>> {
        match self {
            CompilerKwargs::Py(dict) => Ok(dict.get_item(key)?.map(CompilerNode::PyObj)),
            CompilerKwargs::Pure(map) => Ok(map.get(key).map(CompilerNode::Pure)),
            CompilerKwargs::Empty => Ok(None),
        }
    }

    /// Iterate over all kwargs as (name, value) pairs.
    pub fn iter(&self, _py: Python<'py>) -> PyResult<Vec<(String, CompilerNode<'py>)>> {
        match self {
            CompilerKwargs::Py(dict) => {
                let mut result = Vec::new();
                for (k, v) in dict.iter() {
                    result.push((k.extract::<String>()?, CompilerNode::PyObj(v)));
                }
                Ok(result)
            }
            CompilerKwargs::Pure(map) => Ok(map.iter().map(|(k, v)| (k.clone(), CompilerNode::Pure(v))).collect()),
            CompilerKwargs::Empty => Ok(Vec::new()),
        }
    }

    /// Check if kwargs is empty.
    pub fn is_empty(&self) -> PyResult<bool> {
        match self {
            CompilerKwargs::Py(dict) => Ok(dict.is_empty()),
            CompilerKwargs::Pure(map) => Ok(map.is_empty()),
            CompilerKwargs::Empty => Ok(true),
        }
    }
}

// ========== Pattern helpers (free functions) ==========

/// Check if a PyObject pattern contains star or nested patterns.
fn has_complex_pattern_py(_py: Python<'_>, pattern: &Bound<'_, PyAny>) -> bool {
    // Handle wrapped pattern: ((Ref, Ref, ...),) -> unwrap to (Ref, Ref, ...)
    let actual_pattern = if let Ok(tuple) = pattern.cast::<PyTuple>() {
        if tuple.len() == 1 {
            if let Ok(inner) = tuple.get_item(0) {
                if inner.cast::<PyTuple>().is_ok() {
                    inner
                } else {
                    return false;
                }
            } else {
                return false;
            }
        } else {
            pattern.clone()
        }
    } else {
        return false;
    };

    if let Ok(tuple) = actual_pattern.cast::<PyTuple>() {
        for item in tuple.iter() {
            if let Ok(inner_tuple) = item.cast::<PyTuple>() {
                // Star pattern: ('*', 'name')
                if inner_tuple.len() == 2 {
                    if let Ok(first) = inner_tuple.get_item(0).and_then(|f| f.extract::<String>()) {
                        if first == "*" {
                            return true;
                        }
                    }
                }
                // Nested pattern: tuple of Refs or tuples
                if !inner_tuple.is_empty() {
                    let mut is_nested = true;
                    for nested in inner_tuple.iter() {
                        if let Ok(tn) = nested.get_type().name() {
                            let s = tn.to_str().unwrap_or("");
                            if s != "Ref" && s != "tuple" {
                                is_nested = false;
                                break;
                            }
                        } else {
                            is_nested = false;
                            break;
                        }
                    }
                    if is_nested {
                        return true;
                    }
                }
            } else if item.cast::<PyList>().is_ok() {
                return true;
            }
        }
    }
    false
}

/// Check if an IR pattern contains star or nested patterns.
fn has_complex_pattern_ir(pattern: &IR) -> bool {
    // Unwrap single-element tuple wrapper
    let actual = if let IR::Tuple(items) = pattern {
        if items.len() == 1 {
            if let IR::Tuple(_) = &items[0] {
                &items[0]
            } else {
                return false;
            }
        } else {
            pattern
        }
    } else {
        return false;
    };

    if let IR::Tuple(items) = actual {
        for item in items {
            if let IR::Tuple(pair) = item {
                // Star pattern: ("*", name)
                if pair.len() == 2 {
                    if let IR::String(s) = &pair[0] {
                        if s == "*" {
                            return true;
                        }
                    }
                }
                // Nested pattern: a tuple of Refs/Identifiers/Tuples
                if !pair.is_empty() {
                    let is_nested = pair
                        .iter()
                        .all(|nested| matches!(nested, IR::Ref(_, _, _) | IR::Identifier(_) | IR::Tuple(_)));
                    if is_nested {
                        return true;
                    }
                }
            } else if matches!(item, IR::List(_)) {
                return true;
            }
        }
    }
    false
}

/// Recursively extract variable names from a PyObject pattern.
fn extract_names_recursive_py(py: Python<'_>, pattern: &Bound<'_, PyAny>, names: &mut Vec<String>) {
    if let Ok(tuple) = pattern.cast::<PyTuple>() {
        for item in tuple.iter() {
            if item.cast::<PyTuple>().is_ok() || item.cast::<PyList>().is_ok() {
                extract_names_recursive_py(py, &item, names);
            } else if let Some(name) = extract_single_name_py(py, &item) {
                names.push(name);
            }
        }
        return;
    }
    if let Ok(list) = pattern.cast::<PyList>() {
        for item in list.iter() {
            if item.cast::<PyTuple>().is_ok() || item.cast::<PyList>().is_ok() {
                extract_names_recursive_py(py, &item, names);
            } else if let Some(name) = extract_single_name_py(py, &item) {
                names.push(name);
            }
        }
        return;
    }
    if let Some(name) = extract_single_name_py(py, pattern) {
        names.push(name);
    }
}

/// Extract a single variable name from a PyObject pattern node.
fn extract_single_name_py(_py: Python<'_>, node: &Bound<'_, PyAny>) -> Option<String> {
    if let Ok(s) = node.extract::<String>() {
        return Some(s);
    }
    let type_name = node.get_type().name().ok()?;
    match type_name.to_str().unwrap_or("") {
        "Lvalue" => node.getattr("value").ok()?.extract().ok(),
        "Ref" => node.getattr("ident").ok()?.extract().ok(),
        "Identifier" => node.getattr("name").ok()?.extract().ok(),
        _ => None,
    }
}

/// Recursively extract variable names from an IR pattern.
fn extract_names_recursive_ir(pattern: &IR, names: &mut Vec<String>) {
    match pattern {
        IR::Tuple(items) => {
            for item in items {
                if matches!(item, IR::Tuple(_)) {
                    extract_names_recursive_ir(item, names);
                } else if let Some(name) = ir_to_name(item) {
                    names.push(name);
                }
            }
        }
        _ => {
            if let Some(name) = ir_to_name(pattern) {
                names.push(name);
            }
        }
    }
}

/// Convert an IR node to a variable name.
pub fn ir_to_name(node: &IR) -> Option<String> {
    match node {
        IR::Ref(name, _, _) | IR::Identifier(name) | IR::String(name) => Some(name.clone()),
        _ => None,
    }
}

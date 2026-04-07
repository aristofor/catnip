// FILE: catnip_rs/src/ir/pyclass.rs
//! PyO3 wrapper for IR - exposes IR nodes to Python for inspection.

use super::pure::IR;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Read-only wrapper around IR for Python inspection.
///
/// Returned by `Pipeline.parse_to_ir()` and used by
/// `-p 1/2` display and MCP `parse_catnip`.
#[pyclass(name = "IRNode", frozen)]
pub struct PyIRNode {
    pub(crate) inner: IR,
}

impl PyIRNode {
    pub fn new(inner: IR) -> Self {
        Self { inner }
    }

    /// Extract items from a Program node into individual PyIRNode objects.
    /// Non-Program nodes are returned as a single-element vec.
    pub fn unwrap_program(ir: IR) -> Vec<Self> {
        match ir {
            IR::Program(items) => items.into_iter().map(Self::new).collect(),
            other => vec![Self::new(other)],
        }
    }
}

#[pymethods]
impl PyIRNode {
    /// Variant discriminator: "Op", "Int", "Identifier", "Call", etc.
    #[getter]
    fn kind(&self) -> &str {
        match &self.inner {
            IR::Int(_) => "Int",
            IR::Float(_) => "Float",
            IR::String(_) => "String",
            IR::Bytes(_) => "Bytes",
            IR::Bool(_) => "Bool",
            IR::None => "None",
            IR::Decimal(_) => "Decimal",
            IR::Imaginary(_) => "Imaginary",
            IR::Op { .. } => "Op",
            IR::Identifier(_) => "Identifier",
            IR::Ref(_, _, _) => "Ref",
            IR::List(_) => "List",
            IR::Program(_) => "Program",
            IR::Tuple(_) => "Tuple",
            IR::Dict(_) => "Dict",
            IR::Set(_) => "Set",
            IR::Call { .. } => "Call",
            IR::PatternLiteral(_) => "PatternLiteral",
            IR::PatternVar(_) => "PatternVar",
            IR::PatternWildcard => "PatternWildcard",
            IR::PatternOr(_) => "PatternOr",
            IR::PatternTuple(_) => "PatternTuple",
            IR::PatternStruct { .. } => "PatternStruct",
            IR::PatternEnum { .. } => "PatternEnum",
            IR::Slice { .. } => "Slice",
            IR::Broadcast { .. } => "Broadcast",
        }
    }

    /// Opcode name for Op nodes (e.g. "Add", "SetLocals"), None otherwise.
    #[getter]
    fn opcode(&self) -> Option<String> {
        match &self.inner {
            IR::Op { opcode, .. } => Some(format!("{:?}", opcode)),
            _ => Option::None,
        }
    }

    /// Child nodes as a list.
    /// - Op: args list
    /// - Call: [func] + args
    /// - List/Tuple/Set/Program: items
    /// - PatternLiteral: [inner]
    /// - PatternOr/PatternTuple: items
    /// - Slice: [start, stop, step]
    /// - Others: empty
    #[getter]
    fn args(&self) -> Vec<PyIRNode> {
        match &self.inner {
            IR::Op { args, .. } => args.iter().map(|a| PyIRNode::new(a.clone())).collect(),
            IR::Call { func, args, .. } => {
                let mut result = vec![PyIRNode::new((**func).clone())];
                result.extend(args.iter().map(|a| PyIRNode::new(a.clone())));
                result
            }
            IR::List(items)
            | IR::Program(items)
            | IR::Tuple(items)
            | IR::Set(items)
            | IR::PatternOr(items)
            | IR::PatternTuple(items) => items.iter().map(|a| PyIRNode::new(a.clone())).collect(),
            IR::PatternLiteral(inner) => vec![PyIRNode::new((**inner).clone())],
            IR::Slice { start, stop, step } => vec![
                PyIRNode::new((**start).clone()),
                PyIRNode::new((**stop).clone()),
                PyIRNode::new((**step).clone()),
            ],
            _ => vec![],
        }
    }

    /// Keyword arguments (for Op and Call nodes).
    #[getter]
    fn kwargs(&self, py: Python) -> PyResult<Py<PyDict>> {
        let dict = PyDict::new(py);
        let kw = match &self.inner {
            IR::Op { kwargs, .. } | IR::Call { kwargs, .. } => kwargs,
            _ => return Ok(dict.unbind()),
        };
        for (k, v) in kw {
            let node = Py::new(py, PyIRNode::new(v.clone()))?;
            dict.set_item(k, node)?;
        }
        Ok(dict.unbind())
    }

    /// Tail-call flag (Op nodes only).
    #[getter]
    fn tail(&self) -> bool {
        matches!(&self.inner, IR::Op { tail: true, .. })
    }

    /// Source start position (-1 if unavailable).
    #[getter]
    fn start_byte(&self) -> isize {
        match &self.inner {
            IR::Op { start_byte, .. } | IR::Call { start_byte, .. } => *start_byte as isize,
            IR::Ref(_, sb, _) => *sb,
            _ => -1,
        }
    }

    /// Source end position (-1 if unavailable).
    #[getter]
    fn end_byte(&self) -> isize {
        match &self.inner {
            IR::Op { end_byte, .. } | IR::Call { end_byte, .. } => *end_byte as isize,
            IR::Ref(_, _, eb) => *eb,
            _ => -1,
        }
    }

    /// Literal value as Python object (Int→int, Float→float, etc.).
    /// Returns None for non-literal nodes.
    #[getter]
    fn value(&self, py: Python) -> Py<PyAny> {
        match &self.inner {
            IR::Int(v) => v.into_pyobject(py).unwrap().into_any().unbind(),
            IR::Float(v) => v.into_pyobject(py).unwrap().into_any().unbind(),
            IR::String(v) => v.into_pyobject(py).unwrap().into_any().unbind(),
            IR::Bool(v) => pyo3::types::PyBool::new(py, *v).to_owned().into_any().unbind(),
            IR::Decimal(v) | IR::Imaginary(v) => v.into_pyobject(py).unwrap().into_any().unbind(),
            IR::None => py.None(),
            _ => py.None(),
        }
    }

    /// Name for Identifier, Ref, PatternVar nodes.
    #[getter]
    fn name(&self) -> Option<String> {
        match &self.inner {
            IR::Identifier(n) | IR::Ref(n, _, _) | IR::PatternVar(n) => Some(n.clone()),
            _ => Option::None,
        }
    }

    /// Compact JSON representation.
    fn to_json(&self) -> String {
        self.inner.to_compact_json_pretty()
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            IR::Int(v) => format!("IRNode({})", v),
            IR::Float(v) => format!("IRNode({})", v),
            IR::String(v) => format!("IRNode({:?})", v),
            IR::Bool(v) => format!("IRNode({})", v),
            IR::None => "IRNode(None)".to_string(),
            IR::Identifier(n) => format!("IRNode(id={})", n),
            IR::Ref(n, _, _) => format!("IRNode(ref={})", n),
            IR::Op { opcode, args, .. } => {
                format!("IRNode({:?}, {} args)", opcode, args.len())
            }
            IR::Call { args, .. } => format!("IRNode(Call, {} args)", args.len()),
            IR::Program(items) => format!("IRNode(Program, {} stmts)", items.len()),
            IR::List(items) => format!("IRNode(List, {} items)", items.len()),
            IR::Tuple(items) => format!("IRNode(Tuple, {} items)", items.len()),
            _ => format!("IRNode({})", self.kind()),
        }
    }

    fn __str__(&self) -> String {
        self.inner.to_compact_json_pretty()
    }
}

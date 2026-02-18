// FILE: catnip_rs/src/semantic/function_inlining.rs
//! Function inlining optimization pass.
//!
//! Replaces calls to small, pure functions with their body inlined at the
//! call site. The body is wrapped in a Block with SetLocals bindings for
//! each parameter, ensuring correct scoping.
//!
//! Example:
//!   f = (x) => { x * 2 }; f(5)
//!   → Block(SetLocals("x", 5), Mul(Ref("x"), 2))
//!
//! Subsequent passes (CopyProp, ConstFold, BlockFlat, DCE) clean up the
//! introduced Block and bindings.

use super::opcode::OpCode;
use super::optimizer::{default_visit_ir, default_visit_op, OptimizationPass};
use crate::types::catnip;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::collections::HashMap;
use std::sync::RwLock;

/// Max Op nodes in a function body for it to be inlineable
const MAX_INLINE_OPS: usize = 10;

/// Opcodes that disqualify a function body from inlining
const FORBIDDEN_OPCODES: [OpCode; 8] = [
    OpCode::CALL,
    OpCode::OP_WHILE,
    OpCode::OP_FOR,
    OpCode::OP_LAMBDA,
    OpCode::FN_DEF,
    OpCode::ND_RECURSION,
    OpCode::ND_MAP,
    OpCode::ND_EMPTY_TOPOS,
];

struct InlineCandidate {
    params: Vec<String>,
    body: Py<PyAny>,
}

#[pyclass(name = "FunctionInliningPass")]
pub struct FunctionInliningPass {
    functions: RwLock<HashMap<String, InlineCandidate>>,
}

impl FunctionInliningPass {
    pub fn new() -> Self {
        FunctionInliningPass {
            functions: RwLock::new(HashMap::new()),
        }
    }
}

#[pymethods]
impl FunctionInliningPass {
    #[new]
    fn py_new() -> Self {
        Self::new()
    }

    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        self.functions.write().unwrap().clear();
        OptimizationPass::visit(self, py, node)
    }
}

impl OptimizationPass for FunctionInliningPass {
    fn visit(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        super::optimizer::default_visit(self, py, node)
    }

    fn visit_ir(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        // Visit children first (bottom-up)
        let visited = default_visit_ir(self, py, node)?;
        let visited_bound = visited.bind(py);

        let node_type = visited_bound.get_type();
        let type_name_obj = node_type.name()?;
        let type_name = type_name_obj.to_str()?;
        if type_name != catnip::IR {
            return Ok(visited);
        }

        let opcode_int = match visited_bound.getattr("ident")?.extract::<i32>() {
            Ok(v) => v,
            Err(_) => return Ok(visited),
        };
        let opcode = match OpCode::from_i32(opcode_int) {
            Some(v) => v,
            None => return Ok(visited),
        };

        match opcode {
            OpCode::SET_LOCALS => {
                self.try_record_function(py, &visited_bound)?;
                Ok(visited)
            }
            OpCode::CALL => match self.try_inline_call(py, &visited_bound)? {
                Some(inlined) => Ok(inlined),
                None => Ok(visited),
            },
            _ => Ok(visited),
        }
    }

    fn visit_op(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        default_visit_op(self, py, node)
    }

    fn visit_ref(&self, _py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(node.clone().unbind())
    }
}

impl FunctionInliningPass {
    /// Record a function definition if it qualifies for inlining.
    fn try_record_function(&self, py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<()> {
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.len() < 2 {
            return Ok(());
        }

        // args[0] = function name (string for simple assignment)
        let func_name = match args_tuple.get_item(0)?.extract::<String>() {
            Ok(name) => name,
            Err(_) => return Ok(()),
        };

        // args[1] = value (must be OpLambda)
        let value = args_tuple.get_item(1)?;
        let value_type = value.get_type();
        let value_type_name = value_type.name()?;
        let value_type_str = value_type_name.to_str()?;

        if value_type_str != catnip::IR && value_type_str != catnip::OP {
            return Ok(());
        }

        let value_ident = match value.getattr("ident")?.extract::<i32>() {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        if value_ident != OpCode::OP_LAMBDA as i32 {
            return Ok(());
        }

        // Extract params and body
        let lambda_args = value.getattr("args")?;
        let lambda_tuple = lambda_args.cast::<PyTuple>()?;
        if lambda_tuple.len() < 2 {
            return Ok(());
        }

        let params_obj = lambda_tuple.get_item(0)?;
        let body = lambda_tuple.get_item(1)?;

        let params = match extract_param_names(&params_obj) {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };

        // Skip functions with default parameter values
        if has_defaults(&params_obj)? {
            return Ok(());
        }

        // Body size check
        if count_ops(py, &body)? > MAX_INLINE_OPS {
            return Ok(());
        }

        // No forbidden opcodes (Call, While, For, Lambda, etc.)
        if contains_forbidden_op(py, &body)? {
            return Ok(());
        }

        // No self-reference (would mean recursion)
        if contains_ref_to(py, &body, &func_name)? {
            return Ok(());
        }

        self.functions.write().unwrap().insert(
            func_name,
            InlineCandidate {
                params,
                body: body.clone().unbind(),
            },
        );

        Ok(())
    }

    /// Replace a Call node with the inlined function body.
    fn try_inline_call(
        &self,
        py: Python<'_>,
        node: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        let args = node.getattr("args")?;
        let args_tuple = args.cast::<PyTuple>()?;
        if args_tuple.is_empty() {
            return Ok(None);
        }

        // args[0] must be Ref("func_name")
        let func_ref = args_tuple.get_item(0)?;
        let ref_type = func_ref.get_type();
        let ref_type_name = ref_type.name()?;
        if ref_type_name.to_str()? != catnip::REF {
            return Ok(None);
        }

        let func_name = func_ref.getattr("ident")?.extract::<String>()?;

        // Look up candidate and extract what we need before releasing the lock
        let (params, body) = {
            let functions = self.functions.read().unwrap();
            let candidate = match functions.get(&func_name) {
                Some(c) => c,
                None => return Ok(None),
            };

            // Arity check
            let call_arg_count = args_tuple.len() - 1;
            if call_arg_count != candidate.params.len() {
                return Ok(None);
            }

            (candidate.params.clone(), candidate.body.clone_ref(py))
        };

        // No kwargs at call site
        let kwargs = node.getattr("kwargs")?;
        if kwargs.len()? > 0 {
            return Ok(None);
        }

        // Build: Block(SetLocals(p1, a1), SetLocals(p2, a2), ..., body_clone)
        let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
        let empty_kwargs = PyDict::new(py);

        let mut block_children: Vec<Py<PyAny>> = Vec::new();

        for (i, param_name) in params.iter().enumerate() {
            let arg_value = args_tuple.get_item(i + 1)?;
            let name_py = param_name.as_str().into_pyobject(py)?.into_any();
            let set_args = PyTuple::new(py, vec![name_py.unbind(), arg_value.unbind()])?;
            let set_locals =
                ir_class.call1((OpCode::SET_LOCALS as i32, set_args, &empty_kwargs))?;
            block_children.push(set_locals.unbind());
        }

        let body_clone = clone_tree(py, body.bind(py))?;
        block_children.push(body_clone);

        let block_args = PyTuple::new(py, &block_children)?;
        let block = ir_class.call1((OpCode::OP_BLOCK as i32, block_args, &empty_kwargs))?;

        Ok(Some(block.unbind()))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract parameter names from the params structure.
/// Each param is (name, default) or just a string.
fn extract_param_names(params: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let mut names = Vec::new();
    for item in params.try_iter()? {
        let item = item?;
        if item.is_instance_of::<PyTuple>() {
            let tuple = item.cast::<PyTuple>()?;
            names.push(tuple.get_item(0)?.extract::<String>()?);
        } else {
            names.push(item.extract::<String>()?);
        }
    }
    Ok(names)
}

/// Check if any parameter has a non-None default value.
fn has_defaults(params: &Bound<'_, PyAny>) -> PyResult<bool> {
    for item in params.try_iter()? {
        let item = item?;
        if item.is_instance_of::<PyTuple>() {
            let tuple = item.cast::<PyTuple>()?;
            if tuple.len() >= 2 && !tuple.get_item(1)?.is_none() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Count Op/IR nodes recursively.
fn count_ops(py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<usize> {
    let type_name = node.get_type().name()?;
    match type_name.to_str()? {
        "IR" | "Op" => {
            let mut count = 1;
            for arg in node.getattr("args")?.try_iter()? {
                count += count_ops(py, &arg?)?;
            }
            Ok(count)
        }
        _ => Ok(0),
    }
}

/// Check if any forbidden opcode appears in the tree.
fn contains_forbidden_op(py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<bool> {
    let type_name = node.get_type().name()?;
    match type_name.to_str()? {
        "IR" | "Op" => {
            if let Ok(ident) = node.getattr("ident")?.extract::<i32>() {
                if let Some(opcode) = OpCode::from_i32(ident) {
                    if FORBIDDEN_OPCODES.contains(&opcode) {
                        return Ok(true);
                    }
                }
            }
            for arg in node.getattr("args")?.try_iter()? {
                if contains_forbidden_op(py, &arg?)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Check if Ref("name") appears anywhere in the tree.
fn contains_ref_to(py: Python<'_>, node: &Bound<'_, PyAny>, name: &str) -> PyResult<bool> {
    let type_name = node.get_type().name()?;
    match type_name.to_str()? {
        "Ref" => {
            let ident = node.getattr("ident")?.extract::<String>()?;
            Ok(ident == name)
        }
        "IR" | "Op" => {
            for arg in node.getattr("args")?.try_iter()? {
                if contains_ref_to(py, &arg?, name)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// Deep clone an Op/IR tree to avoid shared mutable state.
fn clone_tree(py: Python<'_>, node: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let type_name = node.get_type().name()?;
    match type_name.to_str()? {
        "IR" => {
            let ident = node.getattr("ident")?;
            let mut cloned_args = Vec::new();
            for arg in node.getattr("args")?.try_iter()? {
                cloned_args.push(clone_tree(py, &arg?)?);
            }
            let args_tuple = PyTuple::new(py, &cloned_args)?;

            let cloned_kwargs = PyDict::new(py);
            if let Ok(items) = node.getattr("kwargs")?.call_method0("items") {
                for item in items.try_iter()? {
                    let item = item?;
                    let kv = item.cast::<PyTuple>()?;
                    cloned_kwargs.set_item(kv.get_item(0)?, clone_tree(py, &kv.get_item(1)?)?)?;
                }
            }

            let ir_class = py.import("catnip.transformer")?.getattr("IR")?;
            Ok(ir_class.call1((ident, args_tuple, cloned_kwargs))?.unbind())
        }
        "Op" => {
            let ident = node.getattr("ident")?;
            let mut cloned_args = Vec::new();
            for arg in node.getattr("args")?.try_iter()? {
                cloned_args.push(clone_tree(py, &arg?)?);
            }
            let args_tuple = PyTuple::new(py, &cloned_args)?;

            let cloned_kwargs = PyDict::new(py);
            if let Ok(items) = node.getattr("kwargs")?.call_method0("items") {
                for item in items.try_iter()? {
                    let item = item?;
                    let kv = item.cast::<PyTuple>()?;
                    cloned_kwargs.set_item(kv.get_item(0)?, clone_tree(py, &kv.get_item(1)?)?)?;
                }
            }

            let op_class = py.import("catnip.nodes")?.getattr("Op")?;
            Ok(op_class.call1((ident, args_tuple, cloned_kwargs))?.unbind())
        }
        _ => {
            // Ref, literals: immutable, safe to share
            Ok(node.clone().unbind())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pass() {
        let pass = FunctionInliningPass::new();
        assert_eq!(pass.functions.read().unwrap().len(), 0);
    }

    #[test]
    fn test_forbidden_opcodes() {
        assert!(FORBIDDEN_OPCODES.contains(&OpCode::CALL));
        assert!(FORBIDDEN_OPCODES.contains(&OpCode::OP_WHILE));
        assert!(FORBIDDEN_OPCODES.contains(&OpCode::OP_FOR));
        assert!(FORBIDDEN_OPCODES.contains(&OpCode::OP_LAMBDA));
        assert!(FORBIDDEN_OPCODES.contains(&OpCode::FN_DEF));
        assert!(!FORBIDDEN_OPCODES.contains(&OpCode::ADD));
        assert!(!FORBIDDEN_OPCODES.contains(&OpCode::MUL));
        assert!(!FORBIDDEN_OPCODES.contains(&OpCode::OP_IF));
    }

    #[test]
    fn test_max_inline_ops_threshold() {
        // Sanity: threshold is reasonable
        assert!(MAX_INLINE_OPS > 0);
        assert!(MAX_INLINE_OPS <= 50);
    }
}

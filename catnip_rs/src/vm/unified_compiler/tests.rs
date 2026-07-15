//! Tests for the unified compiler.

use super::*;
use crate::ir::{IR, IROpCode};
use crate::vm::frame::PyCodeObject;

#[test]
fn test_freeze_ir_body_pure() {
    let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
    let node = CompilerNode::Pure(&body);
    // Element 1 carries the params (with type annotations) for the ND worker.
    let params = IR::Tuple(vec![]);
    let params_node = CompilerNode::Pure(&params);
    let frozen = freeze_ir_body(&node, &params_node);
    assert!(frozen.is_some(), "freeze_ir_body should return Some for Pure IR");

    // Verify the frozen bytes can be decoded back (raw bincode, no header):
    // element 0 is the body, element 1 the params.
    let bytes = frozen.unwrap();
    let decoded: Vec<IR> = catnip_core::freeze::decode(&bytes).unwrap();
    assert_eq!(decoded.len(), 2);
}

#[test]
fn test_compile_lambda_has_encoded_ir() {
    Python::attach(|py| {
        // Build a lambda IR: (n) => { n * 2 }
        let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
        let params = IR::List(vec![IR::Identifier("n".into())]);
        let lambda_ir = IR::op(IROpCode::OpLambda, vec![params, body]);

        // Compile via the full pipeline
        let program = IR::Program(vec![lambda_ir]);
        let mut compiler = UnifiedCompiler::new();
        let code = compiler.compile_pure(py, &program).unwrap();

        // The top-level code should have a constant that is a PyCodeObject
        // with encoded_ir set
        let mut found_encoded_ir = false;
        for c in &code.constants {
            if c.is_pyobj() {
                let obj = c.as_pyobject(py).unwrap();
                let bound = obj.bind(py);
                if let Ok(py_code) = bound.cast::<PyCodeObject>() {
                    if py_code.borrow().inner.encoded_ir.is_some() {
                        found_encoded_ir = true;
                    }
                }
            }
        }
        assert!(
            found_encoded_ir,
            "compiled lambda should have encoded_ir in its CodeObject"
        );
    });
}

// ===== @pure static marking on the PyObj path (volet 1, JIT inlining) =====

/// Build a Python `Ref` node.
fn ref_node<'py>(py: Python<'py>, name: &str) -> Bound<'py, PyAny> {
    Py::new(
        py,
        crate::core::Ref {
            ident: name.to_string(),
            start_byte: -1,
            end_byte: -1,
        },
    )
    .unwrap()
    .into_bound(py)
    .into_any()
}

/// Build a Python `Op` node with the given opcode and arg children.
fn op_node<'py>(py: Python<'py>, opcode: IROpCode, args: Vec<Bound<'py, PyAny>>) -> Bound<'py, PyAny> {
    let args_tuple = PyTuple::new(py, args).unwrap();
    let kwargs = PyDict::new(py);
    Py::new(
        py,
        Op::from_rust(
            py,
            opcode as i32,
            args_tuple.into_any().unbind(),
            kwargs.into_any().unbind(),
            false,
            -1,
            -1,
        ),
    )
    .unwrap()
    .into_bound(py)
    .into_any()
}

/// Build `<decorator>((x) => x * x)` as a Python Call Op node.
fn decorated_lambda<'py>(py: Python<'py>, decorator: &str) -> Bound<'py, PyAny> {
    let body = op_node(py, IROpCode::Mul, vec![ref_node(py, "x"), ref_node(py, "x")]);
    let params = PyTuple::new(py, vec![ref_node(py, "x")]).unwrap().into_any();
    let lambda = op_node(py, IROpCode::OpLambda, vec![params, body]);
    op_node(py, IROpCode::Call, vec![ref_node(py, decorator), lambda])
}

/// Build a plain `(x) => x * x` OpLambda node (no decorator).
fn plain_lambda<'py>(py: Python<'py>) -> Bound<'py, PyAny> {
    let body = op_node(py, IROpCode::Mul, vec![ref_node(py, "x"), ref_node(py, "x")]);
    let params = PyTuple::new(py, vec![ref_node(py, "x")]).unwrap().into_any();
    op_node(py, IROpCode::OpLambda, vec![params, body])
}

#[test]
fn test_is_pure_decorated_lambda_pyobj() {
    Python::attach(|py| {
        // pure(lambda) is the shape @pure lowers to.
        assert!(CompilerNode::PyObj(decorated_lambda(py, "pure")).is_pure_decorated_lambda(py));
        // Any other decorator must not match.
        assert!(!CompilerNode::PyObj(decorated_lambda(py, "jit")).is_pure_decorated_lambda(py));
        // A bare lambda is not a Call, so not matched.
        assert!(!CompilerNode::PyObj(plain_lambda(py)).is_pure_decorated_lambda(py));
        // pure(<non-lambda>) is not matched.
        let not_lambda = op_node(py, IROpCode::Call, vec![ref_node(py, "pure"), ref_node(py, "y")]);
        assert!(!CompilerNode::PyObj(not_lambda).is_pure_decorated_lambda(py));
    });
}

/// Scan a compiled CodeObject's constants for a lambda CodeObject and report
/// whether it is marked pure (None when no nested CodeObject is found).
fn nested_code_is_pure(py: Python<'_>, code: &CodeObject) -> Option<bool> {
    for c in &code.constants {
        if c.is_pyobj() {
            let obj = c.as_pyobject(py).unwrap();
            if let Ok(py_code) = obj.bind(py).cast::<PyCodeObject>() {
                return Some(py_code.borrow().inner.is_pure);
            }
        }
    }
    None
}

#[test]
fn test_pure_decorator_marks_codeobject_pyobj() {
    Python::attach(|py| {
        // square = pure((x) => x * x)
        let names = PyTuple::new(py, vec![ref_node(py, "square")]).unwrap().into_any();
        let set_locals = op_node(py, IROpCode::SetLocals, vec![names, decorated_lambda(py, "pure")]);

        let mut compiler = UnifiedCompiler::new();
        let code = compiler.compile_py(py, &set_locals).unwrap();
        assert_eq!(
            nested_code_is_pure(py, &code),
            Some(true),
            "@pure should mark the lambda's CodeObject pure via compile_py"
        );
    });
}

#[test]
fn test_plain_lambda_not_pure_pyobj() {
    Python::attach(|py| {
        // square = (x) => x * x  (no @pure)
        let names = PyTuple::new(py, vec![ref_node(py, "square")]).unwrap().into_any();
        let set_locals = op_node(py, IROpCode::SetLocals, vec![names, plain_lambda(py)]);

        let mut compiler = UnifiedCompiler::new();
        let code = compiler.compile_py(py, &set_locals).unwrap();
        assert_eq!(
            nested_code_is_pure(py, &code),
            Some(false),
            "plain lambda must not be pure"
        );
    });
}

// FILE: catnip_rs/src/cfg/tests.rs
//! Tests for CFG construction and analysis.
//!
//! These tests validate CFG building from IR, dominance analysis,
//! loop detection, and other CFG-based analysis without requiring
//! the full Python parser.

use super::builder_ir::IRCFGBuilder;
use crate::core::op::Op;
use crate::ir::IROpCode;
use pyo3::conversion::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

/// Helper to create a simple Op node in Rust tests.
fn create_op(py: Python<'_>, opcode: IROpCode, args: Vec<Py<PyAny>>) -> Op {
    let args_tuple = PyTuple::new(py, &args).unwrap().unbind();
    let kwargs = PyDict::new(py).unbind();

    Op {
        ident: opcode as i32,
        args: args_tuple.into(),
        kwargs: kwargs.into(),
        tail: false,
        start_byte: -1,
        end_byte: -1,
    }
}

/// Helper to create a literal value (Python int/str/bool/None).
#[allow(deprecated)]
fn literal<'py, T>(py: Python<'py>, value: T) -> Py<PyAny>
where
    T: IntoPyObjectExt<'py>,
{
    value.into_py_any(py).unwrap()
}

/// Helper to create an assignment node.
/// Note: Using SetLocals which is the IR opcode for assignment (x = value).
fn assign_node(py: Python<'_>, name: &str, value: Py<PyAny>) -> Op {
    let names_tuple = PyTuple::new(py, &[name]).unwrap();
    let values_tuple = PyTuple::new(py, &[value]).unwrap();
    create_op(
        py,
        IROpCode::SetLocals,
        vec![names_tuple.unbind().into(), values_tuple.unbind().into()],
    )
}

#[test]
fn test_cfg_simple_assignment() {
    // x = 1 → entry -> exit (2 blocks, 1 edge)
    Python::initialize();
    Python::attach(|py| {
        let one = literal(py, 1);
        let assign = assign_node(py, "x", one);

        let builder = IRCFGBuilder::new("test");
        let cfg = builder.build(vec![assign]);

        assert_eq!(cfg.blocks.len(), 2, "Should have 2 blocks (entry, exit)");
        assert_eq!(cfg.edges.len(), 1, "Should have 1 edge (entry -> exit)");
        assert!(cfg.entry.is_some(), "Entry block should be set");
        assert!(cfg.exit.is_some(), "Exit block should be set");
    });
}

#[test]
fn test_cfg_linear_sequence() {
    // x = 1; y = 2; z = 3 → simple linear flow
    Python::initialize();
    Python::attach(|py| {
        let assign_x = assign_node(py, "x", literal(py, 1));
        let assign_y = assign_node(py, "y", literal(py, 2));
        let assign_z = assign_node(py, "z", literal(py, 3));

        let builder = IRCFGBuilder::new("linear");
        let cfg = builder.build(vec![assign_x, assign_y, assign_z]);

        assert_eq!(cfg.blocks.len(), 2, "Linear sequence: 2 blocks");
        assert_eq!(cfg.edges.len(), 1, "Linear sequence: 1 edge");

        // Check reachability
        let reachable = cfg.get_reachable_blocks();
        assert_eq!(reachable.len(), 2, "All blocks should be reachable");
    });
}

// Removed: test_cfg_dominance_linear — proven by entry_dom_all + dom_trans (CatnipDominanceProof.v:104,142)

#[test]
fn test_cfg_unreachable_blocks() {
    // Simple case - all blocks should be reachable
    Python::initialize();
    Python::attach(|py| {
        let assign = assign_node(py, "x", literal(py, 1));

        let builder = IRCFGBuilder::new("test");
        let cfg = builder.build(vec![assign]);

        let reachable = cfg.get_reachable_blocks();
        let unreachable = cfg.get_unreachable_blocks();

        assert_eq!(unreachable.len(), 0, "No unreachable blocks in simple code");
        assert_eq!(
            reachable.len(),
            cfg.blocks.len(),
            "All blocks should be reachable"
        );
    });
}

#[test]
fn test_cfg_to_dot_basic() {
    // Test DOT generation
    Python::initialize();
    Python::attach(|py| {
        let assign = assign_node(py, "x", literal(py, 1));

        let builder = IRCFGBuilder::new("test");
        let cfg = builder.build(vec![assign]);

        let dot = cfg.to_dot();

        assert!(
            dot.contains("digraph"),
            "DOT should have digraph declaration"
        );
        assert!(dot.contains("test"), "DOT should include CFG name");
        assert!(dot.contains("entry"), "DOT should show entry block");
        assert!(dot.contains("exit"), "DOT should show exit block");
    });
}

#[test]
fn test_cfg_display() {
    // Test Display trait (to_string)
    Python::initialize();
    Python::attach(|py| {
        let assign = assign_node(py, "x", literal(py, 1));

        let builder = IRCFGBuilder::new("test");
        let cfg = builder.build(vec![assign]);

        let str_repr = cfg.to_string();
        assert!(str_repr.contains("CFG"), "Display shows CFG");
        assert!(str_repr.contains("test"), "Display includes name");
        assert!(str_repr.contains("Entry"), "Display shows entry");
        assert!(str_repr.contains("Exit"), "Display shows exit");
        assert!(str_repr.contains("Blocks"), "Display shows blocks count");
    });
}

// Note: Complex control flow tests (if/while/for) are difficult to create
// without the full parser because they require complex nested tuple structures.
// These are better tested via the Python integration tests in tests/cfg/test_cfg_rust.py

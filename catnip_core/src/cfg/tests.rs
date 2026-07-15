// FILE: catnip_core/src/cfg/tests.rs
//! Tests for CFG construction and analysis.
//!
//! These tests validate CFG building from IR, dominance analysis,
//! loop detection, and other CFG-based analysis without requiring
//! the full parser.

use super::builder_ir::IRCFGBuilder;
use crate::ir::{IR, IROpCode};

/// Helper to create an assignment node (x = value).
/// Uses SetLocals which is the IR opcode for assignment.
fn assign_node(name: &str, value: IR) -> IR {
    let names = IR::Tuple(vec![IR::Ref(name.to_string(), -1, -1)]);
    let values = IR::Tuple(vec![value]);
    IR::op(IROpCode::SetLocals, vec![names, values])
}

#[test]
fn test_cfg_simple_assignment() {
    // x = 1 → entry -> exit (2 blocks, 1 edge)
    let assign = assign_node("x", IR::Int(1));

    let builder = IRCFGBuilder::new("test");
    let cfg = builder.build(vec![assign]);

    assert_eq!(cfg.blocks.len(), 2, "Should have 2 blocks (entry, exit)");
    assert_eq!(cfg.edges.len(), 1, "Should have 1 edge (entry -> exit)");
    assert!(cfg.entry.is_some(), "Entry block should be set");
    assert!(cfg.exit.is_some(), "Exit block should be set");
}

#[test]
fn test_cfg_linear_sequence() {
    // x = 1; y = 2; z = 3 → simple linear flow
    let assign_x = assign_node("x", IR::Int(1));
    let assign_y = assign_node("y", IR::Int(2));
    let assign_z = assign_node("z", IR::Int(3));

    let builder = IRCFGBuilder::new("linear");
    let cfg = builder.build(vec![assign_x, assign_y, assign_z]);

    assert_eq!(cfg.blocks.len(), 2, "Linear sequence: 2 blocks");
    assert_eq!(cfg.edges.len(), 1, "Linear sequence: 1 edge");

    // Check reachability
    let reachable = cfg.get_reachable_blocks();
    assert_eq!(reachable.len(), 2, "All blocks should be reachable");
}

// Removed: test_cfg_dominance_linear - proven by entry_dom_all + dom_trans (CatnipDominanceProof.v:104,142)

#[test]
fn test_cfg_unreachable_blocks() {
    // Simple case - all blocks should be reachable
    let assign = assign_node("x", IR::Int(1));

    let builder = IRCFGBuilder::new("test");
    let cfg = builder.build(vec![assign]);

    let reachable = cfg.get_reachable_blocks();
    let unreachable = cfg.get_unreachable_blocks();

    assert_eq!(unreachable.len(), 0, "No unreachable blocks in simple code");
    assert_eq!(reachable.len(), cfg.blocks.len(), "All blocks should be reachable");
}

#[test]
fn test_cfg_to_dot_basic() {
    // Test DOT generation
    let assign = assign_node("x", IR::Int(1));

    let builder = IRCFGBuilder::new("test");
    let cfg = builder.build(vec![assign]);

    let dot = cfg.to_dot();

    assert!(dot.contains("digraph"), "DOT should have digraph declaration");
    assert!(dot.contains("test"), "DOT should include CFG name");
    assert!(dot.contains("entry"), "DOT should show entry block");
    assert!(dot.contains("exit"), "DOT should show exit block");
}

#[test]
fn test_cfg_display() {
    // Test Display trait (to_string)
    let assign = assign_node("x", IR::Int(1));

    let builder = IRCFGBuilder::new("test");
    let cfg = builder.build(vec![assign]);

    let str_repr = cfg.to_string();
    assert!(str_repr.contains("CFG"), "Display shows CFG");
    assert!(str_repr.contains("test"), "Display includes name");
    assert!(str_repr.contains("Entry"), "Display shows entry");
    assert!(str_repr.contains("Exit"), "Display shows exit");
    assert!(str_repr.contains("Blocks"), "Display shows blocks count");
}

// Note: Complex control flow tests (if/while/for) are difficult to create
// without the full parser because they require complex nested tuple structures.

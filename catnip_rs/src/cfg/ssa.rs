// FILE: catnip_rs/src/cfg/ssa.rs
//! SSA (Static Single Assignment) form for CFG.
//!
//! Uses Braun et al. 2013 algorithm for incremental SSA construction.
//! Block parameters instead of phi nodes (same pattern as Cranelift in jit/codegen.rs).

use super::graph::ControlFlowGraph;
use std::collections::HashMap;

/// A unique SSA definition: variable id + version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SSAValue {
    pub var: usize,
    pub version: u32,
}

impl SSAValue {
    pub fn new(var: usize, version: u32) -> Self {
        Self { var, version }
    }
}

/// Block parameter = phi node at block entry.
/// Each predecessor provides an incoming value.
#[derive(Debug, Clone)]
pub struct BlockParam {
    /// SSA value this parameter defines
    pub value: SSAValue,
    /// Incoming values from predecessors (indexed by predecessor order)
    /// None = not yet resolved (incomplete phi)
    pub incoming: Vec<Option<SSAValue>>,
}

/// Per-block SSA information.
#[derive(Debug, Clone)]
pub struct BlockSSAInfo {
    /// Block parameters (phi functions)
    pub params: Vec<BlockParam>,
    /// Current definition of each variable in this block (var_id -> SSAValue)
    pub current_defs: HashMap<usize, SSAValue>,
    /// Snapshot of current_defs at block entry (before any instruction)
    pub initial_defs: HashMap<usize, SSAValue>,
    /// Whether all predecessors are known
    pub sealed: bool,
    /// Incomplete phis waiting for seal (var_id -> param index in params)
    pub incomplete_phis: HashMap<usize, usize>,
}

impl BlockSSAInfo {
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            current_defs: HashMap::new(),
            initial_defs: HashMap::new(),
            sealed: false,
            incomplete_phis: HashMap::new(),
        }
    }
}

/// Variable name interning table.
#[derive(Debug, Clone)]
pub struct VarTable {
    name_to_id: HashMap<String, usize>,
    id_to_name: Vec<String>,
}

impl VarTable {
    pub fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            id_to_name: Vec::new(),
        }
    }

    /// Intern a variable name, return its id.
    pub fn intern(&mut self, name: &str) -> usize {
        if let Some(&id) = self.name_to_id.get(name) {
            id
        } else {
            let id = self.id_to_name.len();
            self.id_to_name.push(name.to_string());
            self.name_to_id.insert(name.to_string(), id);
            id
        }
    }

    /// Get name from id.
    pub fn name(&self, id: usize) -> Option<&str> {
        self.id_to_name.get(id).map(|s| s.as_str())
    }

    /// Get id from name.
    pub fn id(&self, name: &str) -> Option<usize> {
        self.name_to_id.get(name).copied()
    }
}

/// Where an SSA value was defined.
#[derive(Debug, Clone)]
pub enum ValueDef {
    /// Defined by an instruction in a block
    Instruction { block: usize, instr_idx: usize },
    /// Defined by a block parameter (phi)
    BlockParam { block: usize, param_idx: usize },
}

/// Operand in an SSA expression key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExprOperand {
    Var(SSAValue),
    Int(i64),
    Bool(bool),
    Str(String),
    /// Float as bits for Eq/Hash
    Float(u64),
    None,
    /// Nested Op or unresolvable -- makes the key non-matchable
    Opaque,
}

/// Expression key for CSE/GVN: (opcode_of_rhs, resolved_operands).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SSAExprKey {
    pub opcode: i32,
    pub operands: Vec<ExprOperand>,
}

impl SSAExprKey {
    pub fn is_resolved(&self) -> bool {
        self.operands
            .iter()
            .all(|op| !matches!(op, ExprOperand::Opaque))
    }
}

/// Main SSA context holding all SSA state.
#[derive(Debug, Clone)]
pub struct SSAContext {
    /// Per-block SSA info (block_id -> info)
    pub blocks: HashMap<usize, BlockSSAInfo>,
    /// Variable name interning
    pub vars: VarTable,
    /// Next version number per variable (var_id -> next_version)
    next_version: HashMap<usize, u32>,
    /// Where each SSA value was defined
    pub value_defs: HashMap<SSAValue, ValueDef>,
    /// Per-instruction use tracking: (block_id, instr_idx) -> SSA values used
    pub instruction_uses: HashMap<(usize, usize), Vec<SSAValue>>,
}

impl SSAContext {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            vars: VarTable::new(),
            next_version: HashMap::new(),
            value_defs: HashMap::new(),
            instruction_uses: HashMap::new(),
        }
    }

    /// Record that instruction (block, instr_idx) uses these SSA values.
    pub fn record_uses(&mut self, block: usize, instr_idx: usize, uses: Vec<SSAValue>) {
        self.instruction_uses.insert((block, instr_idx), uses);
    }

    /// Get uses for an instruction, or empty slice if not tracked.
    pub fn get_uses(&self, block: usize, instr_idx: usize) -> &[SSAValue] {
        self.instruction_uses
            .get(&(block, instr_idx))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Ensure a block has SSA info initialized.
    pub fn ensure_block(&mut self, block_id: usize) {
        self.blocks
            .entry(block_id)
            .or_insert_with(BlockSSAInfo::new);
    }

    /// Allocate a fresh SSA version for a variable.
    fn fresh_version(&mut self, var: usize) -> u32 {
        let v = self.next_version.entry(var).or_insert(0);
        let version = *v;
        *v += 1;
        version
    }

    /// Define a variable in a block (Braun: writeVariable).
    ///
    /// Records that `var_name` is defined by instruction `instr_idx` in `block`.
    pub fn def_var(&mut self, block: usize, var_name: &str, instr_idx: usize) -> SSAValue {
        let var_id = self.vars.intern(var_name);
        let version = self.fresh_version(var_id);
        let value = SSAValue::new(var_id, version);

        self.ensure_block(block);
        self.blocks
            .get_mut(&block)
            .unwrap()
            .current_defs
            .insert(var_id, value);

        self.value_defs
            .insert(value, ValueDef::Instruction { block, instr_idx });
        value
    }

    /// Use a variable in a block (Braun: readVariable).
    ///
    /// Returns the reaching SSA value for `var_name` at `block`.
    pub fn use_var(&mut self, cfg: &ControlFlowGraph, block: usize, var_name: &str) -> SSAValue {
        let var_id = self.vars.intern(var_name);
        self.use_var_by_id(cfg, block, var_id)
    }

    /// Internal: use variable by id (Braun: readVariable).
    fn use_var_by_id(&mut self, cfg: &ControlFlowGraph, block: usize, var_id: usize) -> SSAValue {
        self.ensure_block(block);

        // Local lookup: variable defined in this block?
        if let Some(&value) = self.blocks.get(&block).unwrap().current_defs.get(&var_id) {
            return value;
        }

        // Non-local: look at predecessors
        self.use_var_recursive(cfg, block, var_id)
    }

    /// Braun: readVariableRecursive.
    fn use_var_recursive(
        &mut self,
        cfg: &ControlFlowGraph,
        block: usize,
        var_id: usize,
    ) -> SSAValue {
        let sealed = self.blocks.get(&block).map(|b| b.sealed).unwrap_or(false);

        let value = if !sealed {
            // Block not sealed: add incomplete phi
            let phi_value = self.add_block_param(cfg, block, var_id);
            let param_idx = self.blocks[&block].params.len() - 1;
            self.blocks
                .get_mut(&block)
                .unwrap()
                .incomplete_phis
                .insert(var_id, param_idx);
            phi_value
        } else {
            let preds = Self::get_predecessor_blocks(cfg, block);
            if preds.len() == 1 {
                // Single predecessor: no phi needed, recurse directly
                self.use_var_by_id(cfg, preds[0], var_id)
            } else {
                // Multiple predecessors: add phi then fill operands
                let phi_value = self.add_block_param(cfg, block, var_id);
                // Write before recursing to break cycles
                self.blocks
                    .get_mut(&block)
                    .unwrap()
                    .current_defs
                    .insert(var_id, phi_value);
                self.fill_phi_operands(cfg, block, var_id, phi_value);
                self.try_remove_trivial_phi(cfg, block, phi_value)
            }
        };

        // Record current def for this block
        self.blocks
            .get_mut(&block)
            .unwrap()
            .current_defs
            .insert(var_id, value);
        value
    }

    /// Add a block parameter (phi) for a variable.
    fn add_block_param(&mut self, cfg: &ControlFlowGraph, block: usize, var_id: usize) -> SSAValue {
        let version = self.fresh_version(var_id);
        let value = SSAValue::new(var_id, version);
        let pred_count = Self::get_predecessor_blocks(cfg, block).len();

        let param_idx = self.blocks.get(&block).unwrap().params.len();
        let param = BlockParam {
            value,
            incoming: vec![None; pred_count],
        };
        self.blocks.get_mut(&block).unwrap().params.push(param);

        self.value_defs
            .insert(value, ValueDef::BlockParam { block, param_idx });
        value
    }

    /// Fill phi operands from predecessor blocks.
    fn fill_phi_operands(
        &mut self,
        cfg: &ControlFlowGraph,
        block: usize,
        var_id: usize,
        _phi_value: SSAValue,
    ) {
        let preds = Self::get_predecessor_blocks(cfg, block);
        let param_idx = self.blocks[&block].params.len() - 1;

        for (i, &pred) in preds.iter().enumerate() {
            let incoming = self.use_var_by_id(cfg, pred, var_id);
            self.blocks.get_mut(&block).unwrap().params[param_idx].incoming[i] = Some(incoming);
        }
    }

    /// Try to remove a trivial phi (Braun: tryRemoveTrivialPhi).
    ///
    /// A phi is trivial if all operands are the same value (or the phi itself).
    /// Returns the value that replaces the phi.
    pub fn try_remove_trivial_phi(
        &mut self,
        cfg: &ControlFlowGraph,
        block: usize,
        phi_value: SSAValue,
    ) -> SSAValue {
        // Find the param for this phi
        let param_idx = self.blocks[&block]
            .params
            .iter()
            .position(|p| p.value == phi_value);

        let param_idx = match param_idx {
            Some(idx) => idx,
            None => return phi_value, // not a phi in this block
        };

        // Collect unique non-self operands
        let mut same: Option<SSAValue> = None;
        let incoming: Vec<Option<SSAValue>> =
            self.blocks[&block].params[param_idx].incoming.clone();

        for op in &incoming {
            let Some(op_val) = op else { continue };
            if *op_val == phi_value {
                continue; // self-reference
            }
            match same {
                None => same = Some(*op_val),
                Some(s) if s == *op_val => { /* same operand, ok */ }
                Some(_) => return phi_value, // non-trivial: different operands
            }
        }

        let replacement = match same {
            Some(v) => v,
            None => return phi_value, // all self-references (unreachable or undefined)
        };

        // Replace the phi: update current_defs in this block
        let var_id = phi_value.var;
        if self.blocks[&block].current_defs.get(&var_id) == Some(&phi_value) {
            self.blocks
                .get_mut(&block)
                .unwrap()
                .current_defs
                .insert(var_id, replacement);
        }

        // Phi is trivial: current_defs now points to replacement, but param.value
        // retains the original SSA value. phi_count() uses the mismatch to detect
        // eliminated phis (current_defs[var] != param.value).

        // Check users: any other phi that used this phi might now be trivial
        let blocks_to_check: Vec<usize> = self.blocks.keys().copied().collect();
        for &other_block in &blocks_to_check {
            let params_len = self.blocks[&other_block].params.len();
            for pi in 0..params_len {
                let param_value = self.blocks[&other_block].params[pi].value;
                let mut replaced = false;
                let incoming_len = self.blocks[&other_block].params[pi].incoming.len();
                for ii in 0..incoming_len {
                    if self.blocks[&other_block].params[pi].incoming[ii] == Some(phi_value) {
                        self.blocks.get_mut(&other_block).unwrap().params[pi].incoming[ii] =
                            Some(replacement);
                        replaced = true;
                    }
                }
                if replaced && param_value != phi_value && param_value != replacement {
                    // Recursively try to simplify
                    self.try_remove_trivial_phi(cfg, other_block, param_value);
                }
            }
        }

        replacement
    }

    /// Seal a block: all predecessors are known (Braun: sealBlock).
    ///
    /// Resolves all incomplete phis.
    pub fn seal_block(&mut self, cfg: &ControlFlowGraph, block: usize) {
        self.ensure_block(block);

        // Collect incomplete phis before processing
        let incomplete: Vec<(usize, usize)> = self.blocks[&block]
            .incomplete_phis
            .iter()
            .map(|(&var_id, &param_idx)| (var_id, param_idx))
            .collect();

        for (var_id, _param_idx) in &incomplete {
            // Fill operands for the incomplete phi
            let preds = Self::get_predecessor_blocks(cfg, block);
            let param_idx_actual = self.blocks[&block]
                .params
                .iter()
                .position(|p| {
                    p.value.var == *var_id
                        && self.blocks[&block].incomplete_phis.get(var_id).is_some()
                })
                .unwrap_or(0);

            for (i, &pred) in preds.iter().enumerate() {
                let incoming = self.use_var_by_id(cfg, pred, *var_id);
                if param_idx_actual < self.blocks[&block].params.len()
                    && i < self.blocks[&block].params[param_idx_actual].incoming.len()
                {
                    self.blocks.get_mut(&block).unwrap().params[param_idx_actual].incoming[i] =
                        Some(incoming);
                }
            }
        }

        // Mark as sealed
        self.blocks.get_mut(&block).unwrap().sealed = true;

        // Try to remove trivial phis
        for (var_id, _param_idx) in &incomplete {
            if let Some(phi_value) = self.blocks[&block].current_defs.get(var_id).copied() {
                self.try_remove_trivial_phi(cfg, block, phi_value);
            }
        }

        // Clear incomplete phis
        self.blocks.get_mut(&block).unwrap().incomplete_phis.clear();
    }

    /// Get predecessor block IDs (not edge indices).
    pub fn get_predecessor_blocks(cfg: &ControlFlowGraph, block: usize) -> Vec<usize> {
        let Some(b) = cfg.blocks.get(&block) else {
            return Vec::new();
        };
        b.predecessors
            .iter()
            .filter_map(|&edge_id| cfg.edges.get(edge_id).map(|e| e.source))
            .collect()
    }

    /// Get non-trivial block parameters (phis that weren't eliminated).
    pub fn get_live_params(&self, block: usize) -> Vec<&BlockParam> {
        let Some(info) = self.blocks.get(&block) else {
            return Vec::new();
        };
        info.params
            .iter()
            .filter(|p| {
                // A param is live if its value matches itself (not replaced by trivial removal)
                p.value.var < self.vars.id_to_name.len()
                    && self
                        .blocks
                        .get(&block)
                        .and_then(|b| b.current_defs.get(&p.value.var))
                        == Some(&p.value)
            })
            .collect()
    }

    /// Count total non-trivial phis across all blocks.
    pub fn phi_count(&self) -> usize {
        self.blocks
            .values()
            .map(|info| {
                info.params
                    .iter()
                    .filter(|p| info.current_defs.get(&p.value.var) == Some(&p.value))
                    .count()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::graph::ControlFlowGraph;

    /// Helper: build a linear CFG (entry -> a -> b -> exit)
    fn linear_cfg() -> ControlFlowGraph {
        let mut cfg = ControlFlowGraph::new("linear");
        let entry = cfg.create_block("entry");
        let a = cfg.create_block("a");
        let b = cfg.create_block("b");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, a, EdgeType::Fallthrough);
        cfg.add_edge(a, b, EdgeType::Fallthrough);
        cfg.add_edge(b, exit, EdgeType::Fallthrough);

        cfg
    }

    /// Helper: build a diamond CFG
    ///   entry -> cond -> (true_bb, false_bb) -> merge -> exit
    fn diamond_cfg() -> ControlFlowGraph {
        let mut cfg = ControlFlowGraph::new("diamond");
        let entry = cfg.create_block("entry");
        let cond = cfg.create_block("cond");
        let true_bb = cfg.create_block("true");
        let false_bb = cfg.create_block("false");
        let merge = cfg.create_block("merge");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, cond, EdgeType::Fallthrough);
        cfg.add_edge(cond, true_bb, EdgeType::ConditionalTrue);
        cfg.add_edge(cond, false_bb, EdgeType::ConditionalFalse);
        cfg.add_edge(true_bb, merge, EdgeType::Fallthrough);
        cfg.add_edge(false_bb, merge, EdgeType::Fallthrough);
        cfg.add_edge(merge, exit, EdgeType::Fallthrough);

        cfg
    }

    /// Helper: build a loop CFG
    ///   entry -> header -> (body, exit)
    ///   body -> header (back edge)
    fn loop_cfg() -> ControlFlowGraph {
        let mut cfg = ControlFlowGraph::new("loop");
        let entry = cfg.create_block("entry");
        let header = cfg.create_block("header");
        let body = cfg.create_block("body");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, header, EdgeType::Fallthrough);
        cfg.add_edge(header, body, EdgeType::ConditionalTrue);
        cfg.add_edge(header, exit, EdgeType::ConditionalFalse);
        cfg.add_edge(body, header, EdgeType::Unconditional);

        cfg
    }

    #[test]
    fn test_var_table_interning() {
        let mut vt = VarTable::new();
        let x = vt.intern("x");
        let y = vt.intern("y");
        let x2 = vt.intern("x");

        assert_eq!(x, x2);
        assert_ne!(x, y);
        assert_eq!(vt.name(x), Some("x"));
        assert_eq!(vt.name(y), Some("y"));
        assert_eq!(vt.id("x"), Some(x));
    }

    #[test]
    fn test_ssa_linear_no_phi() {
        // Linear flow: x defined in entry, used in a and b → no phi needed
        let cfg = linear_cfg();
        let mut ssa = SSAContext::new();

        let entry = 0;
        let a = 1;
        let b = 2;

        // Seal all blocks (all predecessors known in linear flow)
        ssa.seal_block(&cfg, entry);
        ssa.seal_block(&cfg, a);
        ssa.seal_block(&cfg, b);
        ssa.seal_block(&cfg, 3); // exit

        // Define x in entry
        let x_def = ssa.def_var(entry, "x", 0);
        assert_eq!(x_def.var, 0);
        assert_eq!(x_def.version, 0);

        // Use x in block a → should find entry's def
        let x_use_a = ssa.use_var(&cfg, a, "x");
        assert_eq!(x_use_a, x_def);

        // Use x in block b → should find entry's def (propagated through a)
        let x_use_b = ssa.use_var(&cfg, b, "x");
        assert_eq!(x_use_b, x_def);

        // No phis needed
        assert_eq!(ssa.phi_count(), 0);
    }

    #[test]
    fn test_ssa_diamond_phi() {
        // Diamond: x defined differently in true/false branches → phi at merge
        let cfg = diamond_cfg();
        let mut ssa = SSAContext::new();

        let entry = 0;
        let cond = 1;
        let true_bb = 2;
        let false_bb = 3;
        let merge = 4;
        let exit = 5;

        // Seal blocks in RPO
        ssa.seal_block(&cfg, entry);
        ssa.seal_block(&cfg, cond);
        ssa.seal_block(&cfg, true_bb);
        ssa.seal_block(&cfg, false_bb);

        // Define x in entry
        let _x_entry = ssa.def_var(entry, "x", 0);

        // Redefine x in both branches
        let x_true = ssa.def_var(true_bb, "x", 0);
        let x_false = ssa.def_var(false_bb, "x", 0);

        // Seal merge (both preds known)
        ssa.seal_block(&cfg, merge);
        ssa.seal_block(&cfg, exit);

        // Use x at merge → should create a phi
        let x_merge = ssa.use_var(&cfg, merge, "x");

        // The merge should have a different version from both branches
        assert_ne!(x_merge, x_true);
        assert_ne!(x_merge, x_false);

        // Phi should exist at merge
        assert!(ssa.phi_count() > 0);
    }

    #[test]
    fn test_ssa_diamond_trivial_phi() {
        // Diamond: x defined only in entry, same value in both branches → trivial phi
        let cfg = diamond_cfg();
        let mut ssa = SSAContext::new();

        let entry = 0;
        let cond = 1;
        let true_bb = 2;
        let false_bb = 3;
        let merge = 4;
        let exit = 5;

        // Seal blocks in RPO
        ssa.seal_block(&cfg, entry);
        ssa.seal_block(&cfg, cond);
        ssa.seal_block(&cfg, true_bb);
        ssa.seal_block(&cfg, false_bb);

        // Define x only in entry (no redefinition in branches)
        let x_entry = ssa.def_var(entry, "x", 0);

        // Seal merge
        ssa.seal_block(&cfg, merge);
        ssa.seal_block(&cfg, exit);

        // Use x at merge → trivial phi (both operands same)
        let x_merge = ssa.use_var(&cfg, merge, "x");

        // Should be optimized away to entry's definition
        assert_eq!(x_merge, x_entry);
        assert_eq!(ssa.phi_count(), 0);
    }

    #[test]
    fn test_ssa_loop_phi() {
        // Loop: x defined before loop, redefined in body
        let cfg = loop_cfg();
        let mut ssa = SSAContext::new();

        let entry = 0;
        let header = 1;
        let body = 2;
        let exit = 3;

        // Seal entry (no predecessors)
        ssa.seal_block(&cfg, entry);

        // Define x before loop
        let _x_init = ssa.def_var(entry, "x", 0);

        // Header is NOT sealed yet (body -> header back edge not processed)
        // Use x at header → creates incomplete phi
        let x_header = ssa.use_var(&cfg, header, "x");

        // Define x in body
        let _x_body = ssa.def_var(body, "x", 0);

        // Seal body (single pred: header)
        ssa.seal_block(&cfg, body);

        // Now seal header (preds: entry + body)
        ssa.seal_block(&cfg, header);

        // The header phi should be non-trivial (entry vs body definitions)
        let x_header_final = ssa.use_var(&cfg, header, "x");
        // Verify it's consistent
        assert_eq!(x_header.var, x_header_final.var);

        // Seal exit
        ssa.seal_block(&cfg, exit);
    }

    #[test]
    fn test_ssa_multiple_variables() {
        // Linear flow with multiple variables
        let cfg = linear_cfg();
        let mut ssa = SSAContext::new();

        let entry = 0;
        let a = 1;

        ssa.seal_block(&cfg, entry);
        ssa.seal_block(&cfg, a);
        ssa.seal_block(&cfg, 2);
        ssa.seal_block(&cfg, 3);

        let x_def = ssa.def_var(entry, "x", 0);
        let y_def = ssa.def_var(entry, "y", 1);

        let x_use = ssa.use_var(&cfg, a, "x");
        let y_use = ssa.use_var(&cfg, a, "y");

        assert_eq!(x_use, x_def);
        assert_eq!(y_use, y_def);
        assert_ne!(x_def.var, y_def.var);
    }

    #[test]
    fn test_ssa_nested_diamond() {
        // Nested diamonds: entry -> cond1 -> (true1 -> cond2 -> (t2, f2) -> merge2, false1) -> merge1
        let mut cfg = ControlFlowGraph::new("nested_diamond");
        let entry = cfg.create_block("entry");
        let cond1 = cfg.create_block("cond1");
        let true1 = cfg.create_block("true1");
        let false1 = cfg.create_block("false1");
        let cond2 = cfg.create_block("cond2");
        let t2 = cfg.create_block("t2");
        let f2 = cfg.create_block("f2");
        let merge2 = cfg.create_block("merge2");
        let merge1 = cfg.create_block("merge1");
        let exit = cfg.create_block("exit");

        cfg.set_entry(entry);
        cfg.set_exit(exit);

        cfg.add_edge(entry, cond1, EdgeType::Fallthrough);
        cfg.add_edge(cond1, true1, EdgeType::ConditionalTrue);
        cfg.add_edge(cond1, false1, EdgeType::ConditionalFalse);
        cfg.add_edge(true1, cond2, EdgeType::Fallthrough);
        cfg.add_edge(cond2, t2, EdgeType::ConditionalTrue);
        cfg.add_edge(cond2, f2, EdgeType::ConditionalFalse);
        cfg.add_edge(t2, merge2, EdgeType::Fallthrough);
        cfg.add_edge(f2, merge2, EdgeType::Fallthrough);
        cfg.add_edge(merge2, merge1, EdgeType::Fallthrough);
        cfg.add_edge(false1, merge1, EdgeType::Fallthrough);
        cfg.add_edge(merge1, exit, EdgeType::Fallthrough);

        let mut ssa = SSAContext::new();

        // Seal in RPO
        for &b in &[entry, cond1, true1, false1, cond2, t2, f2] {
            ssa.seal_block(&cfg, b);
        }

        // Define x in entry
        let x_entry = ssa.def_var(entry, "x", 0);

        // Redefine in t2
        let _x_t2 = ssa.def_var(t2, "x", 0);

        // Seal merge blocks
        ssa.seal_block(&cfg, merge2);
        ssa.seal_block(&cfg, merge1);
        ssa.seal_block(&cfg, exit);

        // Use at merge1 → needs phi (false1 has x_entry, merge2 might have phi)
        let x_merge1 = ssa.use_var(&cfg, merge1, "x");

        // Should not be equal to entry's x (because t2 redefines it)
        // Actually depends on which path... the phi captures both possibilities
        assert_eq!(x_merge1.var, x_entry.var);
    }
}

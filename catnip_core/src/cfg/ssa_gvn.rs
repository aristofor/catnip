// FILE: catnip_core/src/cfg/ssa_gvn.rs
//! Global Value Numbering (GVN) on SSA form.
//!
//! Walks the dominator tree in preorder. For each SetLocals with a pure RHS,
//! assigns a value number based on (rhs_opcode, VN of operands -- variables and
//! constants). Two expressions with the same value number compute the same
//! value; a later one is redirected to copy the first (canonical) one. This
//! subsumes plain syntactic CSE (it also numbers across copies and phis).

use super::graph::ControlFlowGraph;
use super::ssa::{ExprOperand, SSAContext, SSAValue, ValueDef};
use super::ssa_builder::name_of;
use super::ssa_cse::{extract_rhs_opcode, is_single_target, pure_opcodes, rhs_operands};
use super::ssa_destruction::copy_stmt;
use crate::ir::{IR, IROpCode};
use crate::semantic::passes::collect_refs;
use std::collections::{HashMap, HashSet};

/// A value number: identifies a unique computed value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueNumber(pub u32);

/// Expression key for value numbering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VNExprKey {
    opcode: i32,
    operand_vns: Vec<ValueNumber>,
}

/// GVN state.
pub struct GVNContext {
    /// SSA value -> value number
    value_to_vn: HashMap<SSAValue, ValueNumber>,
    /// Literal operand -> value number (constants are values too)
    literal_to_vn: HashMap<ExprOperand, ValueNumber>,
    /// Expression -> value number (for CSE)
    expr_to_vn: HashMap<VNExprKey, ValueNumber>,
    /// Next value number to assign
    next_vn: u32,
    /// Value number -> canonical SSA value (first definition with this VN)
    vn_to_canonical: HashMap<ValueNumber, SSAValue>,
}

impl GVNContext {
    fn new() -> Self {
        Self {
            value_to_vn: HashMap::new(),
            literal_to_vn: HashMap::new(),
            expr_to_vn: HashMap::new(),
            next_vn: 0,
            vn_to_canonical: HashMap::new(),
        }
    }

    fn alloc_vn(&mut self) -> ValueNumber {
        let vn = ValueNumber(self.next_vn);
        self.next_vn += 1;
        vn
    }

    fn fresh_vn(&mut self, value: SSAValue) -> ValueNumber {
        let vn = self.alloc_vn();
        self.value_to_vn.insert(value, vn);
        self.vn_to_canonical.entry(vn).or_insert(value);
        vn
    }

    fn get_vn(&mut self, value: SSAValue) -> ValueNumber {
        if let Some(&vn) = self.value_to_vn.get(&value) {
            vn
        } else {
            self.fresh_vn(value)
        }
    }

    /// VN of an operand: the SSA value's VN for a variable, or a stable per-value
    /// VN for a literal (so `a + 4` and `a + 11` differ).
    fn vn_of_operand(&mut self, operand: &ExprOperand) -> ValueNumber {
        match operand {
            ExprOperand::Var(v) => self.get_vn(*v),
            literal => {
                if let Some(&vn) = self.literal_to_vn.get(literal) {
                    vn
                } else {
                    let vn = self.alloc_vn();
                    self.literal_to_vn.insert(literal.clone(), vn);
                    vn
                }
            }
        }
    }

    fn lookup_or_add(&mut self, key: VNExprKey, defining_value: SSAValue) -> ValueNumber {
        if let Some(&vn) = self.expr_to_vn.get(&key) {
            self.value_to_vn.insert(defining_value, vn);
            vn
        } else {
            let vn = self.fresh_vn(defining_value);
            self.expr_to_vn.insert(key, vn);
            vn
        }
    }
}

/// Result of GVN.
pub struct GVNResult {
    /// Number of redundant expressions found
    pub redundant: usize,
    /// Replacements: SSA value -> canonical SSA value with same VN
    pub replacements: HashMap<SSAValue, SSAValue>,
}

/// Run GVN on the CFG in SSA form.
pub fn gvn(cfg: &ControlFlowGraph, ssa: &SSAContext) -> GVNResult {
    let pure_ops = pure_opcodes();
    let mut ctx = GVNContext::new();
    let mut replacements: HashMap<SSAValue, SSAValue> = HashMap::new();
    let mut redundant = 0;

    // SSA values proven to hold an immutable scalar (a scalar literal, or a pure
    // op over proven scalars). Only these are safe to materialize as an alias:
    // a redundant `a + b` whose result is a freshly-allocated mutable value (list
    // concat, set op, ...) must be recomputed, else a later mutation through the
    // canonical is observed through the copy. (Codex adversarial review.)
    let mut scalars: HashSet<SSAValue> = HashSet::new();

    // Assign initial VNs to phi-defined values
    for (value, def) in &ssa.value_defs {
        if matches!(def, ValueDef::BlockParam { .. }) {
            ctx.fresh_vn(*value);
        }
    }

    let dom_preorder = dominator_preorder(cfg);

    for &block_id in &dom_preorder {
        let Some(block) = cfg.blocks.get(&block_id) else {
            continue;
        };

        for (instr_idx, op) in block.instructions.iter().enumerate() {
            let def_value = find_def_value(ssa, block_id, instr_idx);
            let single = is_single_target(op);
            let uses = ssa.get_uses(block_id, instr_idx);

            // Only process SetLocals with a pure RHS op
            let rhs_opcode = extract_rhs_opcode(op);
            let is_pure_setlocals = rhs_opcode.map(|opc| pure_ops.contains(&opc)).unwrap_or(false);

            // Track immutable-scalar-ness of the def: a scalar literal, or a pure
            // op over proven scalars. Done before the early continues so that a
            // plain `a = 5` (whose RHS is not an op) still seeds the set.
            let rhs_ops = if is_pure_setlocals {
                rhs_operands(op, uses)
            } else {
                None
            };
            if let Some(value) = def_value {
                if single {
                    if rhs_is_scalar_literal(op) {
                        scalars.insert(value);
                    } else if let Some(ops) = &rhs_ops {
                        if operands_all_scalar(ops, &scalars) {
                            scalars.insert(value);
                        }
                    }
                }
            }

            if !is_pure_setlocals {
                // Non-pure or non-SetLocals: unique VN
                if let Some(value) = def_value {
                    ctx.fresh_vn(value);
                }
                continue;
            }

            let Some(value) = def_value else {
                continue;
            };

            // A destructuring assignment `(a, b) = e` defines *elements* of `e`,
            // not its value, so its target must not take the RHS's value number
            // (`find_def_value` returns just one of the element defs). Only a
            // single-target, non-unpack `x = e` stores the whole RHS. (Codex
            // adversarial review, 2026-06-18.)
            if !single {
                ctx.fresh_vn(value);
                continue;
            }

            // Build the VN key from RHS operands (variables AND literals); a
            // non-keyable RHS (sub-expression, call, kwargs) gets a unique VN.
            let Some(operands) = rhs_ops else {
                ctx.fresh_vn(value);
                continue;
            };
            let operand_vns: Vec<ValueNumber> = operands.iter().map(|o| ctx.vn_of_operand(o)).collect();
            let key = VNExprKey {
                opcode: rhs_opcode.unwrap(),
                operand_vns,
            };

            let vn = ctx.lookup_or_add(key, value);

            // Materialize as an alias only when the result is a proven immutable
            // scalar (so the copy can never observe a later mutation), the
            // canonical dominates this use, and they differ.
            if let Some(&canonical) = ctx.vn_to_canonical.get(&vn) {
                if canonical != value && scalars.contains(&value) && dominates_use(cfg, ssa, canonical, block_id) {
                    replacements.insert(value, canonical);
                    redundant += 1;
                }
            }
        }
    }

    GVNResult {
        redundant,
        replacements,
    }
}

/// Whether the canonical value's definition dominates the block of the use.
///
/// Same soundness concern as CSE: `vn_to_canonical` keeps the first value seen in
/// dominator preorder, which lingers across sibling branches it does not
/// dominate. Redirecting a use to a non-dominating definition reads an undefined
/// value. (Codex adversarial review, 2026-06-18.)
fn dominates_use(cfg: &ControlFlowGraph, ssa: &SSAContext, canonical: SSAValue, use_block: usize) -> bool {
    let def_block = match ssa.value_defs.get(&canonical) {
        Some(ValueDef::Instruction { block, .. }) | Some(ValueDef::BlockParam { block, .. }) => *block,
        None => return false,
    };
    def_block == use_block
        || cfg
            .blocks
            .get(&use_block)
            .is_some_and(|b| b.dominators.contains(&def_block))
}

/// Whether a `SetLocals`'s RHS is a scalar (immutable) literal. Such a value can
/// never be mutated through an alias.
fn rhs_is_scalar_literal(set_locals: &IR) -> bool {
    let IR::Op { args, .. } = set_locals else {
        return false;
    };
    matches!(
        args.get(1),
        Some(IR::Int(_) | IR::Float(_) | IR::Bool(_) | IR::String(_) | IR::None)
    )
}

/// Whether every operand is a proven immutable scalar: a variable already in the
/// scalar set, or a scalar literal (the only literals `rhs_operands` emits). A
/// pure op over such operands yields a native immutable scalar -- no struct
/// operator dispatch, no fresh mutable allocation -- so it is safe to alias.
fn operands_all_scalar(operands: &[ExprOperand], scalars: &HashSet<SSAValue>) -> bool {
    operands.iter().all(|o| match o {
        ExprOperand::Var(v) => scalars.contains(v),
        _ => true,
    })
}

fn find_def_value(ssa: &SSAContext, block_id: usize, instr_idx: usize) -> Option<SSAValue> {
    for (value, def) in &ssa.value_defs {
        if let ValueDef::Instruction { block, instr_idx: idx } = def {
            if *block == block_id && *idx == instr_idx {
                return Some(*value);
            }
        }
    }
    None
}

fn dominator_preorder(cfg: &ControlFlowGraph) -> Vec<usize> {
    let Some(entry) = cfg.entry else {
        return Vec::new();
    };

    let mut result = Vec::new();
    let mut stack = vec![entry];

    while let Some(block_id) = stack.pop() {
        result.push(block_id);
        if let Some(block) = cfg.blocks.get(&block_id) {
            let mut children: Vec<usize> = block.dominated.iter().copied().collect();
            children.sort_unstable();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
    }

    result
}

/// Names already referenced anywhere in the CFG (refs, including inside
/// op-preserved nested bodies, plus SetLocals targets and block conditions): the
/// collision set for fresh snapshot temporaries.
fn used_names(cfg: &ControlFlowGraph) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut record = |n: String| {
        names.insert(n);
    };
    for block in cfg.blocks.values() {
        for op in &block.instructions {
            collect_refs(op, &mut record);
            // SetLocals targets may be Identifier/String nodes, which
            // collect_refs does not visit.
            if let IR::Op { opcode, args, .. } = op {
                if *opcode == IROpCode::SetLocals {
                    match args.first() {
                        Some(IR::Tuple(items)) => {
                            for it in items {
                                if let Some(n) = name_of(it) {
                                    record(n);
                                }
                            }
                        }
                        Some(other) => {
                            if let Some(n) = name_of(other) {
                                record(n);
                            }
                        }
                        None => {}
                    }
                }
            }
        }
        if let Some(cond) = &block.condition {
            collect_refs(cond, &mut record);
        }
    }
    names
}

/// Next `__gvn{n}` not present in `used`. Deterministic (monotonic counter, no
/// randomness) so repeated compilations of the same program emit the same IR.
fn fresh_gvn_name(used: &HashSet<String>, counter: &mut usize) -> String {
    loop {
        let candidate = format!("__gvn{}", *counter);
        *counter += 1;
        if !used.contains(&candidate) {
            return candidate;
        }
    }
}

/// Materialize GVN results into the CFG: rewrite each redundant `SetLocals` so it
/// copies its canonical definition instead of recomputing.
///
/// Two shapes, by the canonical variable's instruction-def count:
///
/// - single-def: the bare name always carries the canonical value, alias to it
///   directly (`y = x`);
/// - multi-def: the bare name may be overwritten between the canonical def and
///   the use, so the canonical value is captured by a fresh snapshot temporary
///   inserted right after its def (`__gvnN = x`) and the redundancy aliases the
///   temp (`y = __gvnN`).
///
/// The snapshot form is additive only: no existing def or use is renamed, so
/// late-bound reads (closure bodies resolve captured names at call time, and
/// op-preserved regions are opaque) cannot observe it -- the soundness hole a
/// versioned rename has (see the `maximal_naming_closure_*` oracle in
/// `catnip_vm/tests/cfg_roundtrip.rs`). Dominance and scalar-immutability were
/// checked in `gvn`; the snapshot inherits dominance by adjacency to the
/// canonical def, and is single-def by construction. Returns the number of
/// redundancies applied.
pub fn materialize_gvn(cfg: &mut ControlFlowGraph, ssa: &SSAContext, result: &GVNResult) -> usize {
    let used = used_names(cfg);
    let mut counter = 0usize;
    // One snapshot per canonical, allocated on first demand.
    let mut snapshots: HashMap<SSAValue, String> = HashMap::new();
    // (block, insertion idx, dst, src): inserted after all RHS rewrites so the
    // SSA instruction indices stay valid while they are still being read.
    let mut insertions: Vec<(usize, usize, String, String)> = Vec::new();
    let mut applied = 0;

    // `replacements` is a HashMap: iterate in the redundant def's program order
    // so snapshot numbering is stable across runs (compilation must stay
    // deterministic for caching).
    let mut repls: Vec<(SSAValue, SSAValue)> = result.replacements.iter().map(|(&v, &c)| (v, c)).collect();
    repls.sort_by_key(|(v, _)| match ssa.value_defs.get(v) {
        Some(ValueDef::Instruction { block, instr_idx }) => (*block, *instr_idx),
        _ => (usize::MAX, usize::MAX),
    });

    for (value, canonical) in repls {
        // Canonical must be an instruction definition (a phi has no stable
        // variable name).
        let Some(&ValueDef::Instruction {
            block: cblock,
            instr_idx: cidx,
        }) = ssa.value_defs.get(&canonical)
        else {
            continue;
        };
        let Some(canonical_name) = ssa.vars.name(canonical.var).map(|s| s.to_string()) else {
            continue;
        };
        // The redundant def must be a rewritable `x = e` before any snapshot is
        // allocated for it (no orphan snapshot on a shape mismatch).
        let Some(&ValueDef::Instruction { block, instr_idx }) = ssa.value_defs.get(&value) else {
            continue;
        };
        let rewritable = cfg
            .blocks
            .get(&block)
            .and_then(|b| b.instructions.get(instr_idx))
            .is_some_and(
                |op| matches!(op, IR::Op { opcode, args, .. } if *opcode == IROpCode::SetLocals && args.len() >= 2),
            );
        if !rewritable {
            continue;
        }

        let def_count = ssa
            .value_defs
            .iter()
            .filter(|(v, d)| v.var == canonical.var && matches!(d, ValueDef::Instruction { .. }))
            .count();
        let src_name = if def_count == 1 {
            canonical_name
        } else {
            snapshots
                .entry(canonical)
                .or_insert_with(|| {
                    let tmp = fresh_gvn_name(&used, &mut counter);
                    insertions.push((cblock, cidx + 1, tmp.clone(), canonical_name.clone()));
                    tmp
                })
                .clone()
        };

        if let Some(b) = cfg.get_block_mut(block) {
            if let Some(IR::Op { args, .. }) = b.instructions.get_mut(instr_idx) {
                args[1] = IR::Ref(src_name, -1, -1);
                applied += 1;
            }
        }
    }

    // Insert snapshots bottom-up per block so earlier insertion points stay
    // valid.
    insertions.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    for (block, idx, dst, src) in insertions {
        if let Some(b) = cfg.get_block_mut(block) {
            let at = idx.min(b.instructions.len());
            b.instructions.insert(at, copy_stmt(&dst, &src));
        }
    }
    applied
}

/// Materialize GVN redundancies against a versioned `naming`, lifting the
/// single-def restriction of [`materialize_gvn`].
///
/// Runs AFTER `rename_versioned`: each redundant value's def already carries
/// versioned names, so its RHS is replaced by a copy of the canonical's
/// *versioned* name. The canonical may be multi-def -- versioning gives its
/// specific version a unique name that holds the value at the use (dominance and
/// scalar-immutability were checked in `gvn`; a versioned name is what the old
/// pass lacked). Returns the number of redundancies applied.
pub fn materialize_gvn_versioned(
    cfg: &mut ControlFlowGraph,
    ssa: &SSAContext,
    result: &GVNResult,
    naming: &HashMap<SSAValue, String>,
) -> usize {
    let mut applied = 0;
    for (&value, &canonical) in &result.replacements {
        // Canonical must be an instruction definition (a phi has no scalar-stable
        // meaning here; kept from materialize_gvn).
        if !matches!(ssa.value_defs.get(&canonical), Some(ValueDef::Instruction { .. })) {
            continue;
        }
        let Some(cname) = naming.get(&canonical) else {
            continue;
        };
        let Some(ValueDef::Instruction { block, instr_idx }) = ssa.value_defs.get(&value) else {
            continue;
        };
        if let Some(b) = cfg.get_block_mut(*block) {
            if let Some(IR::Op { opcode, args, .. }) = b.instructions.get_mut(*instr_idx) {
                if *opcode == IROpCode::SetLocals && args.len() >= 2 {
                    args[1] = IR::Ref(cname.clone(), -1, -1);
                    applied += 1;
                }
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::analysis::compute_dominators;
    use crate::cfg::edge::EdgeType;
    use crate::cfg::ssa_builder::SSABuilder;

    #[test]
    fn test_gvn_empty() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_block("entry");
        let exit = cfg.create_block("exit");
        cfg.set_entry(entry);
        cfg.set_exit(exit);
        cfg.add_edge(entry, exit, EdgeType::Fallthrough);

        compute_dominators(&mut cfg);
        let ssa = SSABuilder::build(&cfg);

        let result = gvn(&cfg, &ssa);
        assert_eq!(result.redundant, 0);
    }

    #[test]
    fn test_gvn_context_fresh_vn() {
        let mut ctx = GVNContext::new();
        let v1 = SSAValue::new(0, 0);
        let v2 = SSAValue::new(1, 0);

        let vn1 = ctx.fresh_vn(v1);
        let vn2 = ctx.fresh_vn(v2);

        assert_ne!(vn1, vn2);
        assert_eq!(ctx.get_vn(v1), vn1);
        assert_eq!(ctx.get_vn(v2), vn2);
    }

    #[test]
    fn test_gvn_context_lookup_or_add() {
        let mut ctx = GVNContext::new();
        let v1 = SSAValue::new(0, 0);
        let v2 = SSAValue::new(0, 1);

        let key = VNExprKey {
            opcode: IROpCode::Add as i32,
            operand_vns: vec![ValueNumber(0)],
        };

        let vn1 = ctx.lookup_or_add(key.clone(), v1);
        let vn2 = ctx.lookup_or_add(key, v2);

        assert_eq!(vn1, vn2);
    }

    /// Distinct literal operands get distinct VNs, so `a + 4` and `a + 11` keys
    /// differ even though the variable operand is the same.
    #[test]
    fn test_gvn_literal_operands_distinct() {
        let mut ctx = GVNContext::new();
        let four = ctx.vn_of_operand(&ExprOperand::Int(4));
        let eleven = ctx.vn_of_operand(&ExprOperand::Int(11));
        let four_again = ctx.vn_of_operand(&ExprOperand::Int(4));

        assert_ne!(four, eleven);
        assert_eq!(four, four_again);
    }
}

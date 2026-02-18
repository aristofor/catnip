// FILE: catnip_rs/src/vm/peephole.rs
//! Peephole optimization for VM bytecode.
//!
//! Applies local optimization passes to reduce bytecode size and improve performance:
//! 1. Jump chaining resolution
//! 2. Dead code detection and elimination
//! 3. Pattern folding (DupTop+PopTop, etc.)
//! 4. Instruction compaction

use crate::vm::opcode::{Instruction, VMOpCode};
use std::collections::{HashMap, HashSet};

/// Peephole optimizer for VM bytecode.
pub struct PeepholeOptimizer;

impl PeepholeOptimizer {
    /// Optimize instructions through multiple passes.
    ///
    /// Returns optimized (instructions, line_table) with reduced bytecode.
    pub fn optimize(
        instructions: Vec<Instruction>,
        line_table: Vec<u32>,
    ) -> (Vec<Instruction>, Vec<u32>) {
        let mut instrs = instructions;

        // Phase 1: Resolve jump chains (must be first)
        instrs = Self::resolve_jumps(instrs);

        // Phase 2: Mark live code (reachable instructions)
        let live_set = Self::mark_live_code(&instrs);

        // Phase 3: Fold peephole patterns
        instrs = Self::fold_patterns(instrs, &live_set);

        // Phase 4: Compact by removing dead code (with line_table)
        let (instrs, line_table) = Self::compact_with_line_table(instrs, line_table, &live_set);

        (instrs, line_table)
    }

    /// **Phase 1: Resolve jump chains**
    ///
    /// Follow chains of jumps to their final destination.
    /// Example: `Jump L1` where `L1: Jump L2` becomes `Jump L2` directly.
    ///
    /// This must be done first because later passes depend on jump targets.
    fn resolve_jumps(mut instrs: Vec<Instruction>) -> Vec<Instruction> {
        // First pass: for each jump, follow the chain to final target
        for i in 0..instrs.len() {
            if instrs[i].op == VMOpCode::Jump {
                let mut target = instrs[i].arg as usize;
                let mut seen = HashSet::new();

                // Follow jump chain, avoiding infinite loops
                while target < instrs.len()
                    && instrs[target].op == VMOpCode::Jump
                    && seen.insert(target)
                {
                    target = instrs[target].arg as usize;
                }

                // Update to final target
                instrs[i].arg = target as u32;
            }
        }

        // Second pass: remove jumps to next instruction (noop jumps)
        for i in 0..instrs.len() {
            if instrs[i].op == VMOpCode::Jump && instrs[i].arg as usize == i + 1 {
                instrs[i].op = VMOpCode::Nop;
            }
        }

        instrs
    }

    /// **Phase 2: Mark live (reachable) code**
    ///
    /// Uses forward reachability analysis to identify which instructions are reachable.
    /// An instruction is live if:
    /// 1. It's the first instruction (entry point)
    /// 2. It's reachable from a live instruction
    /// 3. It's a jump target
    ///
    /// Dead code detection:
    /// - Code after Return/Break/Continue is dead (control doesn't flow)
    /// - Code after unconditional Jump is dead
    /// - Unreachable loop sections are detected
    ///
    /// Example:
    /// ```ignore
    /// 0: LoadConst 1      // Live (entry)
    /// 1: Return           // Live (entry point)
    /// 2: LoadConst 2      // DEAD (after Return)
    /// 3: Return           // DEAD (unreachable)
    ///
    /// 4: Jump 6           // Live
    /// 5: LoadConst 3      // DEAD (skipped by Jump)
    /// 6: Return           // Live (jump target)
    /// ```
    fn mark_live_code(instrs: &[Instruction]) -> HashSet<usize> {
        let mut live = HashSet::new();
        let mut to_visit = vec![0]; // Start from first instruction (entry point)

        while let Some(idx) = to_visit.pop() {
            if !live.insert(idx) {
                continue; // Already visited
            }

            if idx >= instrs.len() {
                continue;
            }

            let instr = &instrs[idx];

            // Terminal instructions: control flow stops, don't continue
            if matches!(
                instr.op,
                VMOpCode::Return
                    | VMOpCode::Break
                    | VMOpCode::Continue
                    | VMOpCode::Nop
                    | VMOpCode::Halt
            ) {
                continue;
            }

            // Unconditional jump: only visit target
            // Code after is dead (unreachable)
            if instr.op == VMOpCode::Jump {
                let target = instr.arg as usize;
                if target < instrs.len() {
                    to_visit.push(target);
                }
                // Don't continue to next instruction (dead code)
                continue;
            }

            // Conditional jumps: visit both branches
            if matches!(
                instr.op,
                VMOpCode::JumpIfFalse
                    | VMOpCode::JumpIfTrue
                    | VMOpCode::JumpIfFalseOrPop
                    | VMOpCode::JumpIfTrueOrPop
                    | VMOpCode::JumpIfNone
            ) {
                let target = instr.arg as usize;
                if target < instrs.len() {
                    to_visit.push(target);
                }
                // Also continue to next (other branch)
                if idx + 1 < instrs.len() {
                    to_visit.push(idx + 1);
                }
                continue;
            }

            // ForIter special handling: can jump or continue
            if instr.op == VMOpCode::ForIter {
                let target = instr.arg as usize;
                if target < instrs.len() {
                    to_visit.push(target); // Jump when iterator exhausted
                }
                if idx + 1 < instrs.len() {
                    to_visit.push(idx + 1); // Continue on next iteration
                }
                continue;
            }

            // ForRangeStep: unconditional backward jump (no fallthrough)
            // arg format: (slot_i << 24) | (step_i8 << 16) | jump_target
            if instr.op == VMOpCode::ForRangeStep {
                let jump_target = (instr.arg & 0xFFFF) as usize;
                if jump_target < instrs.len() {
                    to_visit.push(jump_target);
                }
                continue; // no fallthrough
            }

            // ForRangeInt special handling: can jump (via relative offset) or continue
            // arg format: (slot_i << 24) | (slot_stop << 16) | (step_sign << 15) | jump_offset
            // jump_offset is the lower 15 bits
            if instr.op == VMOpCode::ForRangeInt {
                let jump_offset = (instr.arg & 0x7FFF) as usize;
                let target = idx + jump_offset;
                if target < instrs.len() {
                    to_visit.push(target); // Jump when loop ends
                }
                if idx + 1 < instrs.len() {
                    to_visit.push(idx + 1); // Continue on next iteration
                }
                continue;
            }

            // Normal instruction: continue to next
            if idx + 1 < instrs.len() {
                to_visit.push(idx + 1);
            }
        }

        live
    }

    /// **Phase 3: Fold peephole patterns**
    ///
    /// Detect and eliminate common patterns:
    /// - DupTop + PopTop → remove both (useless stack operation)
    /// - Jump offset errors (handled in resolve_jumps)
    fn fold_patterns(mut instrs: Vec<Instruction>, live_set: &HashSet<usize>) -> Vec<Instruction> {
        let mut i = 0;
        while i < instrs.len().saturating_sub(1) {
            if !live_set.contains(&i) {
                i += 1;
                continue;
            }

            // Pattern: DupTop + PopTop → remove both (dead stack operations)
            if instrs[i].op == VMOpCode::DupTop
                && i + 1 < instrs.len()
                && instrs[i + 1].op == VMOpCode::PopTop
                && live_set.contains(&(i + 1))
            {
                instrs[i].op = VMOpCode::Nop;
                instrs[i + 1].op = VMOpCode::Nop;
                i += 2;
                continue;
            }

            i += 1;
        }

        instrs
    }

    /// **Phase 4: Compact instructions** (with line_table tracking)
    ///
    /// Remove dead code (Nop instructions), adjust jump offsets,
    /// and keep line_table in sync with instructions.
    fn compact_with_line_table(
        instrs: Vec<Instruction>,
        line_table: Vec<u32>,
        live_set: &HashSet<usize>,
    ) -> (Vec<Instruction>, Vec<u32>) {
        let mut old_to_new = HashMap::new();
        let mut result = Vec::new();
        let mut result_line_table = Vec::new();

        for (old_idx, instr) in instrs.iter().enumerate() {
            if live_set.contains(&old_idx) && instr.op != VMOpCode::Nop {
                old_to_new.insert(old_idx, result.len());
                result.push(*instr);
                // Carry over line_table entry (use 0 if missing)
                result_line_table.push(line_table.get(old_idx).copied().unwrap_or(0));
            }
        }

        // Update jump targets
        for i in 0..result.len() {
            if matches!(
                result[i].op,
                VMOpCode::Jump
                    | VMOpCode::JumpIfFalse
                    | VMOpCode::JumpIfTrue
                    | VMOpCode::JumpIfFalseOrPop
                    | VMOpCode::JumpIfTrueOrPop
                    | VMOpCode::JumpIfNone
                    | VMOpCode::ForIter
            ) {
                let old_target = result[i].arg as usize;
                if let Some(&new_target) = old_to_new.get(&old_target) {
                    result[i].arg = new_target as u32;
                }
            }
        }

        (result, result_line_table)
    }

    /// Convenience wrapper for tests: compact without line_table.
    #[cfg(test)]
    fn compact(instrs: Vec<Instruction>, live_set: &HashSet<usize>) -> Vec<Instruction> {
        let n = instrs.len();
        Self::compact_with_line_table(instrs, vec![0; n], live_set).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instr(op: VMOpCode, arg: u32) -> Instruction {
        Instruction::new(op, arg)
    }

    #[test]
    fn test_resolve_simple_jump() {
        // Jump 0, Jump 2, some_op
        let instrs = vec![
            make_instr(VMOpCode::Jump, 1),
            make_instr(VMOpCode::Jump, 2),
            make_instr(VMOpCode::LoadConst, 0),
        ];

        let optimized = PeepholeOptimizer::resolve_jumps(instrs);

        // First jump should now point directly to instruction 2
        assert_eq!(optimized[0].arg, 2);
    }

    #[test]
    fn test_resolve_jump_chain() {
        // Jump 1, Jump 2, Jump 3, Return
        let instrs = vec![
            make_instr(VMOpCode::Jump, 1),
            make_instr(VMOpCode::Jump, 2),
            make_instr(VMOpCode::Jump, 3),
            make_instr(VMOpCode::Return, 0),
        ];

        let optimized = PeepholeOptimizer::resolve_jumps(instrs);

        // Jump should skip entire chain and point to final target (3)
        assert_eq!(optimized[0].arg, 3);
        assert_eq!(optimized[1].arg, 3);
    }

    #[test]
    fn test_remove_noop_jumps() {
        // Jump to next instruction
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Jump, 2), // Jump to next (noop)
            make_instr(VMOpCode::LoadConst, 1),
        ];

        let optimized = PeepholeOptimizer::resolve_jumps(instrs);

        // Noop jump should be converted to Nop
        assert_eq!(optimized[1].op, VMOpCode::Nop);
    }

    #[test]
    fn test_fold_dupop_popop() {
        // DupTop, PopTop, LoadConst
        let instrs = vec![
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
            make_instr(VMOpCode::LoadConst, 0),
        ];

        let live_set: HashSet<_> = [0, 1, 2].iter().copied().collect();
        let optimized = PeepholeOptimizer::fold_patterns(instrs, &live_set);

        // DupTop and PopTop should be marked as Nop
        assert_eq!(optimized[0].op, VMOpCode::Nop);
        assert_eq!(optimized[1].op, VMOpCode::Nop);
    }

    #[test]
    fn test_fold_dupop_popop_multiple() {
        // Multiple DupTop+PopTop sequences
        let instrs = vec![
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
            make_instr(VMOpCode::Return, 0),
        ];

        let live_set: HashSet<_> = [0, 1, 2, 3, 4, 5].iter().copied().collect();
        let optimized = PeepholeOptimizer::fold_patterns(instrs, &live_set);

        // Both DupTop+PopTop pairs should be marked as Nop
        assert_eq!(optimized[0].op, VMOpCode::Nop);
        assert_eq!(optimized[1].op, VMOpCode::Nop);
        assert_eq!(optimized[3].op, VMOpCode::Nop);
        assert_eq!(optimized[4].op, VMOpCode::Nop);
    }

    #[test]
    fn test_fold_dupop_popop_non_live() {
        // DupTop+PopTop in dead code should not be folded
        let instrs = vec![
            make_instr(VMOpCode::Return, 0),
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
        ];

        let live_set: HashSet<_> = [0].iter().copied().collect();
        let optimized = PeepholeOptimizer::fold_patterns(instrs, &live_set);

        // Dead code DupTop+PopTop should NOT be folded (only live code folded)
        assert_eq!(optimized[1].op, VMOpCode::DupTop);
        assert_eq!(optimized[2].op, VMOpCode::PopTop);
    }

    #[test]
    fn test_fold_dupop_popop_partial_live() {
        // Only first of pair is live
        let instrs = vec![
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
            make_instr(VMOpCode::LoadConst, 0),
        ];

        let live_set: HashSet<_> = [0, 2].iter().copied().collect(); // Only DupTop and LoadConst
        let optimized = PeepholeOptimizer::fold_patterns(instrs, &live_set);

        // Pattern not matched if both not live
        assert_eq!(optimized[0].op, VMOpCode::DupTop);
        assert_eq!(optimized[1].op, VMOpCode::PopTop);
    }

    #[test]
    fn test_compact_removes_nop() {
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Nop, 0),
            make_instr(VMOpCode::LoadConst, 1),
        ];

        let live_set: HashSet<_> = [0, 1, 2].iter().copied().collect();
        let compacted = PeepholeOptimizer::compact(instrs, &live_set);

        // Should only have 2 instructions
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0].op, VMOpCode::LoadConst);
        assert_eq!(compacted[1].op, VMOpCode::LoadConst);
    }

    #[test]
    fn test_compact_removes_multiple_nops() {
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Nop, 0),
            make_instr(VMOpCode::Nop, 0),
            make_instr(VMOpCode::LoadConst, 1),
            make_instr(VMOpCode::Nop, 0),
        ];

        let live_set: HashSet<_> = [0, 1, 2, 3, 4].iter().copied().collect();
        let compacted = PeepholeOptimizer::compact(instrs, &live_set);

        // Should only have 2 instructions (LoadConst + LoadConst)
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0].op, VMOpCode::LoadConst);
        assert_eq!(compacted[1].op, VMOpCode::LoadConst);
    }

    #[test]
    fn test_compact_updates_jumps() {
        // Jump 2, LoadConst, Nop, Return
        let instrs = vec![
            make_instr(VMOpCode::Jump, 3),
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Nop, 0), // Will be removed
            make_instr(VMOpCode::Return, 0),
        ];

        let live_set: HashSet<_> = [0, 1, 2, 3].iter().copied().collect();
        let compacted = PeepholeOptimizer::compact(instrs, &live_set);

        // Jump should now point to new index 2 (was 3 before compaction)
        assert_eq!(compacted[0].arg, 2);
    }

    #[test]
    fn test_compact_updates_multiple_jumps() {
        // Multiple jumps needing offset updates
        let instrs = vec![
            make_instr(VMOpCode::JumpIfFalse, 4),
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Nop, 0), // removed
            make_instr(VMOpCode::Jump, 6),
            make_instr(VMOpCode::LoadConst, 1),
            make_instr(VMOpCode::Nop, 0), // removed
            make_instr(VMOpCode::Return, 0),
        ];

        let live_set: HashSet<_> = [0, 1, 2, 3, 4, 5, 6].iter().copied().collect();
        let compacted = PeepholeOptimizer::compact(instrs, &live_set);

        // JumpIfFalse was at 4, after removing indices 2: new index is 3
        assert_eq!(compacted[0].arg, 3);
        // Jump was at 6, after removing indices 2,5: new index is 4
        assert_eq!(compacted[2].arg, 4);
    }

    #[test]
    fn test_compact_respects_live_set() {
        // Instructions not in live_set should be removed even if not Nop
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::LoadConst, 1), // Dead
            make_instr(VMOpCode::LoadConst, 2),
        ];

        let live_set: HashSet<_> = [0, 2].iter().copied().collect();
        let compacted = PeepholeOptimizer::compact(instrs, &live_set);

        // Only indices 0 and 2 should remain
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0].arg, 0);
        assert_eq!(compacted[1].arg, 2);
    }

    #[test]
    fn test_mark_live_simple_return() {
        // LoadConst, Return, LoadConst (dead)
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Return, 0),
            make_instr(VMOpCode::LoadConst, 1), // DEAD - after Return
        ];

        let live = PeepholeOptimizer::mark_live_code(&instrs);

        assert!(live.contains(&0)); // LoadConst is live
        assert!(live.contains(&1)); // Return is live
        assert!(!live.contains(&2)); // LoadConst after Return is DEAD
    }

    #[test]
    fn test_mark_live_unconditional_jump() {
        // LoadConst, Jump 3, LoadConst (dead), Return
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::Jump, 3),
            make_instr(VMOpCode::LoadConst, 1), // DEAD - skipped by Jump
            make_instr(VMOpCode::Return, 0),
        ];

        let live = PeepholeOptimizer::mark_live_code(&instrs);

        assert!(live.contains(&0)); // LoadConst is live
        assert!(live.contains(&1)); // Jump is live
        assert!(!live.contains(&2)); // LoadConst after Jump is DEAD
        assert!(live.contains(&3)); // Return is live (jump target)
    }

    #[test]
    fn test_mark_live_conditional_jump() {
        // LoadConst, JumpIfFalse 3, LoadConst, Return, Return
        // Code after JumpIfFalse should be live (other branch)
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::JumpIfFalse, 3),
            make_instr(VMOpCode::LoadConst, 1), // Live (true branch)
            make_instr(VMOpCode::Return, 0),    // Live (jump target)
            make_instr(VMOpCode::Return, 0),    // Dead (after previous return)
        ];

        let live = PeepholeOptimizer::mark_live_code(&instrs);

        assert!(live.contains(&0)); // LoadConst
        assert!(live.contains(&1)); // JumpIfFalse
        assert!(live.contains(&2)); // LoadConst (true branch)
        assert!(live.contains(&3)); // Return
        assert!(!live.contains(&4)); // Return after previous return - DEAD
    }

    #[test]
    fn test_mark_live_nested_jumps() {
        // Jump 1, (dead), Jump 3, LoadConst, Return
        let instrs = vec![
            make_instr(VMOpCode::Jump, 2),      // Jump over dead code
            make_instr(VMOpCode::LoadConst, 0), // DEAD
            make_instr(VMOpCode::Jump, 4),      // Jump to Return
            make_instr(VMOpCode::LoadConst, 1), // DEAD
            make_instr(VMOpCode::Return, 0),    // Live (final target)
        ];

        let live = PeepholeOptimizer::mark_live_code(&instrs);

        assert!(live.contains(&0)); // First Jump
        assert!(!live.contains(&1)); // LoadConst after Jump - DEAD
        assert!(live.contains(&2)); // Second Jump
        assert!(!live.contains(&3)); // LoadConst after Jump - DEAD
        assert!(live.contains(&4)); // Return
    }

    #[test]
    fn test_full_optimization_flow() {
        // Jump 1, Jump 2, LoadConst, DupTop, PopTop, Return
        let instrs = vec![
            make_instr(VMOpCode::Jump, 1),
            make_instr(VMOpCode::Jump, 2),
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::DupTop, 0),
            make_instr(VMOpCode::PopTop, 0),
            make_instr(VMOpCode::Return, 0),
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // Should have: Jump to 1 (now points to LoadConst after folding)
        // Then LoadConst, Return (DupTop+PopTop removed, Return kept)
        assert!(optimized.len() <= 4); // Some instructions removed
    }

    #[test]
    fn test_phase2_dead_code_complex() {
        // Simulate complex control flow: if-else with dead code
        // JumpIfFalse 5 (else), LoadConst 1, Jump 6, LoadConst 2, Return
        let instrs = vec![
            make_instr(VMOpCode::JumpIfFalse, 5), // if false, jump to else (index 5)
            make_instr(VMOpCode::LoadConst, 1),   // then branch
            make_instr(VMOpCode::Jump, 6),        // jump to end (index 6)
            make_instr(VMOpCode::LoadConst, 99),  // unreachable (skipped by Jump at 2)
            make_instr(VMOpCode::LoadConst, 99),  // unreachable
            make_instr(VMOpCode::LoadConst, 2),   // else branch (jump target from 0)
            make_instr(VMOpCode::Return, 0),      // end (jump target from 2)
            make_instr(VMOpCode::Return, 0),      // DEAD (after previous return)
        ];

        let live = PeepholeOptimizer::mark_live_code(&instrs);

        // All reachable code should be marked
        assert!(live.contains(&0)); // JumpIfFalse
        assert!(live.contains(&1)); // then: LoadConst
        assert!(live.contains(&2)); // Jump to end
        assert!(!live.contains(&3)); // LoadConst - DEAD (skipped by Jump at 2)
        assert!(!live.contains(&4)); // LoadConst - DEAD (skipped by Jump at 2)
        assert!(live.contains(&5)); // else: LoadConst (jump target from 0)
        assert!(live.contains(&6)); // Return (jump target from 2)
        assert!(!live.contains(&7)); // Return after previous return - DEAD
    }

    #[test]
    fn test_integration_jump_chain_and_dead_code() {
        // Jump chains + dead code detection together
        // Jump 1, Jump 2, LoadConst, Return, Nop, Nop
        let instrs = vec![
            make_instr(VMOpCode::Jump, 1),      // Jump chain start
            make_instr(VMOpCode::Jump, 2),      // Jump chain middle
            make_instr(VMOpCode::LoadConst, 0), // Jump chain target
            make_instr(VMOpCode::Return, 0),    // Terminal
            make_instr(VMOpCode::Nop, 0),       // Dead after Return
            make_instr(VMOpCode::Nop, 0),       // Dead after Return
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // After jump resolution: Jump 0 -> 2
        // After dead code: indices 4,5 marked dead
        // After compaction: only indices 0,1,2,3 remain (compacted)
        assert!(optimized.len() >= 3); // At least Jump, LoadConst, Return
        assert_eq!(optimized.last().unwrap().op, VMOpCode::Return);
    }

    #[test]
    fn test_integration_dupop_and_jump_update() {
        // DupTop+PopTop removal + jump offset update
        // LoadConst, DupTop, PopTop, Jump, LoadConst, Return
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0), // Index 0
            make_instr(VMOpCode::DupTop, 0),    // Index 1
            make_instr(VMOpCode::PopTop, 0),    // Index 2
            make_instr(VMOpCode::Jump, 5),      // Index 3, jumps to Return at 5
            make_instr(VMOpCode::LoadConst, 1), // Index 4
            make_instr(VMOpCode::Return, 0),    // Index 5
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // DupTop+PopTop should be folded and removed in compaction
        // Result should be: LoadConst(0), Jump(?), Return
        assert!(optimized.iter().any(|i| i.op == VMOpCode::Jump));

        // Verify Jump target is valid (points to a Return)
        let jump_instr = optimized.iter().find(|i| i.op == VMOpCode::Jump);
        if let Some(j) = jump_instr {
            let target = j.arg as usize;
            assert!(target < optimized.len());
            // Target should point to Return
            assert_eq!(optimized[target].op, VMOpCode::Return);
        }
    }

    #[test]
    fn test_integration_full_realistic_function() {
        // Realistic function with multiple optimization opportunities
        // Simulates: fn foo() { x = 5; if (x > 0) return x else return -x }
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),    // Load 5
            make_instr(VMOpCode::DupTop, 0),       // Dup for store
            make_instr(VMOpCode::StoreLocal, 0),   // Store to x
            make_instr(VMOpCode::DupTop, 0),       // Jump chain to condition
            make_instr(VMOpCode::Jump, 5),         // Jump to condition (jump chain: 3->5->6)
            make_instr(VMOpCode::Jump, 6),         // Jump chain
            make_instr(VMOpCode::LoadLocal, 0),    // Load x for comparison
            make_instr(VMOpCode::LoadConst, 1),    // Load 0
            make_instr(VMOpCode::JumpIfFalse, 12), // if x <= 0, jump to else
            make_instr(VMOpCode::LoadLocal, 0),    // then: return x
            make_instr(VMOpCode::Return, 0),       // Return
            make_instr(VMOpCode::PopTop, 0),       // Dead code after return
            make_instr(VMOpCode::LoadLocal, 0),    // else: load x
            make_instr(VMOpCode::Return, 0),       // Return
        ];

        let original_len = instrs.len();
        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // Verify optimization occurred
        assert!(optimized.len() < original_len); // Should reduce bytecode

        // Verify critical instructions remain
        assert!(optimized.iter().any(|i| i.op == VMOpCode::LoadLocal));
        assert!(optimized.iter().any(|i| i.op == VMOpCode::Return));

        // Jump chains should be resolved
        let jump_instr = optimized.iter().find(|i| i.op == VMOpCode::Jump);
        if let Some(j) = jump_instr {
            assert!(j.arg < optimized.len() as u32); // Valid target
        }
    }

    #[test]
    fn test_edge_case_empty_instructions() {
        let instrs = vec![];
        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);
        assert_eq!(optimized.len(), 0);
    }

    #[test]
    fn test_edge_case_single_instruction() {
        let instrs = vec![make_instr(VMOpCode::Return, 0)];
        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);
        assert_eq!(optimized.len(), 1);
        assert_eq!(optimized[0].op, VMOpCode::Return);
    }

    #[test]
    fn test_edge_case_all_dead_code() {
        // Code after Return with no jumps
        let instrs = vec![
            make_instr(VMOpCode::Return, 0),
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::LoadConst, 1),
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // Only Return should remain
        assert_eq!(optimized.len(), 1);
        assert_eq!(optimized[0].op, VMOpCode::Return);
    }

    #[test]
    fn test_edge_case_jump_to_self() {
        // Jump to self (infinite loop, but shouldn't cause issues)
        let instrs = vec![
            make_instr(VMOpCode::Jump, 0),   // Jump to self
            make_instr(VMOpCode::Return, 0), // Unreachable
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // Both should exist, but second is dead
        assert!(optimized.iter().any(|i| i.op == VMOpCode::Jump));
    }

    #[test]
    fn test_edge_case_multiple_entry_points() {
        // Multiple jump targets (no entry at 0 from jump perspective, but it's the entry)
        let instrs = vec![
            make_instr(VMOpCode::LoadConst, 0),
            make_instr(VMOpCode::JumpIfFalse, 3),
            make_instr(VMOpCode::Jump, 4),
            make_instr(VMOpCode::LoadConst, 1),
            make_instr(VMOpCode::Return, 0),
        ];

        let (optimized, _) = PeepholeOptimizer::optimize(instrs, vec![]);

        // All paths should be live
        assert_eq!(optimized.len(), 5);
    }
}

// FILE: catnip_core/src/pipeline/semantic.rs
//! Standalone semantic analyzer - IR → OpPure validation
//!
//! Port of semantic/analyzer.rs with no PyO3 dependencies.
//! Simple validation without optimizations.

use std::collections::{HashMap, HashSet};

use crate::constants::*;
use crate::ir::{IR, IROpCode};
use crate::semantic::PureOptimizer;
use crate::vm::opcode::{composite_head, composite_params, fn_type_split, split_union_members};

use super::diagnostic::{AnalysisResult, SemanticDiagnostic, SemanticSeverity};

mod types;
use types::{Ty, join_states};

/// Signature of a free function whose binding is provably unique, used for
/// monomorphic inter-procedural argument checking (TH3 step 2).
struct FnSig {
    /// Regular parameters in declaration order, with their resolved `Ty`
    /// (`Top` when unannotated or unresolved).
    params: Vec<(String, Ty)>,
    /// Positional index at which a `*args` variadic begins, if any. Positional
    /// arguments at or after this index are not checked.
    vararg_at: Option<usize>,
}

/// Static signature of a generic union declaration (`union Option[T] { ... }`),
/// used for two things the plain `union_defs` map cannot carry: the arity of the
/// type-parameter list (to validate `Option[int, str]`) and the mapping from a
/// variant's payload fields to type parameters (to bind constructor arguments to
/// type arguments in `infer_type`, e.g. `Option.Some(42)` -> `Option[int]`).
struct UnionSig {
    /// Declared type parameters in order (`[T]` -> `["T"]`). Its length is the
    /// arity; empty for a non-generic union.
    type_params: Vec<String>,
    /// Variant name -> for each payload field (in order), `Some(k)` when the
    /// field's declared type is *exactly* the k-th type parameter (`value: T`),
    /// else `None` (a concrete type such as `int`, an unannotated field, or a
    /// type parameter nested in a composite like `list[T]` -- not substituted in
    /// v1). Only `Some(k)` fields bind a type argument from a constructor call.
    variant_param_fields: HashMap<String, Vec<Option<usize>>>,
    /// Variant name -> its payload fields in source order, as `(name, Ty)`. The
    /// `Ty` is the field's declared type resolved concretely (`Top` when
    /// unannotated, unresolved, or *exactly* a type parameter -- a type-parameter
    /// field is not fixed at construction, so it stays `Top` here and is enforced
    /// at the use-site generic boundary instead). Mirrors `struct_fields`: it
    /// lets a variant constructor `U.A(...)` be argument-checked positionally.
    variant_fields: HashMap<String, Vec<(String, Ty)>>,
}

/// Accumulators for the global binding scan (`collect_unique_fns`). Names that
/// are assigned, bound (parameter/pattern), or declared as a struct are tracked
/// so a call site can only be checked when its target is unambiguous.
#[derive(Default)]
struct ScanAcc {
    /// name -> number of `SetLocals` assignments to it.
    assign_count: HashMap<String, usize>,
    /// name -> number of value uses (a `Ref` that is not a callee/target/binding).
    value_uses: HashMap<String, usize>,
    /// name -> last value assigned to it (only meaningful when `assign_count == 1`).
    last_value: HashMap<String, IR>,
    /// Names bound by a parameter or a match pattern (anti-shadowing).
    bound_names: HashSet<String>,
    /// Names declared as a struct (via `OpStruct`).
    struct_names: HashSet<String>,
}

/// Semantic analyzer standalone
pub struct SemanticAnalyzer {
    /// Valid opcodes (static table)
    valid_opcodes: Vec<IROpCode>,
    /// Optimization passes enabled (host baseline; file pragmas can flip it)
    optimize_enabled: bool,
    /// Tail-call optimization enabled (host baseline; file pragmas can flip it)
    tco_enabled: bool,
    /// Host override for optimization (CLI/env). Wins over file pragmas.
    optimize_override: Option<bool>,
    /// Host override for TCO (CLI/env). Wins over file pragmas.
    tco_override: Option<bool>,
    /// CFG+SSA round-trip enabled. Internal gate only -- no CLI/pragma/config
    /// surface (see `wip/CFG_SSA_REWIRING.md`). Off by default and reachable
    /// solely through `set_cfg_enabled`, because the round-trip is not yet a
    /// true identity (`region.rs` drops code after a `match` and mishandles
    /// loops containing `break`); exposing it to users would let them opt into
    /// silent miscompilation. Kept wired so Phase 2 can validate via tests.
    cfg_enabled: bool,
    /// Known enum definitions: name -> variant names
    enum_defs: HashMap<String, HashSet<String>>,
    /// Known union (ADT) definitions: name -> { variant_name -> has_payload }.
    /// `has_payload = false` means the variant matches as a `pattern_enum`
    /// (`Option.None`); `true` means it requires `pattern_struct`
    /// (`Option.Some{...}`).
    union_defs: HashMap<String, HashMap<String, bool>>,
    /// Static signatures of generic unions (type-parameter arity + per-variant
    /// field-to-parameter mapping), keyed by union name. Populated alongside
    /// `union_defs`; drives generic-annotation arity checking and constructor
    /// argument binding in `infer_type`.
    union_sigs: HashMap<String, UnionSig>,
    /// Known struct definitions: name -> ordered fields `(field name, declared
    /// `Ty`)`. `keys()` is the set of declared struct names (used to infer
    /// `Ty::Struct` on a constructor call, e.g. `Point(1, 2)`). The vector keeps
    /// *all* fields in source order (`Top` for unannotated/unresolved): the
    /// order drives positional constructor-argument checking, while `p.x` is
    /// resolved by name lookup.
    struct_fields: HashMap<String, Vec<(String, Ty)>>,
    /// Signatures of free functions with a provably unique binding, collected by
    /// `collect_unique_fns` before the walk. Used to check call-site arguments
    /// (TH3 step 2). A name absent here is not provably unique -> calls to it are
    /// left unchecked (sound: no false positives).
    fn_sigs: HashMap<String, FnSig>,
    /// Names that are assigned (`SetLocals`) or bound (parameter/pattern)
    /// somewhere in the program. A struct constructor is only checked when its
    /// name is *not* here: otherwise it may be shadowed at the call site.
    shadowed_names: HashSet<String>,
    /// Names that are assigned (`SetLocals`) or bound (parameter/pattern), but
    /// *not* counting plain value uses. A union variant constructor (`U.A(...)`)
    /// reaches its union name through a `GetAttr` target, which is inherently a
    /// value use; that alone must not disqualify it, so it is checked against
    /// this set rather than `shadowed_names` (which also folds in value uses). A
    /// reassignment or a local binding of the name still shadows the union.
    bound_or_assigned_names: HashSet<String>,
    /// Names assigned (`SetLocals`) anywhere in the program. The declared
    /// function-type call check only trusts a name that is NEVER assigned (a
    /// pure parameter): an assigned name can be rebound -- including by a
    /// closure between the visible flow points, which the flow-sensitive
    /// `var_types` cannot see -- and E300 is fatal, so a stale type would be
    /// a false rejection of valid code.
    assigned_names: HashSet<String>,
    /// Variable type bindings (concrete `Ty` only; `Top` = absent, see `types`)
    var_types: HashMap<String, Ty>,
    /// Non-fatal diagnostics collected during analysis
    diagnostics: Vec<SemanticDiagnostic>,
}

impl SemanticAnalyzer {
    /// Create a new analyzer (no optimization passes by default)
    pub fn new() -> Self {
        Self {
            valid_opcodes: Self::all_opcodes(),
            optimize_enabled: false,
            tco_enabled: true,
            optimize_override: None,
            tco_override: None,
            cfg_enabled: false,
            enum_defs: HashMap::new(),
            union_defs: HashMap::new(),
            union_sigs: HashMap::new(),
            struct_fields: HashMap::new(),
            fn_sigs: HashMap::new(),
            shadowed_names: HashSet::new(),
            bound_or_assigned_names: HashSet::new(),
            assigned_names: HashSet::new(),
            var_types: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Create a new analyzer with optimization passes enabled by default
    pub fn with_optimizer() -> Self {
        Self {
            optimize_enabled: true,
            ..Self::new()
        }
    }

    /// Enable or disable tail-call optimization marking (baseline).
    pub fn set_tco_enabled(&mut self, enabled: bool) {
        self.tco_enabled = enabled;
    }

    /// Force TCO on/off regardless of file pragmas (CLI/env override).
    pub fn set_tco_override(&mut self, forced: Option<bool>) {
        self.tco_override = forced;
    }

    /// Force optimization on/off regardless of file pragmas (CLI/env override).
    pub fn set_optimize_override(&mut self, forced: Option<bool>) {
        self.optimize_override = forced;
    }

    /// Enable or disable the CFG+SSA round-trip. Internal gate, no user surface.
    pub fn set_cfg_enabled(&mut self, enabled: bool) {
        self.cfg_enabled = enabled;
    }

    /// Analyze, transform, optimize and validate the IR.
    ///
    /// Execution path: an `Error`-severity diagnostic (today only E300, a
    /// provable type mismatch) is fatal here, so a proven mismatch is refused
    /// before compilation instead of reaching the VM. The lint path keeps the
    /// non-fatal diagnostics via `analyze_full`.
    pub fn analyze(&mut self, ir: &IR) -> Result<IR, String> {
        let result = self.analyze_full(ir)?;
        if let Some(err) = result
            .diagnostics
            .iter()
            .find(|d| d.severity == SemanticSeverity::Error)
        {
            return Err(format!("{}: {}", err.code, err.message));
        }
        Ok(result.ir)
    }

    /// Full analysis returning IR + non-fatal diagnostics
    pub fn analyze_full(&mut self, ir: &IR) -> Result<AnalysisResult, String> {
        self.enum_defs.clear();
        self.union_defs.clear();
        self.union_sigs.clear();
        self.struct_fields.clear();
        self.fn_sigs.clear();
        self.shadowed_names.clear();
        self.bound_or_assigned_names.clear();
        self.assigned_names.clear();
        self.var_types.clear();
        self.diagnostics.clear();

        let transformed = self.transform(ir);
        // Precedence: host override (CLI/env) > file pragma > host baseline
        let (file_tco, file_optimize) = Self::scan_file_pragmas(&transformed);
        let tco = self.tco_override.or(file_tco).unwrap_or(self.tco_enabled);
        let optimize = self
            .optimize_override
            .or(file_optimize)
            .unwrap_or(self.optimize_enabled);
        // CFG has no pragma/override surface: it is an internal gate only.
        let cfg = self.cfg_enabled;

        let tail_marked = if tco {
            Self::mark_tail_calls(&transformed)
        } else {
            transformed
        };
        let optimized = if optimize {
            PureOptimizer::new().optimize(tail_marked)
        } else {
            tail_marked
        };
        // CFG+SSA round-trip (wip/CFG_SSA_REWIRING.md): build → SSA → destruction
        // → reconstruction, no inter-block passes. Meant to be a semantic
        // identity but is not one yet (code after a `match` is dropped, loops
        // with `break` miscompile), which is why the gate has no user surface.
        let optimized = if cfg { Self::cfg_roundtrip(optimized) } else { optimized };
        self.validate(&optimized)?;
        self.collect_unique_fns(&optimized);
        self.check_exhaustiveness(&optimized);
        // TH4 canal A: rewrite Add -> AddInt/AddFloat on proven-typed operands.
        // This is a type specialization enabled by the boundary enforcement, not
        // an algebraic optimization, so it runs regardless of `optimize` (and is
        // therefore exercised by the default test flow). Pure constant pairs are
        // left to the folder and stay Add, which keeps `pragma optimize 0` honest.
        let optimized = rewrite_typed_arith(optimized);
        // FT2-A: enforce declared-callback returns on the caller side.
        let optimized = rewrite_callback_return_checks(optimized, &self.assigned_names);

        Ok(AnalysisResult {
            ir: optimized,
            diagnostics: std::mem::take(&mut self.diagnostics),
        })
    }

    /// Transform the IR: intercept intrinsic calls (type, breakpoint)
    fn transform(&self, ir: &IR) -> IR {
        match ir {
            // Intercept Call nodes for intrinsic functions
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                let func_name = match func.as_ref() {
                    IR::Identifier(n) => Some(n.as_str()),
                    IR::Ref(n, _, _) => Some(n.as_str()),
                    _ => None,
                };
                if let Some(name) = func_name {
                    // typeof(expr) → Op(TypeOf, [expr])
                    if name == "typeof" && args.len() == 1 && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::TypeOf,
                            args: vec![self.transform(&args[0])],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // breakpoint() → Op(Breakpoint, [])
                    if name == "breakpoint" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Breakpoint,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // globals() → Op(Globals, [])
                    if name == "globals" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Globals,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                    // locals() → Op(Locals, [])
                    if name == "locals" && args.is_empty() && kwargs.is_empty() {
                        return IR::Op {
                            opcode: IROpCode::Locals,
                            args: vec![],
                            kwargs: indexmap::IndexMap::new(),
                            tail: false,
                            start_byte: *start_byte,
                            end_byte: *end_byte,
                        };
                    }
                }
                // Not an intrinsic - transform children
                IR::Call {
                    func: Box::new(self.transform(func)),
                    args: args.iter().map(|a| self.transform(a)).collect(),
                    kwargs: kwargs.iter().map(|(k, v)| (k.clone(), self.transform(v))).collect(),
                    tail: false,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Recursively transform all node types
            IR::Program(items) => IR::Program(items.iter().map(|i| self.transform(i)).collect()),
            IR::List(items) => IR::List(items.iter().map(|i| self.transform(i)).collect()),
            IR::Tuple(items) => IR::Tuple(items.iter().map(|i| self.transform(i)).collect()),
            IR::Set(items) => IR::Set(items.iter().map(|i| self.transform(i)).collect()),
            IR::Dict(pairs) => IR::Dict(
                pairs
                    .iter()
                    .map(|(k, v)| (self.transform(k), self.transform(v)))
                    .collect(),
            ),
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => IR::Op {
                opcode: *opcode,
                args: args.iter().map(|a| self.transform(a)).collect(),
                kwargs: kwargs.iter().map(|(k, v)| (k.clone(), self.transform(v))).collect(),
                tail: *tail,
                start_byte: *start_byte,
                end_byte: *end_byte,
            },
            IR::PatternLiteral(v) => IR::PatternLiteral(Box::new(self.transform(v))),
            IR::PatternOr(ps) => IR::PatternOr(ps.iter().map(|p| self.transform(p)).collect()),
            IR::PatternTuple(ps) => IR::PatternTuple(ps.iter().map(|p| self.transform(p)).collect()),
            IR::Slice { start, stop, step } => IR::Slice {
                start: Box::new(self.transform(start)),
                stop: Box::new(self.transform(stop)),
                step: Box::new(self.transform(step)),
            },
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => IR::Broadcast {
                target: target.as_ref().map(|t| Box::new(self.transform(t))),
                operator: Box::new(self.transform(operator)),
                operand: operand.as_ref().map(|o| Box::new(self.transform(o))),
                broadcast_type: broadcast_type.clone(),
            },

            // Leaf nodes - return as-is
            _ => ir.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Tail-call marking
    // -----------------------------------------------------------------------

    /// Mark tail calls for TCO (proper tail calls).
    ///
    /// Inside every lambda body, any call whose target is a plain name
    /// (Ref/Identifier) in tail position gets `tail: true`: self-recursion,
    /// mutual recursion and terminal calls alike. All three runtimes execute
    /// a marked call in O(1) stack: frame reuse in the VMs, trampoline with
    /// scope swap in the AST interpreter (non-Catnip targets are called
    /// directly).
    fn mark_tail_calls(ir: &IR) -> IR {
        Self::mark_tails(ir, false)
    }

    /// Single traversal. `in_tail` is true when `ir` sits in tail position
    /// of the enclosing lambda body. `in_tail` only becomes true at lambda
    /// bodies, so `tail: true` never escapes to top level (a TailCall
    /// leaking outside a trampoline would surface as a value).
    fn mark_tails(ir: &IR, in_tail: bool) -> IR {
        match ir {
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                let nominal = matches!(func.as_ref(), IR::Ref(..) | IR::Identifier(_));
                IR::Call {
                    func: Box::new(Self::mark_tails(func, false)),
                    args: args.iter().map(|a| Self::mark_tails(a, false)).collect(),
                    kwargs: kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::mark_tails(v, false)))
                        .collect(),
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                    tail: in_tail && nominal,
                }
            }

            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                let new_args: Vec<IR> = match opcode {
                    // Lambda body: the only place where tail positions begin.
                    // args: [params, body, ...] -- parameter defaults are not tail.
                    IROpCode::OpLambda | IROpCode::FnDef if args.len() >= 2 => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| Self::mark_tails(a, i == 1))
                        .collect(),

                    // Block: only the last statement inherits tail position
                    IROpCode::OpBlock => {
                        let last = args.len().saturating_sub(1);
                        args.iter()
                            .enumerate()
                            .map(|(i, a)| Self::mark_tails(a, in_tail && i == last))
                            .collect()
                    }

                    // If: args = [branches, else_block] where branches is a
                    // list of (condition, block) pairs. Blocks inherit tail
                    // position, conditions do not.
                    IROpCode::OpIf => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| match (i, a) {
                            (0, IR::List(branches)) | (0, IR::Tuple(branches)) => {
                                let marked: Vec<IR> = branches
                                    .iter()
                                    .map(|branch| match branch {
                                        IR::Tuple(pair) if pair.len() == 2 => IR::Tuple(vec![
                                            Self::mark_tails(&pair[0], false),
                                            Self::mark_tails(&pair[1], in_tail),
                                        ]),
                                        _ => Self::mark_tails(branch, false),
                                    })
                                    .collect();
                                if matches!(a, IR::Tuple(_)) {
                                    IR::Tuple(marked)
                                } else {
                                    IR::List(marked)
                                }
                            }
                            (1, _) => Self::mark_tails(a, in_tail),
                            _ => Self::mark_tails(a, false),
                        })
                        .collect(),

                    // Match: args = [scrutinee, cases] with cases of shape
                    // Tuple([pattern, guard, body]). Bodies inherit tail
                    // position; scrutinee, patterns and guards do not.
                    IROpCode::OpMatch => args
                        .iter()
                        .enumerate()
                        .map(|(i, a)| match (i, a) {
                            (1, IR::Tuple(cases)) | (1, IR::List(cases)) => {
                                let marked: Vec<IR> = cases
                                    .iter()
                                    .map(|case| match case {
                                        IR::Tuple(parts) if parts.len() >= 3 => {
                                            let new_parts: Vec<IR> = parts
                                                .iter()
                                                .enumerate()
                                                .map(|(j, p)| Self::mark_tails(p, in_tail && j == 2))
                                                .collect();
                                            IR::Tuple(new_parts)
                                        }
                                        _ => Self::mark_tails(case, false),
                                    })
                                    .collect();
                                if matches!(a, IR::List(_)) {
                                    IR::List(marked)
                                } else {
                                    IR::Tuple(marked)
                                }
                            }
                            _ => Self::mark_tails(a, false),
                        })
                        .collect(),

                    // Return: inherits in_tail rather than forcing it.
                    // op_return propagates a TailCall as a value; marking
                    // `return f()` inside a loop body would leak the TailCall
                    // object into op_while/op_for which would treat it as a
                    // plain statement value.
                    IROpCode::OpReturn => args.iter().map(|a| Self::mark_tails(a, in_tail)).collect(),

                    // Everything else is not a tail position. Notably:
                    // - OpTry: the handler must stay on the stack
                    // - OpWhile/OpFor: loop bodies re-enter
                    // - And/Or/NullCoalesce: operands feed is_truthy(), a
                    //   TailCall object would be consumed as truthy
                    // We still recurse to reach nested lambda bodies.
                    _ => args.iter().map(|a| Self::mark_tails(a, false)).collect(),
                };
                IR::Op {
                    opcode: *opcode,
                    args: new_args,
                    kwargs: kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), Self::mark_tails(v, false)))
                        .collect(),
                    tail: *tail,
                    start_byte: *start_byte,
                    end_byte: *end_byte,
                }
            }

            // Structural containers: no tail position inside, but nested
            // lambdas must still be reached
            IR::Program(items) => IR::Program(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::List(items) => IR::List(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Tuple(items) => IR::Tuple(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Set(items) => IR::Set(items.iter().map(|i| Self::mark_tails(i, false)).collect()),
            IR::Dict(entries) => IR::Dict(
                entries
                    .iter()
                    .map(|(k, v)| (Self::mark_tails(k, false), Self::mark_tails(v, false)))
                    .collect(),
            ),
            IR::Slice { start, stop, step } => IR::Slice {
                start: Box::new(Self::mark_tails(start, false)),
                stop: Box::new(Self::mark_tails(stop, false)),
                step: Box::new(Self::mark_tails(step, false)),
            },
            IR::Broadcast {
                target,
                operator,
                operand,
                broadcast_type,
            } => IR::Broadcast {
                target: target.as_ref().map(|t| Box::new(Self::mark_tails(t, false))),
                operator: Box::new(Self::mark_tails(operator, false)),
                operand: operand.as_ref().map(|o| Box::new(Self::mark_tails(o, false))),
                broadcast_type: broadcast_type.clone(),
            },

            // Leaves (literals, identifiers, patterns)
            _ => ir.clone(),
        }
    }

    /// Recursively validate the IR
    fn validate(&self, ir: &IR) -> Result<(), String> {
        match ir {
            // Literals sont toujours valides
            IR::Int(_)
            | IR::Float(_)
            | IR::String(_)
            | IR::Bytes(_)
            | IR::Bool(_)
            | IR::None
            | IR::Decimal(_)
            | IR::Imaginary(_) => Ok(()),

            // Identifiers et Refs
            IR::Identifier(_) | IR::Ref(..) => Ok(()),

            // Collections + Program
            IR::Program(items) | IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
                for item in items {
                    self.validate(item)?;
                }
                Ok(())
            }

            IR::Dict(pairs) => {
                for (key, value) in pairs {
                    self.validate(key)?;
                    self.validate(value)?;
                }
                Ok(())
            }

            // Function calls
            IR::Call { func, args, kwargs, .. } => {
                self.validate(func)?;
                for arg in args {
                    self.validate(arg)?;
                }
                for (_, value) in kwargs {
                    self.validate(value)?;
                }
                Ok(())
            }

            // Operations
            IR::Op {
                opcode,
                args,
                kwargs,
                start_byte,
                ..
            } => {
                // Vérifier que l'opcode existe
                if !self.valid_opcodes.contains(opcode) {
                    return Err(format!("Unknown opcode: {:?}", opcode));
                }

                // Validate pragma directives and values
                if *opcode == IROpCode::Pragma && args.len() >= 2 {
                    self.validate_pragma(args, *start_byte)?;
                }

                // Valider les arguments
                for arg in args {
                    self.validate(arg)?;
                }

                // Valider les kwargs
                for (_, value) in kwargs {
                    self.validate(value)?;
                }

                Ok(())
            }

            // Pattern matching
            IR::PatternLiteral(value) => self.validate(value),
            IR::PatternVar(_) => Ok(()),
            IR::PatternWildcard => Ok(()),
            IR::PatternOr(patterns) | IR::PatternTuple(patterns) => {
                for pattern in patterns {
                    self.validate(pattern)?;
                }
                Ok(())
            }
            IR::PatternStruct { .. } => Ok(()),
            IR::PatternEnum { .. } => Ok(()),

            // Slice
            IR::Slice { start, stop, step } => {
                self.validate(start)?;
                self.validate(stop)?;
                self.validate(step)
            }

            // Broadcast
            IR::Broadcast {
                target,
                operator,
                operand,
                ..
            } => {
                if let Some(t) = target {
                    self.validate(t)?;
                }
                self.validate(operator)?;
                if let Some(o) = operand {
                    self.validate(o)?;
                }
                Ok(())
            }
        }
    }

    /// Scan top-level pragma directives that affect this analysis pass.
    /// Returns (tco, optimize); `None` when the file does not set the pragma.
    /// Pragmas are file-scoped and sequential: the last one wins. Invalid
    /// values are ignored here -- `validate_pragma` reports them later with
    /// source positions.
    fn scan_file_pragmas(ir: &IR) -> (Option<bool>, Option<bool>) {
        let stmts = match ir {
            IR::Program(stmts) => stmts.as_slice(),
            other => std::slice::from_ref(other),
        };
        let mut tco = None;
        let mut optimize = None;
        for stmt in stmts {
            if let IR::Op {
                opcode: IROpCode::Pragma,
                args,
                ..
            } = stmt
            {
                let (Some(IR::String(directive)), Some(value)) = (args.first(), args.get(1)) else {
                    continue;
                };
                match (directive.to_lowercase().as_str(), value) {
                    ("tco", IR::Bool(b)) => tco = Some(*b),
                    (PRAGMA_OPTIMIZE, IR::Int(n)) if (0..=OPTIMIZE_MAX).contains(n) => optimize = Some(*n > 0),
                    _ => {}
                }
            }
        }
        (tco, optimize)
    }

    /// CFG+SSA round-trip: IR → CFG → SSA → LICM → DSE → GVN → SSA destruction
    /// → reconstruction (wip/CFG_SSA_REWIRING.md, Phase 3).
    ///
    /// LICM hoists loop-invariant pure defs of `while` loops into a block
    /// guarded by a copy of the (pure) loop condition -- the guard closes the
    /// zero-trip speculation hole, see `ssa_licm`. When it moved anything, the
    /// SSA is rebuilt (instruction indices went stale) before GVN, which then
    /// also sees the hoisted code.
    ///
    /// DSE runs between LICM and GVN on the same SSA: Nop-ing in place shifts
    /// no instruction index, and the two domains are disjoint by construction
    /// -- DSE only eliminates non-op RHS (scalar literals / interned refs)
    /// while GVN only keys op RHS, so no GVN canonical can be an eliminated
    /// def. (A Nop-ed store still enters GVN's `scalars` set through its
    /// intact args; harmless, its value is nobody's operand.) See `ssa_dse`
    /// for the soundness argument: transparent stores only, kill on all
    /// forward paths, calls and faultable ops barrier the window.
    ///
    /// GVN is the inter-block redundancy pass (it subsumes plain syntactic
    /// CSE); redundant pure expressions are materialized by `materialize_gvn`
    /// -- a bare-name alias when the canonical variable is single-def, an
    /// additive snapshot temporary (`__gvnN`) when it is multi-def. No existing
    /// def or use is renamed, so late-bound reads (closures) stay sound; the
    /// versioned-rename path (`destroy_ssa_versioned`) is reserved for a future
    /// lambda-aware consumer (see the `maximal_naming_closure_*` oracle for why
    /// renaming cannot ship as is).
    ///
    /// The known reconstruction holes are closed -- post-`match` code is
    /// preserved, loops whose body breaks/returns are recognized, and
    /// break/continue/return edges stop body reconstruction. The gate stays
    /// internal: `match` round-trips by op-preservation (arms are not rebuilt
    /// from their blocks, so passes never reach inside an arm), and the env
    /// activation channel (`CATNIP_CFG_INTERNAL`) must leave any distributed
    /// binary before ship. Structural verifiers run in the builder/SSA stages
    /// via `debug_assert!`.
    fn cfg_roundtrip(ir: IR) -> IR {
        use crate::cfg::analysis::compute_dominators;
        use crate::cfg::ssa_builder::SSABuilder;
        use crate::cfg::ssa_destruction::destroy_ssa;
        use crate::cfg::ssa_dse::{apply_dse, global_dse};
        use crate::cfg::ssa_gvn::{gvn, materialize_gvn};
        use crate::cfg::ssa_licm::licm;
        use crate::cfg::{IRCFGBuilder, reconstruct_from_cfg};

        let stmts = match ir {
            IR::Program(stmts) => stmts,
            other => vec![other],
        };
        let mut cfg = IRCFGBuilder::new("analyze").build(stmts);
        compute_dominators(&mut cfg);
        let mut ssa = SSABuilder::build(&cfg);
        if licm(&mut cfg, &ssa).hoisted > 0 {
            // licm recomputed dominators; the SSA indices are stale.
            ssa = SSABuilder::build(&cfg);
        }
        let dse_result = global_dse(&cfg, &ssa);
        apply_dse(&mut cfg, &dse_result);
        let gvn_result = gvn(&cfg, &ssa);
        materialize_gvn(&mut cfg, &ssa, &gvn_result);
        destroy_ssa(&mut cfg, &ssa);
        IR::Program(reconstruct_from_cfg(&cfg))
    }

    /// Validate pragma directive name and value.
    /// Errors are prefixed with `@pragma:BYTE ` for downstream position enrichment.
    fn validate_pragma(&self, args: &[IR], start_byte: usize) -> Result<(), String> {
        let directive = match &args[0] {
            IR::String(s) => s.to_lowercase(),
            _ => return Ok(()), // non-string directive caught elsewhere
        };

        let pragma_err = |msg: String| -> Result<(), String> { Err(format!("@pragma:{} {}", start_byte, msg)) };

        if !PRAGMA_DIRECTIVES.contains(&directive.as_str()) {
            let known = PRAGMA_DIRECTIVES.join(", ");
            return pragma_err(format!("Unknown pragma directive: '{}'. Known: {}", directive, known));
        }

        match directive.as_str() {
            d if PRAGMA_BOOL.contains(&d) => match &args[1] {
                IR::Bool(_) => {}
                _ => return pragma_err(format!("Pragma '{}' requires True or False", directive)),
            },
            PRAGMA_OPTIMIZE => match &args[1] {
                IR::Int(n) if *n >= 0 && *n <= OPTIMIZE_MAX => {}
                IR::Int(n) => return pragma_err(format!("Optimization level must be 0-{}, got {}", OPTIMIZE_MAX, n)),
                _ => {
                    return pragma_err(format!(
                        "Pragma '{}' requires an integer 0-{}",
                        PRAGMA_OPTIMIZE, OPTIMIZE_MAX
                    ));
                }
            },
            PRAGMA_JIT => match &args[1] {
                IR::Bool(_) => {}
                IR::String(s) if s == "all" => {}
                _ => return pragma_err(format!("Pragma '{}' requires True, False, or \"all\"", PRAGMA_JIT)),
            },
            PRAGMA_ND_MODE => match &args[1] {
                IR::String(s) if ND_MODE_VALUES.contains(&s.to_lowercase().as_str()) => {}
                IR::String(s) => {
                    return pragma_err(format!(
                        "Unknown ND mode: '{}'. Use ND.sequential, ND.thread, or ND.process",
                        s
                    ));
                }
                _ => {
                    return pragma_err(format!(
                        "Pragma '{}' requires ND.sequential, ND.thread, or ND.process",
                        PRAGMA_ND_MODE
                    ));
                }
            },
            d if PRAGMA_UINT.contains(&d) => match &args[1] {
                IR::Int(n) if *n < 0 => return pragma_err(format!("Pragma '{}' must be non-negative, got {}", d, n)),
                IR::Int(_) => {}
                _ => return pragma_err(format!("Pragma '{}' requires a non-negative integer", d)),
            },
            d if PRAGMA_DEFERRED.contains(&d) => {} // validated elsewhere
            _ => {}                                 // covered by PRAGMA_DIRECTIVES check above
        }
        Ok(())
    }

    /// Return all valid opcodes
    fn all_opcodes() -> Vec<IROpCode> {
        vec![
            IROpCode::Nop,
            IROpCode::OpIf,
            IROpCode::OpWhile,
            IROpCode::OpFor,
            IROpCode::OpMatch,
            IROpCode::OpBlock,
            IROpCode::OpReturn,
            IROpCode::OpBreak,
            IROpCode::OpContinue,
            IROpCode::Call,
            IROpCode::OpLambda,
            IROpCode::FnDef,
            IROpCode::SetLocals,
            IROpCode::GetAttr,
            IROpCode::SetAttr,
            IROpCode::GetItem,
            IROpCode::SetItem,
            IROpCode::Slice,
            IROpCode::Add,
            IROpCode::Sub,
            IROpCode::Mul,
            IROpCode::Div,
            IROpCode::TrueDiv,
            IROpCode::FloorDiv,
            IROpCode::Mod,
            IROpCode::Pow,
            IROpCode::Neg,
            IROpCode::Pos,
            IROpCode::Eq,
            IROpCode::Ne,
            IROpCode::Lt,
            IROpCode::Le,
            IROpCode::Gt,
            IROpCode::Ge,
            IROpCode::And,
            IROpCode::Or,
            IROpCode::Not,
            IROpCode::BAnd,
            IROpCode::BOr,
            IROpCode::BXor,
            IROpCode::BNot,
            IROpCode::LShift,
            IROpCode::RShift,
            IROpCode::Broadcast,
            IROpCode::ListLiteral,
            IROpCode::TupleLiteral,
            IROpCode::SetLiteral,
            IROpCode::DictLiteral,
            IROpCode::Push,
            IROpCode::Pop,
            IROpCode::PushPeek,
            IROpCode::Fstring,
            IROpCode::Pragma,
            IROpCode::NdRecursion,
            IROpCode::NdMap,
            IROpCode::NdEmptyTopos,
            IROpCode::Breakpoint,
            IROpCode::OpStruct,
            IROpCode::TraitDef,
            IROpCode::In,
            IROpCode::NotIn,
            IROpCode::Is,
            IROpCode::IsNot,
            IROpCode::NullCoalesce,
            IROpCode::TypeOf,
            IROpCode::Globals,
            IROpCode::Locals,
            IROpCode::EnumDef,
            IROpCode::UnionDef,
            IROpCode::OpTry,
            IROpCode::OpRaise,
            IROpCode::ExcInfo,
        ]
    }

    // -----------------------------------------------------------------------
    // Exhaustiveness checking (I103)
    // -----------------------------------------------------------------------

    /// Walk the IR collecting enum defs, tracking variable types,
    /// and checking match exhaustiveness.
    fn check_exhaustiveness(&mut self, ir: &IR) {
        match ir {
            IR::Program(items) => {
                for item in items {
                    self.check_exhaustiveness(item);
                }
            }

            // Register enum definitions
            IR::Op { opcode, args, .. } if *opcode == IROpCode::EnumDef => {
                if let (Some(IR::String(name)), Some(IR::Tuple(variants))) = (args.first(), args.get(1)) {
                    let variant_set: HashSet<String> = variants
                        .iter()
                        .filter_map(|v| if let IR::String(s) = v { Some(s.clone()) } else { None })
                        .collect();
                    self.enum_defs.insert(name.clone(), variant_set);
                }
            }

            // Register union definitions
            // IR layout: UnionDef(name, type_params_list, variants_list[, methods_list])
            // where each variant is Tuple([variant_name, fields_list]).
            // A variant has a payload iff its fields_list is non-empty.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::UnionDef => {
                if let (Some(IR::String(name)), Some(IR::List(variants))) = (args.first(), args.get(2)) {
                    let variants_map: HashMap<String, bool> = variants
                        .iter()
                        .filter_map(|v| match v {
                            IR::Tuple(parts) if parts.len() >= 2 => {
                                let vname = match parts.first() {
                                    Some(IR::String(s)) => s.clone(),
                                    _ => return None,
                                };
                                let has_payload = match &parts[1] {
                                    IR::List(fields) => !fields.is_empty(),
                                    _ => false,
                                };
                                Some((vname, has_payload))
                            }
                            _ => None,
                        })
                        .collect();
                    self.union_defs.insert(name.clone(), variants_map);

                    // Static signature: type-parameter arity (from args[1]) plus,
                    // per variant, which payload fields are exactly a type parameter
                    // (`value: T` -> `Some(k)`). Drives generic-annotation arity
                    // checking and constructor-argument binding in `infer_type`.
                    let type_params: Vec<String> = match args.get(1) {
                        Some(IR::List(tps)) => tps
                            .iter()
                            .filter_map(|t| if let IR::String(s) = t { Some(s.clone()) } else { None })
                            .collect(),
                        _ => Vec::new(),
                    };
                    let mut variant_param_fields: HashMap<String, Vec<Option<usize>>> = HashMap::new();
                    let mut variant_fields: HashMap<String, Vec<(String, Ty)>> = HashMap::new();
                    for v in variants {
                        let IR::Tuple(parts) = v else { continue };
                        let Some(IR::String(vname)) = parts.first() else {
                            continue;
                        };
                        let fields: &[IR] = match parts.get(1) {
                            Some(IR::List(fs)) => fs,
                            _ => &[],
                        };
                        let mut slots: Vec<Option<usize>> = Vec::with_capacity(fields.len());
                        let mut typed: Vec<(String, Ty)> = Vec::with_capacity(fields.len());
                        for f in fields {
                            // Field is `Tuple([name, type_or_none])`. Keep exactly one
                            // entry per field position: both `variant_param_fields` and
                            // `variant_fields` are consumed positionally against the
                            // constructor's arguments, so skipping a field would
                            // misalign every field after it. A malformed field (never
                            // emitted by the transformer) degrades to an unnamed `Top`
                            // slot rather than being dropped.
                            let IR::Tuple(pair) = f else {
                                slots.push(None);
                                typed.push((String::new(), Ty::Top));
                                continue;
                            };
                            let fname = match pair.first() {
                                Some(IR::String(s)) => s.clone(),
                                _ => String::new(),
                            };
                            // `Some(k)` iff the type text is exactly type_params[k].
                            let slot = match pair.get(1) {
                                Some(IR::String(text)) => type_params.iter().position(|p| p == text.trim()),
                                _ => None,
                            };
                            // A type-parameter field is not fixed at construction:
                            // keep it `Top` (deferred to the generic boundary). A
                            // concrete field resolves to its declared `Ty`.
                            let ty = if slot.is_some() {
                                Ty::Top
                            } else {
                                self.resolve_annotation_ir(pair.get(1))
                            };
                            slots.push(slot);
                            typed.push((fname, ty));
                        }
                        variant_param_fields.insert(vname.clone(), slots);
                        variant_fields.insert(vname.clone(), typed);
                    }
                    self.union_sigs.insert(
                        name.clone(),
                        UnionSig {
                            type_params,
                            variant_param_fields,
                            variant_fields,
                        },
                    );

                    // Walk method bodies. Same scope isolation as the
                    // OpLambda arm, except `self` is typed as this union:
                    // a method receives whichever variant it is called on,
                    // so `match self` gets precise I103 reporting.
                    if let Some(IR::List(methods) | IR::Tuple(methods)) = args.get(3) {
                        let union_name = name.clone();
                        for m in methods {
                            let parts = match m {
                                IR::Tuple(parts) | IR::List(parts) => parts,
                                _ => continue,
                            };
                            let Some(IR::Op {
                                opcode: l_opcode,
                                args: l_args,
                                start_byte: l_start,
                                end_byte: l_end,
                                ..
                            }) = parts.get(1)
                            else {
                                continue;
                            };
                            if *l_opcode != IROpCode::OpLambda {
                                continue;
                            }
                            let saved_vars = self.var_types.clone();
                            let saved_enums = self.enum_defs.clone();
                            let saved_unions = self.union_defs.clone();
                            if let Some(IR::Tuple(params) | IR::List(params)) = l_args.first() {
                                self.bind_params(params, *l_start, *l_end);
                            }
                            self.var_types
                                .insert("self".to_string(), Ty::Union(union_name.clone(), Vec::new()));
                            for arg in l_args {
                                self.check_exhaustiveness(arg);
                            }
                            self.check_return_type(l_args, *l_start, *l_end);
                            self.var_types = saved_vars;
                            self.enum_defs = saved_enums;
                            self.union_defs = saved_unions;
                        }
                    }
                }
            }

            // Register struct definitions: name + field types (for constructor
            // and field-access inference), and check field defaults (TH2-A).
            IR::Op {
                opcode,
                args,
                start_byte,
                end_byte,
                ..
            } if *opcode == IROpCode::OpStruct => {
                if let Some(IR::String(name)) = args.first() {
                    let mut fields_ordered: Vec<(String, Ty)> = Vec::new();
                    if let Some(IR::Tuple(fields)) = args.get(1) {
                        for field in fields {
                            // Field tuple: [name, has_default, default, type]
                            let IR::Tuple(parts) = field else { continue };
                            let Some(IR::String(fname)) = parts.first() else {
                                continue;
                            };
                            let declared = self.resolve_annotation_ir(parts.get(3));
                            self.check_annotation_arity(
                                &declared,
                                *start_byte,
                                *end_byte,
                                &format!("struct field '{}'", fname),
                            );
                            // E300: a field whose default provably mismatches its
                            // annotation. `has_default` gates it, so an explicit
                            // `= None` default is checked (unlike params).
                            if parts.get(1) == Some(&IR::Bool(true)) {
                                if let Some(default) = parts.get(2) {
                                    self.check_value_against(
                                        default,
                                        &declared,
                                        *start_byte,
                                        *end_byte,
                                        &format!("struct field '{}'", fname),
                                    );
                                }
                            }
                            // Keep every field in source order (`Top` for
                            // unresolved) so the constructor can be checked
                            // positionally; `p.x` looks it up by name.
                            fields_ordered.push((fname.clone(), declared));
                        }
                    }
                    self.struct_fields.insert(name.clone(), fields_ordered);
                }
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
            }

            // Track variable types from assignments
            IR::Op { opcode, args, .. } if *opcode == IROpCode::SetLocals => {
                if args.len() >= 2 {
                    if let Some(name) = Self::extract_single_assign_name(&args[0]) {
                        // Shadow: invalidate any enum/union def with the same name
                        self.enum_defs.remove(&name);
                        self.union_defs.remove(&name);
                        if let Some(ty) = self.infer_type(&args[1]) {
                            self.var_types.insert(name, ty);
                        } else {
                            self.var_types.remove(&name);
                        }
                    }
                }
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
            }

            // Exception handlers: the `except E => name` binding shadows any
            // outer binding of the same name inside its handler block, like a
            // pattern variable (the outer type must not be read there). The
            // binding is handler-local; the walk of each handler starts from
            // the current state minus the binding and is restored after (the
            // conservative pre-join behavior this walker already had for try).
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpTry => {
                if let Some(body) = args.first() {
                    self.check_exhaustiveness(body);
                }
                if let Some(IR::List(handlers) | IR::Tuple(handlers)) = args.get(1) {
                    for h in handlers {
                        let saved_vars = self.var_types.clone();
                        if let IR::Tuple(parts) = h {
                            if let Some(IR::String(binding)) = parts.get(1) {
                                self.var_types.remove(binding);
                            }
                        }
                        self.check_exhaustiveness(h);
                        self.var_types = saved_vars;
                    }
                }
                for arg in args.iter().skip(2) {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
            }

            // Check match expressions. Arms are mutually exclusive branches,
            // handled like `if` arms: each walks from the entry state with its
            // pattern-bound names invalidated (a pattern variable shadows any
            // outer binding of the same name -- an outer `f: (int) -> int`
            // shadowed by a pattern var must not be call-checked inside the
            // arm), and the per-arm exit states are JOINED so an assignment
            // that survives every arm flows out (discarding them would
            // reintroduce the restored-entry-state hole the `if` arm closed).
            // The entry state is not a join input: a match where no arm
            // matches raises MatchFail, so control does not continue. Pattern
            // bindings are arm-local and are dropped from each exit before
            // the join. An OpMatch is always `[scrutinee, Tuple(cases)]`.
            IR::Op {
                opcode,
                args,
                start_byte,
                end_byte,
                ..
            } if *opcode == IROpCode::OpMatch => {
                self.check_match_node(args, *start_byte, *end_byte);
                if let Some(scrutinee) = args.first() {
                    self.check_exhaustiveness(scrutinee);
                }
                if let Some(IR::Tuple(cases) | IR::List(cases)) = args.get(1) {
                    let saved_vars = self.var_types.clone();
                    let mut exits: Vec<HashMap<String, Ty>> = Vec::with_capacity(cases.len());
                    for case in cases {
                        self.var_types = saved_vars.clone();
                        let mut bound = HashSet::new();
                        if let IR::Tuple(parts) = case {
                            if let Some(pattern) = parts.first() {
                                pattern_binding_names(pattern, &mut bound);
                            }
                        }
                        for name in &bound {
                            self.var_types.remove(name);
                        }
                        self.check_exhaustiveness(case);
                        let mut exit = std::mem::take(&mut self.var_types);
                        for name in &bound {
                            exit.remove(name);
                        }
                        exits.push(exit);
                    }
                    self.var_types = if exits.is_empty() {
                        saved_vars
                    } else {
                        join_states(&exits)
                    };
                }
            }

            // Scope isolation for lambdas/functions: save/restore var_types + enum/union defs.
            // Parameters are bound into var_types from their annotations (or cleared
            // when untyped) so the body sees declared types; the return annotation is
            // checked against the inferred body result type.
            IR::Op {
                opcode,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } if *opcode == IROpCode::OpLambda || *opcode == IROpCode::FnDef => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                let saved_unions = self.union_defs.clone();
                if let Some(IR::Tuple(params) | IR::List(params)) = args.first() {
                    self.bind_params(params, *start_byte, *end_byte);
                }
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
                // Body locals are now visible in var_types; check the return type
                // before restoring the outer scope.
                self.check_return_type(args, *start_byte, *end_byte);
                self.var_types = saved_vars;
                self.enum_defs = saved_enums;
                self.union_defs = saved_unions;
            }

            // Control-flow join point. Each branch is analyzed from the entry
            // state (inter-branch isolation: an assignment in one branch must
            // not leak into a sibling). After the construct, the per-branch exit
            // states are JOINED instead of discarded, so a type that survives
            // every path is kept and any divergence widens to `Top` (= dropped).
            // This closes the hole where a reassignment inside a branch was
            // silently forgotten (the old code restored the entry state).
            //
            // Branch enumeration. For an `if/elif/else`, `args[0]` is the Tuple
            // of `[cond, block]` pairs and `args[1]` is the optional else: each
            // pair is one mutually-exclusive branch, so it must be reset to the
            // entry state, not chained after its predecessor (a reassignment in
            // an earlier `elif` body must not leak into a later one). Treating
            // the whole pair-Tuple as a single branch (the previous behavior)
            // let the pairs leak sequentially, masking widening on later arms.
            // For `while`/`for`, the top-level args are the branches; the
            // always-run header parts (condition, `for` target/iterable) leave
            // the state ~unchanged, so joining their exit states is harmless.
            // The entry state is added as an extra branch only when a path can
            // skip every body -- a `while`/`for` that runs zero times, or an
            // `if` with no else -- otherwise an `if/else` always takes one arm.
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpIf || *opcode == IROpCode::OpWhile || *opcode == IROpCode::OpFor => {
                let saved_vars = self.var_types.clone();
                let saved_enums = self.enum_defs.clone();
                let saved_unions = self.union_defs.clone();

                let branches: Vec<&IR> = if *opcode == IROpCode::OpIf {
                    let mut v: Vec<&IR> = Vec::new();
                    match args.first() {
                        Some(IR::Tuple(pairs) | IR::List(pairs)) => v.extend(pairs.iter()),
                        Some(other) => v.push(other),
                        None => {}
                    }
                    if let Some(else_node) = args.get(1) {
                        v.push(else_node);
                    }
                    v
                } else {
                    args.iter().collect()
                };

                // A `for` target is a binding, not a SetLocals: its name(s)
                // shadow any outer binding inside the body, so the outer type
                // must not survive there (an outer `cb: (int) -> int` shadowed
                // by `for cb in ...` was still call-checked in the body). The
                // target names are invalidated for every branch (the iterable
                // evaluates before the first binding, but losing a type there
                // is conservative, never a false rejection) and dropped from
                // the exits (the binding is loop-local).
                let mut for_bound: HashSet<String> = HashSet::new();
                if *opcode == IROpCode::OpFor {
                    if let Some(target) = args.first() {
                        binding_target_names(target, &mut for_bound);
                    }
                }
                let mut exits: Vec<HashMap<String, Ty>> = Vec::with_capacity(branches.len() + 1);
                for branch in branches {
                    self.var_types = saved_vars.clone();
                    self.enum_defs = saved_enums.clone();
                    self.union_defs = saved_unions.clone();
                    for name in &for_bound {
                        self.var_types.remove(name);
                    }
                    self.check_exhaustiveness(branch);
                    let mut exit = std::mem::take(&mut self.var_types);
                    for name in &for_bound {
                        exit.remove(name);
                    }
                    exits.push(exit);
                }
                for (_, v) in kwargs {
                    self.var_types = saved_vars.clone();
                    self.enum_defs = saved_enums.clone();
                    self.union_defs = saved_unions.clone();
                    self.check_exhaustiveness(v);
                }

                let has_skip_path = *opcode != IROpCode::OpIf || args.len() < 2;
                if has_skip_path {
                    exits.push(saved_vars.clone());
                }

                // Variable types: join the branch exits. Type definitions stay
                // scoped to the entry (a type declared inside a branch does not
                // escape the construct), so they are restored, not joined.
                self.var_types = if exits.is_empty() {
                    saved_vars
                } else {
                    join_states(&exits)
                };
                self.enum_defs = saved_enums;
                self.union_defs = saved_unions;
            }

            // Recurse into other node types
            IR::Op { args, kwargs, .. } => {
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
            }
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                ..
            } => {
                self.check_call_site(func, args, kwargs, *start_byte, *end_byte);
                self.check_exhaustiveness(func);
                for arg in args {
                    self.check_exhaustiveness(arg);
                }
                for (_, v) in kwargs {
                    self.check_exhaustiveness(v);
                }
            }
            IR::List(items) | IR::Tuple(items) | IR::Set(items) => {
                for item in items {
                    self.check_exhaustiveness(item);
                }
            }
            IR::Dict(pairs) => {
                for (k, v) in pairs {
                    self.check_exhaustiveness(k);
                    self.check_exhaustiveness(v);
                }
            }
            _ => {}
        }
    }

    /// Resolve an annotation type-name to a lattice `Ty`. Primitives, the
    /// `list`/`set`/`dict`/`tuple` composite constructors, known nominals
    /// (enum/union/struct, by walk-order visibility), and generic unions
    /// (`Option[int]` -> `Union("Option", [Int])`, by declared arity) resolve to a
    /// concrete type; anything else -- an unknown name, or a bracketed head that is
    /// not a declared union -- widens to `Ty::Top`.
    fn resolve_annotation(&self, text: &str) -> Ty {
        // Type union (`int | str`, `Point | None`): resolve each member. Any
        // unresolved member (unmodeled composite, unknown) makes the whole union
        // inert (`Top`), never partially enforced. Members are deduplicated; a
        // single surviving member degenerates to that member. The split is
        // bracket-depth aware so a `|` inside a composite (`dict[int | str, V]`)
        // is not mistaken for a union separator.
        let members = split_union_members(text);
        if members.len() > 1 {
            let mut tys: Vec<Ty> = Vec::new();
            for part in &members {
                let ty = self.resolve_atom(part);
                if !ty.is_concrete() {
                    return Ty::Top;
                }
                if !tys.contains(&ty) {
                    tys.push(ty);
                }
            }
            return match tys.len() {
                0 => Ty::Top,
                1 => tys.into_iter().next().unwrap(),
                _ => Ty::OneOf(tys),
            };
        }
        self.resolve_atom(members[0])
    }

    /// Resolve a single (non-union) annotation atom to a lattice `Ty`.
    fn resolve_atom(&self, text: &str) -> Ty {
        let text = text.trim();
        // Function type (`(int, str) -> bool`): parameters and return resolve
        // recursively (each may be a union, a composite, or another function
        // type -- the return absorbs to the right, mirroring the grammar).
        // Tested first: the leading `(` cannot start any other atom.
        if let Some((params, ret)) = fn_type_split(text) {
            let param_tys: Vec<Ty> = params.iter().map(|p| self.resolve_annotation(p)).collect();
            return Ty::Fn(param_tys, Box::new(self.resolve_annotation(ret)));
        }
        // Composite constructors (`list`/`set`/`dict`): resolve the type
        // parameters (element, or key/value) recursively; an absent parameter is
        // `Top`. A parameter may itself be a union (`dict[int | str, V]`) or a
        // nested composite (`list[list[int]]`), so it goes through
        // `resolve_annotation`. `list`/`set` are homogeneous (one element param).
        if let Some(head) = composite_head(text) {
            let params = composite_params(text);
            let elem = || params.first().map_or(Ty::Top, |p| self.resolve_annotation(p));
            return match head {
                "list" => Ty::List(Box::new(elem())),
                "set" => Ty::Set(Box::new(elem())),
                "dict" => {
                    let key = params.first().map_or(Ty::Top, |p| self.resolve_annotation(p));
                    let val = params.get(1).map_or(Ty::Top, |p| self.resolve_annotation(p));
                    Ty::Dict(Box::new(key), Box::new(val))
                }
                // Positional: one resolved type per parameter, in order. A bare
                // `tuple` (no params, or empty `tuple[]`) has an unknown arity
                // (`None`); only a parameterized annotation carries a known arity.
                "tuple" if params.is_empty() => Ty::Tuple(None),
                "tuple" => Ty::Tuple(Some(params.iter().map(|p| self.resolve_annotation(p)).collect())),
                // `composite_head` returns only list/set/dict/tuple today; a future
                // head it gains must add its arm here. Stay inert (`Top`) rather
                // than miscompile it as a dict.
                _ => Ty::Top,
            };
        }
        // Generic nominal union: `Option[int]`, `Result[T, E]`. The head before
        // `[` must name a declared union; each argument is resolved recursively
        // (an argument may itself be a union or a composite). A bracketed head
        // that is not a known union falls through to `Top` (unmodeled generic).
        // Arity is not validated here (this resolver is pure, emits no
        // diagnostic) but by `check_annotation_arity` at the declaration site.
        if text.ends_with(']') {
            if let Some(open) = text.find('[') {
                let head = text[..open].trim();
                if self.union_defs.contains_key(head) {
                    let args: Vec<Ty> = composite_params(text)
                        .iter()
                        .map(|p| self.resolve_annotation(p))
                        .collect();
                    return Ty::Union(head.to_string(), args);
                }
            }
        }
        match text {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "str" => Ty::Str,
            "bool" => Ty::Bool,
            "None" => Ty::NoneT,
            _ if self.enum_defs.contains_key(text) => Ty::Enum(text.to_string()),
            _ if self.union_defs.contains_key(text) => Ty::Union(text.to_string(), Vec::new()),
            _ if self.struct_fields.contains_key(text) => Ty::Struct(text.to_string()),
            _ => Ty::Top,
        }
    }

    /// Resolve an annotation slot (`IR::String(text)`, or `IR::None`/absent) to
    /// a `Ty`. `Ty::Top` means "no usable annotation".
    fn resolve_annotation_ir(&self, slot: Option<&IR>) -> Ty {
        match slot {
            Some(IR::String(text)) => self.resolve_annotation(text),
            _ => Ty::Top,
        }
    }

    /// Bind lambda/function parameters into `var_types` for the body scope.
    /// Annotated params get their declared `Ty`; unannotated or unresolved
    /// params are cleared so an outer binding of the same name does not leak in.
    /// A parameter name also shadows any enum/union namespace of the same name
    /// (otherwise `infer_type` would treat `(Option) => match Option.None` as a
    /// union scrutinee). Reports a provable default/annotation conflict (TH2-A).
    fn bind_params(&mut self, params: &[IR], start_byte: usize, end_byte: usize) {
        for param in params {
            let IR::Tuple(pair) = param else { continue };
            let Some(IR::String(name)) = pair.first() else { continue };
            self.enum_defs.remove(name);
            self.union_defs.remove(name);
            // Regular param: [name, default, type]. Variadic: ["*", name] (no type slot).
            let declared = self.resolve_annotation_ir(pair.get(2));
            self.check_annotation_arity(&declared, start_byte, end_byte, &format!("parameter '{}'", name));
            // E300: a literal/inferred default that provably mismatches the
            // annotation. An absent default is encoded as `IR::None`, which is
            // indistinguishable from an explicit `= None`, so we skip `None` to
            // avoid falsely flagging `(x: int)` as `int` vs `None`.
            if let Some(default) = pair.get(1) {
                if !matches!(default, IR::None) {
                    self.check_value_against(
                        default,
                        &declared,
                        start_byte,
                        end_byte,
                        &format!("parameter '{}'", name),
                    );
                }
            }
            if declared.is_concrete() {
                self.var_types.insert(name.clone(), declared);
            } else {
                self.var_types.remove(name);
            }
        }
    }

    /// Emit E300 when `value` is provably of a different concrete type than the
    /// declared `Ty` at an annotated site (TH2-A: report only what is provable).
    fn check_value_against(&mut self, value: &IR, declared: &Ty, start_byte: usize, end_byte: usize, site: &str) {
        if !declared.is_concrete() {
            return;
        }
        if let Some(found) = self.infer_type(value) {
            let covariant = Self::is_composite_literal(value);
            if found.is_concrete() && !declared.accepts_value(&found, covariant) {
                self.diagnostics.push(SemanticDiagnostic {
                    code: "E300".to_string(),
                    message: format!(
                        "Type mismatch: {} declared '{}' but value has type '{}'",
                        site, declared, found
                    ),
                    severity: SemanticSeverity::Error,
                    start_byte,
                    end_byte,
                });
            }
        }
    }

    /// Check a lambda's declared return type (`OpLambda` args[2]) against the
    /// inferred result type of its body. Conservative: reports only a concrete
    /// mismatch (TH2-A/TH6).
    fn check_return_type(&mut self, lambda_args: &[IR], start_byte: usize, end_byte: usize) {
        let (Some(IR::String(ret)), Some(body)) = (lambda_args.get(2), lambda_args.get(1)) else {
            return;
        };
        let declared = self.resolve_annotation(ret);
        self.check_annotation_arity(&declared, start_byte, end_byte, "return type");
        if !declared.is_concrete() {
            return;
        }
        self.check_result_positions(body, &declared, start_byte, end_byte);
    }

    /// True if `expr` is a composite literal (`[...]`/`{...}`, or `list(...)`/
    /// `set(...)`/`tuple(...)`/`dict(...)` which share these opcodes) -- a freshly
    /// built container, so its type parameters are checked covariantly. Any other
    /// expression (a typed variable, a call result) is an already-typed value,
    /// checked invariantly.
    fn is_composite_literal(expr: &IR) -> bool {
        matches!(
            expr,
            IR::Op { opcode, .. }
                if *opcode == IROpCode::ListLiteral
                    || *opcode == IROpCode::SetLiteral
                    || *opcode == IROpCode::TupleLiteral
                    || *opcode == IROpCode::DictLiteral
        )
    }

    /// Check every return position of a function body against the declared
    /// return type. Descends through the tail of a block, both arms of an `if`,
    /// every case body of a `match`, and an explicit `return`, so a concrete
    /// mismatch in *any* tail branch is caught -- not only when the body is a
    /// single leaf expression (a `match`/`if` tail previously inferred to `Top`
    /// and was silently accepted). Each leaf is checked independently and
    /// conservatively (TH2-A: report only what is provable): a branch whose type
    /// can't be inferred defers to the runtime boundary, and `OneOf`/numeric-tower
    /// assignability means a branch producing any member of a union return
    /// (`LogEntry | None`) is accepted. A missing `else` is the implicit-nil path
    /// and is not flagged, to avoid a fatal false positive on `if c { x }`.
    fn check_result_positions(&mut self, expr: &IR, declared: &Ty, start_byte: usize, end_byte: usize) {
        match expr {
            // Block: only the tail statement is in return position.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpBlock => {
                if let Some(tail) = args.last() {
                    self.check_result_positions(tail, declared, start_byte, end_byte);
                }
            }
            // Explicit `return e`: `e` is the result.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpReturn => {
                if let Some(value) = args.first() {
                    self.check_result_positions(value, declared, start_byte, end_byte);
                }
            }
            // If: args = [branches, else?] where `branches` is a list of
            // `(condition, block)` pairs. Each block and the else are in return
            // position; the conditions are not.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpIf => {
                if let Some(IR::List(branches) | IR::Tuple(branches)) = args.first() {
                    for branch in branches {
                        if let IR::Tuple(pair) = branch {
                            if let Some(block) = pair.get(1) {
                                self.check_result_positions(block, declared, start_byte, end_byte);
                            }
                        }
                    }
                }
                if let Some(else_block) = args.get(1) {
                    self.check_result_positions(else_block, declared, start_byte, end_byte);
                }
            }
            // Match: args = [scrutinee, cases] with cases of shape
            // `Tuple([pattern, guard, body])`. Each case body is in return position.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpMatch => {
                if let Some(IR::Tuple(cases) | IR::List(cases)) = args.get(1) {
                    for case in cases {
                        if let IR::Tuple(parts) = case {
                            if let Some(body) = parts.get(2) {
                                self.check_result_positions(body, declared, start_byte, end_byte);
                            }
                        }
                    }
                }
            }
            // Leaf: infer its type and check against the declared return type.
            leaf => {
                let covariant = Self::is_composite_literal(leaf);
                if let Some(found) = self.infer_type(leaf) {
                    if found.is_concrete() && !declared.accepts_value(&found, covariant) {
                        self.diagnostics.push(SemanticDiagnostic {
                            code: "E300".to_string(),
                            message: format!(
                                "Type mismatch: function declared return type '{}' but body produces '{}'",
                                declared, found
                            ),
                            severity: SemanticSeverity::Error,
                            start_byte,
                            end_byte,
                        });
                    }
                }
            }
        }
    }

    /// Infer the type of an expression on the flat lattice (`types::Ty`).
    ///
    /// Returns `None` for anything that lands on `Top` (unknown / not modeled):
    /// the caller never stores `Top`, it just leaves the binding absent.
    fn infer_type(&self, expr: &IR) -> Option<Ty> {
        match expr {
            // `Color.red`, `Option.None` -- attribute access on a known type;
            // `p.x` -- field access on a variable of a known struct type.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::GetAttr => {
                if let Some(IR::Ref(name, _, _)) = args.first() {
                    if self.enum_defs.contains_key(name) {
                        return Some(Ty::Enum(name.clone()));
                    }
                    if self.union_defs.contains_key(name) {
                        return Some(Ty::Union(name.clone(), Vec::new()));
                    }
                    if let (Some(Ty::Struct(sname)), Some(IR::String(field))) = (self.var_types.get(name), args.get(1))
                    {
                        return self
                            .struct_fields
                            .get(sname)
                            .and_then(|fields| fields.iter().find(|(n, _)| n == field))
                            .map(|(_, ty)| ty.clone())
                            .filter(Ty::is_concrete);
                    }
                }
                None
            }
            // `Option.Some(42)` -- union variant constructor; `Point(1, 2)` --
            // struct constructor (a bare `Ref` to a declared struct); `cb(x)`
            // -- a call through a variable of a declared function type carries
            // that type's return (the TH3 payoff: inference continues through
            // callbacks with zero CFA).
            IR::Call {
                func, args: call_args, ..
            } => match func.as_ref() {
                IR::Op { opcode, args, .. } if *opcode == IROpCode::GetAttr => {
                    if let Some(IR::Ref(name, _, _)) = args.first() {
                        if self.union_defs.contains_key(name) {
                            // Extended inference: bind the constructor's argument
                            // types to the union's type parameters (`Option.Some(42)`
                            // -> `Option[int]`). The variant name is the GetAttr
                            // attribute (`args[1]`).
                            let variant = match args.get(1) {
                                Some(IR::String(v)) => Some(v.as_str()),
                                _ => None,
                            };
                            return Some(Ty::Union(name.clone(), self.infer_union_args(name, variant, call_args)));
                        }
                    }
                    None
                }
                IR::Ref(name, _, _) if self.struct_fields.contains_key(name) => Some(Ty::Struct(name.clone())),
                IR::Ref(name, _, _) => match self.var_types.get(name) {
                    // Same trust rule as the FT call check: only a name that
                    // is never assigned anywhere (a pure parameter) keeps its
                    // declared return -- an assigned name can be rebound by a
                    // closure between the visible flow points, and a stale
                    // return type rejects valid code (E300 is fatal).
                    Some(Ty::Fn(_, ret)) if !self.assigned_names.contains(name) => {
                        Some(ret.as_ref().clone()).filter(Ty::is_concrete)
                    }
                    _ => None,
                },
                _ => None,
            },
            // A lambda literal infers to its function type: one parameter type
            // per declared parameter (annotation, or `Top` when unannotated),
            // and the annotated return (`Top` when absent -- the body is not
            // inferred in v1; a declared return is already enforced against the
            // body by `check_return_type`). A variadic parameter (2-tuple
            // `("*", name)`) OR a defaulted parameter makes the arity open
            // (callable below the declared count), which `Ty::Fn` cannot
            // represent in v1: no function type is produced, the runtime
            // boundary (which reads the real defaults) decides. Concluding on
            // the raw parameter count rejected valid programs -- E300 is fatal.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpLambda => {
                let params: Vec<Ty> = match args.first() {
                    Some(IR::Tuple(ps) | IR::List(ps)) => {
                        let mut tys = Vec::with_capacity(ps.len());
                        for p in ps {
                            match p {
                                IR::Tuple(items) if items.len() >= 3 => {
                                    // items[1] is the default slot: IR::None
                                    // encodes "no default" (an explicit
                                    // `= None` is indistinguishable -- both
                                    // widen the arity, so both bail).
                                    if !matches!(items.get(1), Some(IR::None)) {
                                        return None;
                                    }
                                    tys.push(self.resolve_annotation_ir(items.get(2)));
                                }
                                _ => return None,
                            }
                        }
                        tys
                    }
                    _ => Vec::new(),
                };
                Some(Ty::Fn(params, Box::new(self.resolve_annotation_ir(args.get(2)))))
            }
            // Literal `[...]`/`set(...)`/`{...}` infer to the constructor type
            // (params not tracked, TH4 v1). These are dedicated opcodes, not
            // name-resolved calls, so the inference is sound: a `ListLiteral` is
            // always a list, never a shadowed `list` binding. A literal in a slot
            // of the wrong type is then caught by E300; everything else defers to
            // the runtime boundary. Set is homogeneous like list (one parameter).
            IR::Op { opcode, args, .. } if *opcode == IROpCode::ListLiteral => {
                Some(Ty::List(Box::new(self.infer_join(args.iter()))))
            }
            IR::Op { opcode, args, .. } if *opcode == IROpCode::SetLiteral => {
                Some(Ty::Set(Box::new(self.infer_join(args.iter()))))
            }
            // A tuple literal is positional: one inferred type per position (no
            // join), so the arity and each slot are tracked. A non-inferable
            // element widens that position to `Top` (unknown), never the whole
            // tuple. A literal always has a *known* arity (`Some`), including the
            // empty `()` (`Some([])`), so a fixed-arity slot rejects it statically.
            IR::Op { opcode, args, .. } if *opcode == IROpCode::TupleLiteral => Some(Ty::Tuple(Some(
                args.iter().map(|a| self.infer_type(a).unwrap_or(Ty::Top)).collect(),
            ))),
            IR::Op { opcode, args, .. } if *opcode == IROpCode::DictLiteral => {
                // Each entry is a `TupleLiteral` of [key, value]; join keys and
                // values independently. An entry that isn't a plain pair (spread,
                // unexpected shape) widens the parameter to `Top` (unknown).
                let mut key = Ty::Bottom;
                let mut val = Ty::Bottom;
                for entry in args {
                    if let IR::Op { args: kv, .. } = entry {
                        key = key.join(&kv.first().and_then(|k| self.infer_type(k)).unwrap_or(Ty::Top));
                        val = val.join(&kv.get(1).and_then(|v| self.infer_type(v)).unwrap_or(Ty::Top));
                    } else {
                        key = Ty::Top;
                        val = Ty::Top;
                    }
                }
                let or_top = |t: Ty| if t == Ty::Bottom { Ty::Top } else { t };
                Some(Ty::Dict(Box::new(or_top(key)), Box::new(or_top(val))))
            }
            IR::Bool(_) => Some(Ty::Bool),
            IR::Int(_) => Some(Ty::Int),
            IR::Float(_) => Some(Ty::Float),
            IR::String(_) => Some(Ty::Str),
            IR::None => Some(Ty::NoneT),
            IR::Ref(name, _, _) => self.var_types.get(name).cloned(),
            _ => None,
        }
    }

    /// Join the inferred types of a sequence of element expressions, for a
    /// list/dict literal's parameter inference (E300). An element whose type
    /// can't be inferred widens the result to `Top`; an empty sequence yields
    /// `Top` (unknown element).
    fn infer_join<'a>(&self, items: impl Iterator<Item = &'a IR>) -> Ty {
        let mut acc = Ty::Bottom;
        for it in items {
            acc = acc.join(&self.infer_type(it).unwrap_or(Ty::Top));
            if acc == Ty::Top {
                break;
            }
        }
        if acc == Ty::Bottom { Ty::Top } else { acc }
    }

    /// Bind a union constructor's argument types to the union's type parameters,
    /// producing the type-argument vector for `Ty::Union` (extended inference).
    /// A payload field that is *exactly* a type parameter (`Some(value: T)`) binds
    /// that parameter to the inferred argument type; a parameter left unbound (or
    /// a non-generic union) stays `Top` (unknown, defers at the boundary). Several
    /// fields mapping to the same parameter join. This is what lets
    /// `Option.Some("x")` be inferred `Option[str]` and rejected statically in an
    /// `Option[int]` slot.
    fn infer_union_args(&self, name: &str, variant: Option<&str>, call_args: &[IR]) -> Vec<Ty> {
        let Some(sig) = self.union_sigs.get(name) else {
            return Vec::new();
        };
        let arity = sig.type_params.len();
        if arity == 0 {
            return Vec::new(); // non-generic union -> bare
        }
        let mut bound = vec![Ty::Bottom; arity];
        if let Some(slots) = variant.and_then(|v| sig.variant_param_fields.get(v)) {
            for (i, slot) in slots.iter().enumerate() {
                if let Some(k) = slot {
                    if let Some(arg) = call_args.get(i) {
                        if let Some(ty) = self.infer_type(arg) {
                            bound[*k] = bound[*k].join(&ty);
                        }
                    }
                }
            }
        }
        bound
            .into_iter()
            .map(|t| if t == Ty::Bottom { Ty::Top } else { t })
            .collect()
    }

    /// Emit E300 when a resolved annotation applies a union with the wrong number
    /// of type arguments (arity is part of the contract, TH6). Recurses into
    /// composite parameters, tuple positions, and union members so a nested
    /// `list[Option[int, str]]` is caught. A bare union (empty arguments) is
    /// always well-formed (unknown arity, defers). A non-generic union given
    /// arguments (`Point[int]` where the union has no `[T]`) is a 0-vs-N mismatch,
    /// reported the same way.
    fn check_annotation_arity(&mut self, ty: &Ty, start_byte: usize, end_byte: usize, subject: &str) {
        match ty {
            Ty::Union(name, args) if !args.is_empty() => {
                if let Some(sig) = self.union_sigs.get(name) {
                    let expected = sig.type_params.len();
                    if expected != args.len() {
                        self.diagnostics.push(SemanticDiagnostic {
                            code: "E300".to_string(),
                            message: format!(
                                "{subject}: union '{name}' expects {expected} type argument(s), got {}",
                                args.len()
                            ),
                            severity: SemanticSeverity::Error,
                            start_byte,
                            end_byte,
                        });
                    }
                }
                for a in args {
                    self.check_annotation_arity(a, start_byte, end_byte, subject);
                }
            }
            Ty::List(e) | Ty::Set(e) => self.check_annotation_arity(e, start_byte, end_byte, subject),
            Ty::Dict(k, v) => {
                self.check_annotation_arity(k, start_byte, end_byte, subject);
                self.check_annotation_arity(v, start_byte, end_byte, subject);
            }
            Ty::Tuple(Some(ps)) => {
                for p in ps {
                    self.check_annotation_arity(p, start_byte, end_byte, subject);
                }
            }
            Ty::OneOf(ms) => {
                for m in ms {
                    self.check_annotation_arity(m, start_byte, end_byte, subject);
                }
            }
            Ty::Fn(params, ret) => {
                for p in params {
                    self.check_annotation_arity(p, start_byte, end_byte, subject);
                }
                self.check_annotation_arity(ret, start_byte, end_byte, subject);
            }
            _ => {}
        }
    }

    /// TH3 step 2: find free functions with a provably unique binding (recorded
    /// in `self.fn_sigs`) and the set of names that may shadow a struct
    /// constructor (recorded in `self.shadowed_names`).
    ///
    /// A function name qualifies when, across the whole program, it is assigned
    /// exactly once (to a lambda), never used as a value (every `Ref` to it is
    /// the callee of a `Call`), never bound (parameter/pattern), and not also a
    /// struct name. The *global* count is what makes this sound without scope
    /// tracking: a name rebound in any scope, captured, aliased, or shadowed is
    /// excluded, so a recorded signature is unambiguous at every call site.
    ///
    /// A struct constructor (checked separately, against `struct_fields`) is only
    /// trusted when its name is never assigned or bound anywhere -- otherwise a
    /// parameter, local, or pattern of the same name could be the real callee.
    fn collect_unique_fns(&mut self, ir: &IR) {
        let mut acc = ScanAcc::default();
        Self::scan_bindings(ir, &mut acc);

        for (name, count) in &acc.assign_count {
            if *count != 1
                || acc.value_uses.get(name).copied().unwrap_or(0) != 0
                || acc.bound_names.contains(name)
                || acc.struct_names.contains(name)
            {
                continue;
            }
            let Some(IR::Op { opcode, args, .. }) = acc.last_value.get(name) else {
                continue;
            };
            if *opcode != IROpCode::OpLambda {
                continue;
            }
            if let Some(IR::Tuple(params) | IR::List(params)) = args.first() {
                let sig = self.extract_fn_sig(params);
                self.fn_sigs.insert(name.clone(), sig);
            }
        }

        // A struct constructor is only trusted when its name appears *only* as a
        // struct declaration and as a call target. Any other occurrence -- an
        // assignment, a parameter/pattern binding, or a value use (a loop
        // variable, an alias, an argument) -- means it could be shadowed at the
        // call site, so record all of those and skip checking them.
        self.assigned_names = acc.assign_count.keys().cloned().collect();
        let mut shadowed: HashSet<String> = acc.bound_names;
        shadowed.extend(acc.assign_count.keys().cloned());
        // Bound-or-assigned, before value uses are folded in: the guard a union
        // constructor uses, since reaching `U` through `U.A(...)` is a value use
        // that must not disqualify it (see `bound_or_assigned_names`).
        self.bound_or_assigned_names = shadowed.clone();
        shadowed.extend(acc.value_uses.keys().cloned());
        self.shadowed_names = shadowed;
    }

    /// Build a `FnSig` from a lambda's parameter list. Regular params (3-tuple
    /// `[name, default, type]`) carry their resolved `Ty`; a variadic
    /// (`["*", name]`) records where positional checking stops.
    fn extract_fn_sig(&self, params: &[IR]) -> FnSig {
        let mut out: Vec<(String, Ty)> = Vec::new();
        let mut vararg_at = None;
        for p in params {
            let IR::Tuple(parts) = p else { continue };
            if matches!(parts.first(), Some(IR::String(s)) if s == "*") {
                vararg_at = Some(out.len());
                break;
            }
            let Some(IR::String(pname)) = parts.first() else {
                continue;
            };
            let ty = self.resolve_annotation_ir(parts.get(2));
            out.push((pname.clone(), ty));
        }
        FnSig { params: out, vararg_at }
    }

    /// Walk for `collect_unique_fns`: count assignments, value-uses, bound names,
    /// and struct names. A `Ref` reached through the generic arms is a value use;
    /// the special arms suppress the positions that are bindings, not reads
    /// (callee of a `Call`, assignment target, parameter/pattern name).
    fn scan_bindings(ir: &IR, acc: &mut ScanAcc) {
        match ir {
            IR::Ref(name, _, _) | IR::Identifier(name) => {
                *acc.value_uses.entry(name.clone()).or_insert(0) += 1;
            }
            IR::Call { func, args, kwargs, .. } => {
                // The callee Ref is not a value use; a non-Ref callee is scanned.
                if !matches!(func.as_ref(), IR::Ref(_, _, _) | IR::Identifier(_)) {
                    Self::scan_bindings(func, acc);
                }
                for a in args {
                    Self::scan_bindings(a, acc);
                }
                for v in kwargs.values() {
                    Self::scan_bindings(v, acc);
                }
            }
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpStruct => {
                // Record the struct name; a constructor with this name is checked
                // only if the name is never assigned/bound elsewhere.
                if let Some(IR::String(name)) = args.first() {
                    acc.struct_names.insert(name.clone());
                }
                for a in args {
                    Self::scan_bindings(a, acc);
                }
            }
            IR::Op { opcode, args, .. } if *opcode == IROpCode::OpTry => {
                // An exception binding (`except E => name { ... }`) is a bound
                // name stored as a String in `clause[1]` of each handler tuple;
                // record it like a parameter. args[1] is the handler list.
                if let Some(IR::List(handlers) | IR::Tuple(handlers)) = args.get(1) {
                    for h in handlers {
                        if let IR::Tuple(clause) = h {
                            if let Some(IR::String(binding)) = clause.get(1) {
                                acc.bound_names.insert(binding.clone());
                            }
                        }
                    }
                }
                for a in args {
                    Self::scan_bindings(a, acc);
                }
            }
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::SetLocals => {
                // `scan_target` records each assigned name (single target or one
                // slot of an unpacking) in `assign_count`; complex sub-targets
                // (a[i], o.f) are reads. Keep the value only for a single target,
                // where a function signature can be extracted.
                if let Some(target) = args.first() {
                    Self::scan_target(target, acc);
                }
                if let Some(name) = Self::extract_single_assign_name(&args[0]) {
                    if let Some(value) = args.get(1) {
                        acc.last_value.insert(name, value.clone());
                    }
                }
                for a in args.iter().skip(1) {
                    Self::scan_bindings(a, acc);
                }
                for v in kwargs.values() {
                    Self::scan_bindings(v, acc);
                }
            }
            IR::Op {
                opcode, args, kwargs, ..
            } if *opcode == IROpCode::OpLambda || *opcode == IROpCode::FnDef => {
                // Parameter names are bindings, not value uses; their default
                // values are still scanned (a default can capture a function).
                if let Some(IR::Tuple(params) | IR::List(params)) = args.first() {
                    for p in params {
                        let IR::Tuple(parts) = p else { continue };
                        let is_vararg = matches!(parts.first(), Some(IR::String(s)) if s == "*");
                        let name_idx = if is_vararg { 1 } else { 0 };
                        if let Some(IR::String(pname)) = parts.get(name_idx) {
                            acc.bound_names.insert(pname.clone());
                        }
                        if !is_vararg {
                            if let Some(default) = parts.get(1) {
                                Self::scan_bindings(default, acc);
                            }
                        }
                    }
                }
                for a in args.iter().skip(1) {
                    Self::scan_bindings(a, acc);
                }
                for v in kwargs.values() {
                    Self::scan_bindings(v, acc);
                }
            }
            IR::Op { args, kwargs, .. } => {
                for a in args {
                    Self::scan_bindings(a, acc);
                }
                for v in kwargs.values() {
                    Self::scan_bindings(v, acc);
                }
            }
            IR::Program(items)
            | IR::List(items)
            | IR::Tuple(items)
            | IR::Set(items)
            | IR::PatternOr(items)
            | IR::PatternTuple(items) => {
                for i in items {
                    Self::scan_bindings(i, acc);
                }
            }
            IR::Dict(pairs) => {
                for (k, v) in pairs {
                    Self::scan_bindings(k, acc);
                    Self::scan_bindings(v, acc);
                }
            }
            IR::Slice { start, stop, step } => {
                Self::scan_bindings(start, acc);
                Self::scan_bindings(stop, acc);
                Self::scan_bindings(step, acc);
            }
            IR::Broadcast {
                target,
                operator,
                operand,
                ..
            } => {
                if let Some(t) = target {
                    Self::scan_bindings(t, acc);
                }
                Self::scan_bindings(operator, acc);
                if let Some(o) = operand {
                    Self::scan_bindings(o, acc);
                }
            }
            IR::PatternLiteral(inner) => {
                Self::scan_bindings(inner, acc);
            }
            // Pattern-bound names are bindings (like parameters): record them so a
            // function or constructor name shadowed by a `match` pattern is not
            // checked.
            IR::PatternVar(name) => {
                acc.bound_names.insert(name.clone());
            }
            IR::PatternStruct { fields, .. } => {
                for f in fields {
                    acc.bound_names.insert(f.clone());
                }
            }
            _ => {}
        }
    }

    /// Scan an assignment target. A bare `Ref`/`Identifier` (possibly nested in
    /// an unpacking tuple) is a binding and contributes no value use; a complex
    /// target (subscript, attribute) reads its base/index, so it is scanned.
    fn scan_target(target: &IR, acc: &mut ScanAcc) {
        match target {
            // A bare target name is an assignment (a single target or one slot of
            // an unpacking), not a value use: count it so the name is excluded
            // from unique-function status and from constructor checking.
            IR::Ref(name, _, _) | IR::Identifier(name) => {
                *acc.assign_count.entry(name.clone()).or_insert(0) += 1;
            }
            IR::Tuple(items) | IR::List(items) => {
                for it in items {
                    Self::scan_target(it, acc);
                }
            }
            // Complex target (subscript, attribute): the base/index are reads.
            other => Self::scan_bindings(other, acc),
        }
    }

    /// TH3 step 2: at a call to a provably-unique free function or a struct
    /// constructor, check each provided argument against the declared parameter
    /// (or field) type. No-op for any other callee.
    fn check_call_site(
        &mut self,
        func: &IR,
        args: &[IR],
        kwargs: &indexmap::IndexMap<String, IR>,
        start_byte: usize,
        end_byte: usize,
    ) {
        // Union variant constructor: `U.A(args)`, where `func` is
        // `GetAttr(Ref(U), "A")`. Checked like a struct constructor -- each
        // payload field's concrete declared type checks the argument at its
        // position -- but reached through the attribute callee the `Ref` path
        // below cannot see. The union name must be unambiguous (never shadowed),
        // the same trust rule as a struct constructor. Type-parameter fields are
        // `Top` here (bound and enforced at the generic boundary), so they never
        // fire a false positive.
        if let IR::Op {
            opcode, args: gargs, ..
        } = func
        {
            if *opcode == IROpCode::GetAttr {
                if let (Some(IR::Ref(uname, _, _) | IR::Identifier(uname)), Some(IR::String(variant))) =
                    (gargs.first(), gargs.get(1))
                {
                    if !self.bound_or_assigned_names.contains(uname) {
                        if let Some(fields) = self.union_sigs.get(uname).and_then(|s| s.variant_fields.get(variant)) {
                            let fields = fields.clone();
                            let callee = format!("{uname}.{variant}");
                            self.check_call_args(&fields, None, args, kwargs, &callee, start_byte, end_byte);
                        }
                    }
                }
            }
            return;
        }

        let (IR::Ref(name, _, _) | IR::Identifier(name)) = func else {
            return;
        };
        // Clone the parameter list to release the immutable borrow of `self`
        // before pushing diagnostics. A constructor is only trusted when its name
        // cannot be shadowed at the call site (never assigned or bound anywhere);
        // `fn_sigs` already encodes that guarantee for functions.
        let (params, vararg_at) = if let Some(sig) = self.fn_sigs.get(name) {
            (sig.params.clone(), sig.vararg_at)
        } else if let Some(fields) = self.struct_fields.get(name) {
            if self.shadowed_names.contains(name) {
                return;
            }
            (fields.clone(), None)
        } else if let (Some(Ty::Fn(fparams, fret)), false) =
            (self.var_types.get(name), self.assigned_names.contains(name))
        {
            // A call through a declared function type (`cb: (int) -> int`).
            // The declared arity IS the contract -- fixed, with no defaults or
            // keywords expressible in the type -- so a call that provides a
            // different count (or any keyword) is a provable mismatch, even
            // though the concrete value might tolerate it through defaults:
            // the contract only promises this exact shape (same reading as
            // tuple arity). Each provided argument then checks positionally.
            let fret = fret.clone();
            let fparams = fparams.clone();
            let callee = name.clone();
            if args.len() != fparams.len() || !kwargs.is_empty() {
                // The declared type is rebuilt here only for the message: the
                // conforming path (the common case) pays no extra clone.
                let declared = Ty::Fn(fparams.clone(), fret);
                self.diagnostics.push(SemanticDiagnostic {
                    code: "E300".to_string(),
                    message: format!(
                        "call to '{callee}' of type '{declared}' expects {} argument(s) but got {}{}",
                        fparams.len(),
                        args.len(),
                        if kwargs.is_empty() { "" } else { " plus keyword(s)" },
                    ),
                    severity: SemanticSeverity::Error,
                    start_byte,
                    end_byte,
                });
                return;
            }
            let params: Vec<(String, Ty)> = fparams
                .into_iter()
                .enumerate()
                .map(|(i, t)| (format!("#{}", i + 1), t))
                .collect();
            self.check_call_args(&params, None, args, kwargs, &callee, start_byte, end_byte);
            return;
        } else {
            return;
        };
        let callee = name.clone();
        self.check_call_args(&params, vararg_at, args, kwargs, &callee, start_byte, end_byte);
    }

    /// Check positional arguments (up to the variadic, if any) and keyword
    /// arguments (matched by name) against `params`.
    #[allow(clippy::too_many_arguments)]
    fn check_call_args(
        &mut self,
        params: &[(String, Ty)],
        vararg_at: Option<usize>,
        args: &[IR],
        kwargs: &indexmap::IndexMap<String, IR>,
        callee: &str,
        start_byte: usize,
        end_byte: usize,
    ) {
        let positional_limit = vararg_at.unwrap_or(params.len());
        for (i, arg) in args.iter().enumerate() {
            if i >= positional_limit {
                break;
            }
            let (pname, declared) = &params[i];
            self.check_arg_against(arg, declared, callee, pname, start_byte, end_byte);
        }
        for (kw, val) in kwargs {
            if let Some((_, declared)) = params.iter().find(|(n, _)| n == kw) {
                self.check_arg_against(val, declared, callee, kw, start_byte, end_byte);
            }
        }
    }

    /// Emit E300 when an argument's inferred type provably mismatches the
    /// declared parameter type (both concrete, different). Sound: a `Top`
    /// (unknown) on either side never fires.
    fn check_arg_against(
        &mut self,
        arg: &IR,
        declared: &Ty,
        callee: &str,
        param: &str,
        start_byte: usize,
        end_byte: usize,
    ) {
        if !declared.is_concrete() {
            return;
        }
        let Some(actual) = self.infer_type(arg) else {
            return;
        };
        let covariant = Self::is_composite_literal(arg);
        if actual.is_concrete() && !declared.accepts_value(&actual, covariant) {
            self.diagnostics.push(SemanticDiagnostic {
                code: "E300".to_string(),
                message: format!("argument '{param}' of '{callee}' expects '{declared}' but got '{actual}'"),
                severity: SemanticSeverity::Error,
                start_byte,
                end_byte,
            });
        }
    }

    fn extract_single_assign_name(target: &IR) -> Option<String> {
        match target {
            IR::Tuple(items) | IR::List(items) if items.len() == 1 => match &items[0] {
                IR::Ref(name, _, _) | IR::Identifier(name) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn check_match_node(&mut self, args: &[IR], start_byte: usize, end_byte: usize) {
        if args.len() < 2 {
            return;
        }
        let scrutinee = &args[0];
        let cases = match &args[1] {
            IR::Tuple(cases) => cases,
            _ => return,
        };

        if Self::has_unconditional_catchall(cases) {
            return;
        }

        let patterns = Self::collect_unguarded_patterns(cases);
        let scrutinee_type = self.infer_type(scrutinee);

        let is_exhaustive = match &scrutinee_type {
            Some(Ty::Enum(enum_name)) => self.check_enum_exhaustive(&patterns, enum_name),
            Some(Ty::Union(union_name, _)) => self.check_union_exhaustive(&patterns, union_name),
            Some(Ty::Bool) => Self::check_bool_exhaustive(&patterns),
            // Primitives, structs, unknown (`Top`/absent): no finite variant set
            // to check against -> never suppress, exactly as for an unknown type.
            _ => false,
        };

        if !is_exhaustive {
            let message = match &scrutinee_type {
                Some(Ty::Enum(name)) => {
                    let covered = Self::collect_enum_variants(&patterns, name);
                    if let Some(all) = self.enum_defs.get(name) {
                        let mut missing: Vec<_> = all.iter().filter(|v| !covered.contains(*v)).cloned().collect();
                        missing.sort();
                        format!(
                            "Non-exhaustive match on enum '{}'; missing: {}",
                            name,
                            missing.join(", ")
                        )
                    } else {
                        format!("Non-exhaustive match on enum '{}'", name)
                    }
                }
                Some(Ty::Union(name, _)) => {
                    let covered = self.collect_union_variants(&patterns, name);
                    if let Some(all) = self.union_defs.get(name) {
                        let mut missing: Vec<String> = all.keys().filter(|v| !covered.contains(*v)).cloned().collect();
                        missing.sort();
                        format!(
                            "Non-exhaustive match on union '{}'; missing: {}",
                            name,
                            missing.join(", ")
                        )
                    } else {
                        format!("Non-exhaustive match on union '{}'", name)
                    }
                }
                Some(Ty::Bool) => {
                    let (has_true, has_false) = Self::bool_coverage(&patterns);
                    let missing = match (has_true, has_false) {
                        (false, false) => "True, False",
                        (true, false) => "False",
                        (false, true) => "True",
                        _ => "",
                    };
                    format!("Non-exhaustive match on boolean; missing: {}", missing)
                }
                _ => "Match has no wildcard branch; exhaustiveness depends on runtime values".to_string(),
            };
            self.diagnostics.push(SemanticDiagnostic {
                code: "I103".to_string(),
                message,
                severity: SemanticSeverity::Hint,
                start_byte,
                end_byte,
            });
        }
    }

    fn has_unconditional_catchall(cases: &[IR]) -> bool {
        for case in cases {
            if let IR::Tuple(elems) = case {
                if elems.len() >= 2 && elems[1] == IR::None && Self::is_catchall(&elems[0]) {
                    return true;
                }
            }
        }
        false
    }

    fn is_catchall(pattern: &IR) -> bool {
        match pattern {
            IR::PatternWildcard | IR::PatternVar(_) => true,
            IR::PatternOr(pats) => pats.iter().any(Self::is_catchall),
            _ => false,
        }
    }

    fn collect_unguarded_patterns(cases: &[IR]) -> Vec<IR> {
        let mut patterns = Vec::new();
        for case in cases {
            if let IR::Tuple(elems) = case {
                if elems.len() >= 2 && elems[1] == IR::None {
                    Self::flatten_pattern(&elems[0], &mut patterns);
                }
            }
        }
        patterns
    }

    fn flatten_pattern(pattern: &IR, out: &mut Vec<IR>) {
        match pattern {
            IR::PatternOr(pats) => {
                for p in pats {
                    Self::flatten_pattern(p, out);
                }
            }
            other => out.push(other.clone()),
        }
    }

    fn check_enum_exhaustive(&self, patterns: &[IR], enum_name: &str) -> bool {
        let all_variants = match self.enum_defs.get(enum_name) {
            Some(vs) => vs,
            None => return false,
        };
        let covered = Self::collect_enum_variants(patterns, enum_name);
        covered == *all_variants
    }

    fn collect_enum_variants(patterns: &[IR], enum_name: &str) -> HashSet<String> {
        let mut covered = HashSet::new();
        for pat in patterns {
            if let IR::PatternEnum {
                enum_name: en,
                variant_name: vn,
            } = pat
            {
                if en == enum_name {
                    covered.insert(vn.clone());
                }
            }
        }
        covered
    }

    fn check_union_exhaustive(&self, patterns: &[IR], union_name: &str) -> bool {
        let all_variants = match self.union_defs.get(union_name) {
            Some(vs) => vs,
            None => return false,
        };
        let covered = self.collect_union_variants(patterns, union_name);
        all_variants.keys().all(|v| covered.contains(v))
    }

    /// Collect variant names covered by a set of patterns on a union scrutinee.
    ///
    /// Two pattern shapes can target a union variant:
    /// - Nullary variants use `pattern_enum` (`Option.None`). A `pattern_enum`
    ///   pointing at a payload-bearing variant never matches at runtime, so we
    ///   refuse to count it as coverage -- otherwise `Option.Some` (no braces)
    ///   would suppress exhaustiveness warnings for `Option.Some(1)`.
    /// - Payload-bearing variants use the qualified `pattern_struct`
    ///   (`Option.Some{value}`), distinguished by its `variant` field.
    fn collect_union_variants(&self, patterns: &[IR], union_name: &str) -> HashSet<String> {
        let variants = self.union_defs.get(union_name);
        let mut covered = HashSet::new();
        for pat in patterns {
            match pat {
                IR::PatternEnum {
                    enum_name: en,
                    variant_name: vn,
                } if en == union_name => {
                    // Only count if the variant is actually nullary; a
                    // bare `Option.Some` against a payload variant is dead.
                    if let Some(v) = variants {
                        if v.get(vn) == Some(&false) {
                            covered.insert(vn.clone());
                        }
                    }
                }
                IR::PatternStruct {
                    name,
                    variant: Some(vn),
                    ..
                } if name == union_name => {
                    // PatternStruct with payload-fields covers any variant
                    // that exists on the union (nullary or not). The
                    // matcher uses the qualified name lookup so even a
                    // zero-field `Option.None{}` resolves cleanly.
                    if let Some(v) = variants {
                        if v.contains_key(vn) {
                            covered.insert(vn.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        covered
    }

    fn check_bool_exhaustive(patterns: &[IR]) -> bool {
        let (has_true, has_false) = Self::bool_coverage(patterns);
        has_true && has_false
    }

    fn bool_coverage(patterns: &[IR]) -> (bool, bool) {
        let mut has_true = false;
        let mut has_false = false;
        for pat in patterns {
            if let IR::PatternLiteral(inner) = pat {
                match inner.as_ref() {
                    IR::Bool(true) => has_true = true,
                    IR::Bool(false) => has_false = true,
                    _ => {}
                }
            }
        }
        (has_true, has_false)
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export for convenience
pub use crate::semantic::passes::PurePass;

/// A star element `*rest` (in unpacking targets and patterns alike) is
/// transformed to `("*", name)`; recover the bound name.
fn star_name(node: &IR) -> Option<&str> {
    if let IR::Tuple(parts) = node {
        if let [IR::String(s), IR::String(name)] = parts.as_slice() {
            if s == "*" {
                return Some(name);
            }
        }
    }
    None
}

/// Names bound by an assignment / `for` target (bare name, unpacking tuple,
/// star element). Shared by `collect_rebound` (typed-arith proving) and the
/// analyzer walker's binding-scope invalidation.
fn binding_target_names(t: &IR, acc: &mut HashSet<String>) {
    if let Some(name) = star_name(t) {
        acc.insert(name.to_string());
        return;
    }
    match t {
        IR::Ref(name, _, _) | IR::Identifier(name) => {
            acc.insert(name.clone());
        }
        IR::Tuple(v) | IR::List(v) => {
            for x in v {
                binding_target_names(x, acc);
            }
        }
        _ => {}
    }
}

/// Names bound by a match pattern (variable patterns, struct fields, star
/// elements, and their nested/OR/tuple forms). These shadow a same-named outer
/// binding inside the arm. Shared by `collect_rebound` and the walker's
/// per-arm `var_types` invalidation -- one enumeration of the binding pattern
/// forms, so the two cannot drift.
fn pattern_binding_names(pat: &IR, acc: &mut HashSet<String>) {
    if let Some(name) = star_name(pat) {
        acc.insert(name.to_string());
        return;
    }
    match pat {
        IR::PatternVar(name) => {
            acc.insert(name.clone());
        }
        IR::PatternStruct { fields, .. } => {
            for f in fields {
                acc.insert(f.clone());
            }
        }
        IR::PatternOr(pats) | IR::PatternTuple(pats) => {
            for p in pats {
                pattern_binding_names(p, acc);
            }
        }
        // PatternLiteral, PatternWildcard, PatternEnum bind nothing.
        _ => {}
    }
}

// Names rebound anywhere in a body: assignment targets, loop variables and
// except bindings. A param that gets rebound is no longer guaranteed to hold
// its declared type, so it must drop out of the proven set (sound: a wrong
// type never reaches a typed opcode). Recurses through nested lambdas too --
// conservative w.r.t. shadowing, never unsound.
fn collect_rebound(ir: &IR, acc: &mut HashSet<String>) {
    let add_targets = binding_target_names;
    let pattern_bindings = pattern_binding_names;
    match ir {
        IR::Op {
            opcode: IROpCode::SetLocals,
            args,
            ..
        }
        | IR::Op {
            opcode: IROpCode::OpFor,
            args,
            ..
        } => {
            if let Some(t) = args.first() {
                add_targets(t, acc);
            }
        }
        IR::Op {
            opcode: IROpCode::OpTry,
            args,
            ..
        } => {
            // args[1] = handler list; each handler is (types, binding, block).
            // The binding is a bare name string (or None when absent).
            if let Some(IR::List(handlers)) = args.get(1) {
                for h in handlers {
                    if let IR::Tuple(parts) = h {
                        match parts.get(1) {
                            Some(IR::String(name)) => {
                                acc.insert(name.clone());
                            }
                            Some(other) => add_targets(other, acc),
                            None => {}
                        }
                    }
                }
            }
        }
        IR::Op {
            opcode: IROpCode::OpMatch,
            args,
            ..
        } => {
            // args[1] = case tuple; each case is (pattern, guard, block).
            if let Some(IR::Tuple(cases)) = args.get(1) {
                for case in cases {
                    if let IR::Tuple(parts) = case {
                        if let Some(pat) = parts.first() {
                            pattern_bindings(pat, acc);
                        }
                    }
                }
            }
        }
        IR::Op {
            opcode: IROpCode::OpStruct | IROpCode::EnumDef | IROpCode::UnionDef | IROpCode::TraitDef,
            args,
            ..
        } => {
            // A local type definition binds its name (args[0]), shadowing a
            // same-named param.
            if let Some(IR::String(name)) = args.first() {
                acc.insert(name.clone());
            }
        }
        _ => {}
    }
    match ir {
        IR::Op { args, kwargs, .. } => {
            for a in args {
                collect_rebound(a, acc);
            }
            for v in kwargs.values() {
                collect_rebound(v, acc);
            }
        }
        IR::Call { func, args, kwargs, .. } => {
            collect_rebound(func, acc);
            for a in args {
                collect_rebound(a, acc);
            }
            for v in kwargs.values() {
                collect_rebound(v, acc);
            }
        }
        IR::Program(v) | IR::List(v) | IR::Tuple(v) => {
            for i in v {
                collect_rebound(i, acc);
            }
        }
        _ => {}
    }
}

/// FT2-A: wrap calls through a declared-callback param in a `CheckReturn`
/// node, so the callback's declared return is enforced on the caller side at
/// runtime -- a callback value is opaque to the static half, and the typed
/// zone consumes what it returns. Only a param that is never assigned
/// anywhere (the same trust rule as the call-site check) and not rebound in
/// the body qualifies; the wrap clears the call's tail flag (a checked return
/// must be consumed, so the site structurally cannot be a tail call).
fn rewrite_callback_return_checks(ir: IR, assigned: &HashSet<String>) -> IR {
    use crate::vm::opcode::{ParamCheck, fn_type_split};

    // FT params of a lambda's param list whose declared return is
    // runtime-enforceable (name -> return annotation text), plus the names of
    // ALL the lambda's params -- a non-FT param shadows any same-named binding
    // inherited from the enclosing lambda.
    fn ft_params_env(params_ir: &IR, assigned: &HashSet<String>) -> (HashMap<String, String>, HashSet<String>) {
        let mut env = HashMap::new();
        let mut own_names = HashSet::new();
        let items = match params_ir {
            IR::Tuple(v) | IR::List(v) => v.as_slice(),
            _ => return (env, own_names),
        };
        for item in items {
            let parts = match item {
                IR::Tuple(v) | IR::List(v) => v.as_slice(),
                _ => continue,
            };
            let name = match parts.first() {
                Some(IR::String(n)) => n,
                _ => continue,
            };
            // The vararg marker ("*", name) binds parts[1].
            if name == "*" {
                if let Some(IR::String(vn)) = parts.get(1) {
                    own_names.insert(vn.clone());
                }
                continue;
            }
            own_names.insert(name.clone());
            if let Some(IR::String(ty)) = parts.get(2) {
                if assigned.contains(name) {
                    continue;
                }
                if let Some((_params, ret)) = fn_type_split(ty) {
                    if ParamCheck::from_annotation(ret) != ParamCheck::None {
                        env.insert(name.clone(), ret.to_string());
                    }
                }
            }
        }
        (env, own_names)
    }

    fn go(ir: IR, env: &HashMap<String, String>, assigned: &HashSet<String>) -> IR {
        match ir {
            IR::Op {
                opcode: IROpCode::OpLambda,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                // The env LAYERS over the enclosing one: a callback captured
                // by an inner closure keeps its declared return (the trust
                // comes from the annotation, and a capture preserves it), so
                // `w = () => { cb(1) }` is checked like a direct call. The
                // inner lambda's own params shadow inherited names, and any
                // name rebound in the body drops out (same rule as
                // rewrite_typed_arith).
                let (own_env, own_names) = args.first().map(|p| ft_params_env(p, assigned)).unwrap_or_default();
                let mut new_env: HashMap<String, String> = env
                    .iter()
                    .filter(|(name, _)| !own_names.contains(*name))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                new_env.extend(own_env);
                if !new_env.is_empty() {
                    if let Some(body) = args.get(1) {
                        let mut rebound = HashSet::new();
                        collect_rebound(body, &mut rebound);
                        new_env.retain(|name, _| !rebound.contains(name));
                    }
                }
                let args = args
                    .into_iter()
                    .enumerate()
                    .map(|(i, a)| if i == 0 { a } else { go(a, &new_env, assigned) })
                    .collect();
                IR::Op {
                    opcode: IROpCode::OpLambda,
                    args,
                    kwargs,
                    tail,
                    start_byte,
                    end_byte,
                }
            }
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                tail,
            } => {
                let ret = match func.as_ref() {
                    IR::Ref(name, _, _) | IR::Identifier(name) => env.get(name).cloned(),
                    _ => None,
                };
                let call = IR::Call {
                    func: Box::new(go(*func, env, assigned)),
                    args: args.into_iter().map(|a| go(a, env, assigned)).collect(),
                    kwargs: kwargs.into_iter().map(|(k, v)| (k, go(v, env, assigned))).collect(),
                    start_byte,
                    end_byte,
                    // A wrapped call loses its tail position (see doc above).
                    tail: tail && ret.is_none(),
                };
                match ret {
                    Some(ret) => IR::Op {
                        opcode: IROpCode::CheckReturn,
                        args: vec![call, IR::String(ret)],
                        kwargs: Default::default(),
                        tail,
                        start_byte,
                        end_byte,
                    },
                    None => call,
                }
            }
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => IR::Op {
                opcode,
                args: args.into_iter().map(|a| go(a, env, assigned)).collect(),
                kwargs: kwargs.into_iter().map(|(k, v)| (k, go(v, env, assigned))).collect(),
                tail,
                start_byte,
                end_byte,
            },
            IR::Program(v) => IR::Program(v.into_iter().map(|i| go(i, env, assigned)).collect()),
            IR::List(v) => IR::List(v.into_iter().map(|i| go(i, env, assigned)).collect()),
            IR::Tuple(v) => IR::Tuple(v.into_iter().map(|i| go(i, env, assigned)).collect()),
            other => other,
        }
    }

    go(ir, &HashMap::new(), assigned)
}

/// TH4 canal A: rewrite `Add` -> `AddInt`/`AddFloat` when both operands are a
/// proven primitive runtime fact, so the compiler emits the specialized opcode
/// (and the JIT a pre-typed trace). Conservative v1: only literals, `int`/
/// `float`-annotated params (enforced at the boundary by the prologue CheckType)
/// and the result of an already-rewritten typed add count as proven; locals,
/// captures and other types stay polymorphic. Sound by construction: a wrong
/// type never reaches a typed opcode.
fn rewrite_typed_arith(ir: IR) -> IR {
    use std::collections::{HashMap, HashSet};

    fn proven_ty(op: &IR, env: &HashMap<String, Ty>) -> Option<Ty> {
        match op {
            IR::Int(_) => Some(Ty::Int),
            IR::Float(_) => Some(Ty::Float),
            IR::Ref(name, _, _) | IR::Identifier(name) => env.get(name).cloned(),
            // A typed-arithmetic result carries its proven type (enables chaining,
            // e.g. (x + 1) + 1 specializes both adds).
            IR::Op {
                opcode: IROpCode::AddInt | IROpCode::SubInt | IROpCode::MulInt,
                ..
            } => Some(Ty::Int),
            IR::Op {
                opcode: IROpCode::AddFloat | IROpCode::SubFloat | IROpCode::MulFloat | IROpCode::DivFloat,
                ..
            } => Some(Ty::Float),
            _ => None,
        }
    }

    // Map a polymorphic arithmetic opcode to its typed specialization given the
    // proven operand types. `Add`/`Sub`/`Mul` specialize for both int and float;
    // `Div`/`TrueDiv` only for float (int/int via `/` yields a float, so it stays
    // polymorphic). Any unproven/mixed pair keeps the polymorphic opcode.
    fn typed_arith_opcode(poly: IROpCode, lhs: Option<Ty>, rhs: Option<Ty>) -> IROpCode {
        match (lhs, rhs) {
            (Some(Ty::Int), Some(Ty::Int)) => match poly {
                IROpCode::Add => IROpCode::AddInt,
                IROpCode::Sub => IROpCode::SubInt,
                IROpCode::Mul => IROpCode::MulInt,
                _ => poly,
            },
            (Some(Ty::Float), Some(Ty::Float)) => match poly {
                IROpCode::Add => IROpCode::AddFloat,
                IROpCode::Sub => IROpCode::SubFloat,
                IROpCode::Mul => IROpCode::MulFloat,
                IROpCode::Div | IROpCode::TrueDiv => IROpCode::DivFloat,
                _ => poly,
            },
            _ => poly,
        }
    }

    // Annotated `int`/`float` params of a lambda's param list; these are runtime
    // facts once the prologue CheckType enforces them.
    fn params_env(params_ir: &IR) -> HashMap<String, Ty> {
        let mut env = HashMap::new();
        let items = match params_ir {
            IR::Tuple(v) | IR::List(v) => v.as_slice(),
            _ => return env,
        };
        for item in items {
            let parts = match item {
                IR::Tuple(v) | IR::List(v) => v.as_slice(),
                _ => continue,
            };
            if let (Some(IR::String(name)), Some(IR::String(ty))) = (parts.first(), parts.get(2)) {
                match ty.as_str() {
                    "int" => {
                        env.insert(name.clone(), Ty::Int);
                    }
                    "float" => {
                        env.insert(name.clone(), Ty::Float);
                    }
                    _ => {}
                }
            }
        }
        env
    }

    fn go(ir: IR, env: &HashMap<String, Ty>) -> IR {
        match ir {
            IR::Op {
                opcode: IROpCode::OpLambda,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => {
                let mut new_env = args.first().map(params_env).unwrap_or_default();
                // Drop any param rebound in the body: it no longer carries its
                // declared type as a runtime fact.
                if !new_env.is_empty() {
                    if let Some(body) = args.get(1) {
                        let mut rebound = HashSet::new();
                        collect_rebound(body, &mut rebound);
                        new_env.retain(|name, _| !rebound.contains(name));
                    }
                }
                let args = args
                    .into_iter()
                    .enumerate()
                    .map(|(i, a)| if i == 0 { a } else { go(a, &new_env) })
                    .collect();
                IR::Op {
                    opcode: IROpCode::OpLambda,
                    args,
                    kwargs,
                    tail,
                    start_byte,
                    end_byte,
                }
            }
            IR::Op {
                opcode: poly @ (IROpCode::Add | IROpCode::Sub | IROpCode::Mul | IROpCode::Div | IROpCode::TrueDiv),
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } if args.len() == 2 => {
                let args: Vec<IR> = args.into_iter().map(|a| go(a, env)).collect();
                // A pure literal pair is the constant folder's job (and must stay
                // polymorphic when optimization is off); only specialize when a
                // non-const operand carries the proven type.
                let is_const = |x: &IR| matches!(x, IR::Int(_) | IR::Float(_));
                let opcode = if is_const(&args[0]) && is_const(&args[1]) {
                    poly
                } else {
                    typed_arith_opcode(poly, proven_ty(&args[0], env), proven_ty(&args[1], env))
                };
                IR::Op {
                    opcode,
                    args,
                    kwargs,
                    tail,
                    start_byte,
                    end_byte,
                }
            }
            IR::Op {
                opcode,
                args,
                kwargs,
                tail,
                start_byte,
                end_byte,
            } => IR::Op {
                opcode,
                args: args.into_iter().map(|a| go(a, env)).collect(),
                kwargs: kwargs.into_iter().map(|(k, v)| (k, go(v, env))).collect(),
                tail,
                start_byte,
                end_byte,
            },
            IR::Call {
                func,
                args,
                kwargs,
                start_byte,
                end_byte,
                tail,
            } => IR::Call {
                func: Box::new(go(*func, env)),
                args: args.into_iter().map(|a| go(a, env)).collect(),
                kwargs: kwargs.into_iter().map(|(k, v)| (k, go(v, env))).collect(),
                start_byte,
                end_byte,
                tail,
            },
            IR::Program(v) => IR::Program(v.into_iter().map(|i| go(i, env)).collect()),
            IR::List(v) => IR::List(v.into_iter().map(|i| go(i, env)).collect()),
            IR::Tuple(v) => IR::Tuple(v.into_iter().map(|i| go(i, env)).collect()),
            other => other,
        }
    }

    go(ir, &HashMap::new())
}

#[cfg(test)]
mod tests;

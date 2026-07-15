// FILE: catnip_vm/tests/cfg_proptest.rs
//! Property-based differential for the CFG gate (Phase 4, `wip/CFG_SSA_REWIRING.md`).
//!
//! The finite corpus in `cfg_roundtrip.rs` pins every bug the round-trip has
//! had; this harness samples the space *between* those cases. A generator
//! builds well-formed, terminating Catnip programs biased toward the shapes
//! the wired passes (LICM, DSE, GVN) rewrite -- nested counted loops, early
//! `break`/`continue`, guarded `match`, closures read after redefinition --
//! and the property asserts the analyzer gate is observationally transparent:
//! the same source executed with the CFG round-trip off and on must agree,
//! on the value or on the error.
//!
//! Well-formedness by construction, not by tracking: a fixed preamble binds
//! every variable and lambda slot the generator can reference, so no sampled
//! program can hit `NameError` at line one and test nothing. Termination by
//! construction: each `while` owns a per-depth counter (`c0`, `c1`, ...)
//! that only the loop's own header template assigns -- incremented as the
//! first body statement, so a generated `continue` can never skip it; nested
//! loops use deeper counters and cannot reset an outer one.
//!
//! proptest shrinking works on the statement tree, so a failure minimizes to
//! a small program before it reaches the assert message.

use proptest::prelude::*;

use catnip_vm::pipeline::PurePipeline;

// Variable pool the generator may read/assign. Names steer clear of the
// reserved loop counters (`c{depth}`), match captures (`mv{depth}`), loop
// iterators (`it{depth}`) and lambda slots (`f{i}`).
const VARS: [&str; 6] = ["a", "b", "d", "e", "g", "h"];
const N_LAMBDAS: u8 = 2;
const MAX_DEPTH: u32 = 3;

#[derive(Debug, Clone)]
enum Expr {
    Lit(i8),
    Var(u8),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Lt,
    Gt,
    Eq,
}

#[derive(Debug, Clone)]
struct Cond {
    left: Expr,
    op: CmpOp,
    right: Expr,
}

#[derive(Debug, Clone)]
enum Stmt {
    Assign(u8, Expr),
    If {
        cond: Cond,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
    While {
        bound: u8,
        step: u8,
        body: Vec<Stmt>,
    },
    For {
        n: u8,
        body: Vec<Stmt>,
    },
    Match {
        scrutinee: Expr,
        lit_arm: (i8, Vec<Stmt>),
        guard_bound: i8,
        guard_arm: Vec<Stmt>,
        default_arm: Vec<Stmt>,
    },
    /// `f{i} = () => { expr }` -- captures nothing, reads VARS late-bound.
    LambdaSet(u8, Expr),
    /// `{var} = f{i}()`
    CallLambda(u8, u8),
    Break,
    Continue,
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_expr(e: &Expr) -> String {
    match e {
        Expr::Lit(n) => format!("{n}"),
        Expr::Var(v) => VARS[*v as usize % VARS.len()].to_string(),
        Expr::Add(l, r) => format!("({} + {})", render_expr(l), render_expr(r)),
        Expr::Sub(l, r) => format!("({} - {})", render_expr(l), render_expr(r)),
        Expr::Mul(l, r) => format!("({} * {})", render_expr(l), render_expr(r)),
    }
}

fn render_cond(c: &Cond) -> String {
    let op = match c.op {
        CmpOp::Lt => "<",
        CmpOp::Gt => ">",
        CmpOp::Eq => "==",
    };
    format!("{} {} {}", render_expr(&c.left), op, render_expr(&c.right))
}

/// Render a block body; an empty block degrades to a neutral expression
/// (the grammar wants a non-empty block).
fn render_block(stmts: &[Stmt], depth: u32, out: &mut String, indent: &str) {
    if stmts.is_empty() {
        out.push_str(indent);
        out.push_str("0\n");
        return;
    }
    for s in stmts {
        render_stmt(s, depth, out, indent);
    }
}

fn render_stmt(s: &Stmt, depth: u32, out: &mut String, indent: &str) {
    let deeper = format!("{indent}    ");
    match s {
        Stmt::Assign(v, e) => {
            out.push_str(indent);
            out.push_str(&format!("{} = {}\n", VARS[*v as usize % VARS.len()], render_expr(e)));
        }
        Stmt::If { cond, then, els } => {
            out.push_str(indent);
            out.push_str(&format!("if {} {{\n", render_cond(cond)));
            render_block(then, depth, out, &deeper);
            out.push_str(indent);
            out.push_str("} else {\n");
            render_block(els, depth, out, &deeper);
            out.push_str(indent);
            out.push_str("}\n");
        }
        Stmt::While { bound, step, body } => {
            // Reserved counter per depth: the increment leads the body, so a
            // generated `continue` re-tests the condition with the counter
            // already advanced -- termination is structural.
            let c = format!("c{depth}");
            out.push_str(indent);
            out.push_str(&format!("{c} = 0\n"));
            out.push_str(indent);
            out.push_str(&format!("while {c} < {bound} {{\n"));
            out.push_str(&deeper);
            out.push_str(&format!("{c} = {c} + {step}\n"));
            render_block(body, depth + 1, out, &deeper);
            out.push_str(indent);
            out.push_str("}\n");
        }
        Stmt::For { n, body } => {
            out.push_str(indent);
            out.push_str(&format!("for it{depth} in range({n}) {{\n"));
            render_block(body, depth + 1, out, &deeper);
            out.push_str(indent);
            out.push_str("}\n");
        }
        Stmt::Match {
            scrutinee,
            lit_arm,
            guard_bound,
            guard_arm,
            default_arm,
        } => {
            let mv = format!("mv{depth}");
            out.push_str(indent);
            out.push_str(&format!("match {} {{\n", render_expr(scrutinee)));
            out.push_str(&deeper);
            out.push_str(&format!("{} => {{\n", lit_arm.0));
            render_block(&lit_arm.1, depth, out, &format!("{deeper}    "));
            out.push_str(&deeper);
            out.push_str("}\n");
            out.push_str(&deeper);
            out.push_str(&format!("{mv} if {mv} > {guard_bound} => {{\n"));
            render_block(guard_arm, depth, out, &format!("{deeper}    "));
            out.push_str(&deeper);
            out.push_str("}\n");
            out.push_str(&deeper);
            out.push_str("_ => {\n");
            render_block(default_arm, depth, out, &format!("{deeper}    "));
            out.push_str(&deeper);
            out.push_str("}\n");
            out.push_str(indent);
            out.push_str("}\n");
        }
        Stmt::LambdaSet(i, e) => {
            out.push_str(indent);
            out.push_str(&format!("f{} = () => {{ {} }}\n", i % N_LAMBDAS, render_expr(e)));
        }
        Stmt::CallLambda(v, i) => {
            out.push_str(indent);
            out.push_str(&format!("{} = f{}()\n", VARS[*v as usize % VARS.len()], i % N_LAMBDAS));
        }
        Stmt::Break => {
            out.push_str(indent);
            out.push_str("break\n");
        }
        Stmt::Continue => {
            out.push_str(indent);
            out.push_str("continue\n");
        }
    }
}

/// Full program: preamble binding every referenceable name, the sampled
/// statements, then an observation summing the variable pool (so a divergence
/// in any assigned name is visible in the compared result).
fn render_program(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    for (i, v) in VARS.iter().enumerate() {
        out.push_str(&format!("{v} = {}\n", i + 1));
    }
    for i in 0..N_LAMBDAS {
        out.push_str(&format!("f{i} = () => {{ {i} }}\n"));
    }
    render_block(stmts, 0, &mut out, "");
    out.push_str("a + b + d + e + g + h\n");
    out
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn expr_strategy() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        (-9i8..10).prop_map(Expr::Lit),
        (0u8..VARS.len() as u8).prop_map(Expr::Var),
    ];
    leaf.prop_recursive(3, 12, 2, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(l, r)| Expr::Add(Box::new(l), Box::new(r))),
            (inner.clone(), inner.clone()).prop_map(|(l, r)| Expr::Sub(Box::new(l), Box::new(r))),
            (inner.clone(), inner).prop_map(|(l, r)| Expr::Mul(Box::new(l), Box::new(r))),
        ]
    })
}

fn cond_strategy() -> impl Strategy<Value = Cond> {
    (
        expr_strategy(),
        prop_oneof![Just(CmpOp::Lt), Just(CmpOp::Gt), Just(CmpOp::Eq)],
        expr_strategy(),
    )
        .prop_map(|(left, op, right)| Cond { left, op, right })
}

/// Statements allowed at `depth`, `in_loop` gating break/continue.
fn stmt_strategy(depth: u32, in_loop: bool) -> BoxedStrategy<Stmt> {
    let assign = || (0u8..VARS.len() as u8, expr_strategy()).prop_map(|(v, e)| Stmt::Assign(v, e));
    let lambda_set = (0..N_LAMBDAS, expr_strategy()).prop_map(|(i, e)| Stmt::LambdaSet(i, e));
    let call_lambda = (0u8..VARS.len() as u8, 0..N_LAMBDAS).prop_map(|(v, i)| Stmt::CallLambda(v, i));

    let mut flat: Vec<BoxedStrategy<Stmt>> = vec![
        assign().boxed(),
        assign().boxed(), // weight plain assigns higher: they feed DSE/GVN
        lambda_set.boxed(),
        call_lambda.boxed(),
    ];
    if in_loop {
        flat.push(Just(Stmt::Break).boxed());
        flat.push(Just(Stmt::Continue).boxed());
    }
    let flat_union = proptest::strategy::Union::new(flat);

    if depth >= MAX_DEPTH {
        return flat_union.boxed();
    }

    let body = move |in_l: bool| proptest::collection::vec(stmt_strategy(depth + 1, in_l), 0..4);

    let if_stmt =
        (cond_strategy(), body(in_loop), body(in_loop)).prop_map(|(cond, then, els)| Stmt::If { cond, then, els });
    let while_stmt = (1u8..7, 1u8..4, body(true)).prop_map(|(bound, step, body)| Stmt::While { bound, step, body });
    let for_stmt = (0u8..6, body(true)).prop_map(|(n, body)| Stmt::For { n, body });
    let match_stmt = (
        expr_strategy(),
        (-9i8..10),
        body(in_loop),
        (-9i8..10),
        body(in_loop),
        body(in_loop),
    )
        .prop_map(
            |(scrutinee, lit, lit_body, guard_bound, guard_arm, default_arm)| Stmt::Match {
                scrutinee,
                lit_arm: (lit, lit_body),
                guard_bound,
                guard_arm,
                default_arm,
            },
        );

    prop_oneof![
        4 => flat_union,
        2 => if_stmt,
        2 => while_stmt,
        2 => for_stmt,
        1 => match_stmt,
    ]
    .boxed()
}

fn program_strategy() -> impl Strategy<Value = Vec<Stmt>> {
    proptest::collection::vec(stmt_strategy(0, false), 1..6)
}

// ---------------------------------------------------------------------------
// Property
// ---------------------------------------------------------------------------

/// Execute with the analyzer CFG gate set; errors are compared by message
/// (both runs must agree on the failure, not just on success values). A VM
/// panic is captured as an error too, so a crashing sample shrinks cleanly
/// instead of aborting the property.
fn run(source: &str, cfg: bool) -> Result<catnip_vm::Value, String> {
    let src = source.to_string();
    std::panic::catch_unwind(move || {
        let mut p = PurePipeline::new().unwrap();
        p.set_cfg_enabled(cfg);
        p.execute(&src).map_err(|e| e.to_string())
    })
    .unwrap_or_else(|payload| {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "opaque panic".into());
        Err(format!("panic: {msg}"))
    })
}

proptest! {
    /// The CFG round-trip (LICM + DSE + GVN through the analyzer gate) is
    /// observationally transparent on sampled programs.
    #[test]
    fn cfg_gate_is_observationally_transparent(stmts in program_strategy()) {
        let src = render_program(&stmts);
        let off = run(&src, false);
        let on = run(&src, true);
        prop_assert_eq!(&off, &on, "CFG on/off diverge for:\n{}", src);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]
    /// Long-run variant for overnight sweeps (`cargo test -p catnip_vm --test
    /// cfg_proptest -- --ignored`).
    #[test]
    #[ignore]
    fn cfg_gate_transparent_long(stmts in program_strategy()) {
        let src = render_program(&stmts);
        let off = run(&src, false);
        let on = run(&src, true);
        prop_assert_eq!(&off, &on, "CFG on/off diverge for:\n{}", src);
    }
}

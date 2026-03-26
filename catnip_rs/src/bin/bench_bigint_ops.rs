// FILE: catnip_rs/src/bin/bench_bigint_ops.rs
//! Micro-benchmark for VM BigInt arithmetic operations (rug/GMP backend).
//!
//! Run with:
//!   cargo run -p catnip_rs --bin bench_bigint_ops --no-default-features --features embedded --release -- [iters]

use _rs::vm::{bench_bigint_ops, get_vm_fallback_stats, reset_vm_fallback_stats};

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200_000);

    println!("VM BigInt ops micro-bench (rug/GMP)");
    println!("iterations: {iterations}\n");

    reset_vm_fallback_stats();
    let baseline = get_vm_fallback_stats();
    println!(
        "baseline fallbacks: div={} floordiv={} mod={} eq={} ne={} pattern_eq={}\n",
        baseline.py_binary_div,
        baseline.py_binary_floordiv,
        baseline.py_binary_mod,
        baseline.py_compare_eq,
        baseline.py_compare_ne,
        baseline.py_pattern_literal_eq
    );

    for bits in [128_u32, 512_u32, 2048_u32] {
        let r = bench_bigint_ops(bits, iterations);
        println!("== {}-bit ==", r.bits);
        println!("add       {:>10.2} ns/op", r.add_ns);
        println!("mul       {:>10.2} ns/op", r.mul_ns);
        println!("floordiv  {:>10.2} ns/op", r.floordiv_ns);
        println!("mod       {:>10.2} ns/op", r.mod_ns);
        println!("div       {:>10.2} ns/op", r.div_ns);
        println!(
            "fallback delta: div={} floordiv={} mod={} eq={} ne={} pattern_eq={}\n",
            r.fallback_delta.py_binary_div,
            r.fallback_delta.py_binary_floordiv,
            r.fallback_delta.py_binary_mod,
            r.fallback_delta.py_compare_eq,
            r.fallback_delta.py_compare_ne,
            r.fallback_delta.py_pattern_literal_eq
        );
    }

    let final_stats = get_vm_fallback_stats();
    println!(
        "final cumulative fallbacks: div={} floordiv={} mod={} eq={} ne={} pattern_eq={}",
        final_stats.py_binary_div,
        final_stats.py_binary_floordiv,
        final_stats.py_binary_mod,
        final_stats.py_compare_eq,
        final_stats.py_compare_ne,
        final_stats.py_pattern_literal_eq
    );
}

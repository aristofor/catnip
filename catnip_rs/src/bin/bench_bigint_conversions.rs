// FILE: catnip_rs/src/bin/bench_bigint_conversions.rs
//! Micro-benchmark for BigInt <-> PyObject conversion paths.
//!
//! Run with:
//!   cargo run -p catnip_rs --bin bench_bigint_conversions --no-default-features --features embedded --release -- [iters]

use std::hint::black_box;
use std::time::{Duration, Instant};

use _rs::vm::Value;
use num_bigint::BigInt;
use pyo3::prelude::*;
use pyo3::types::PyInt;

fn measure<F>(iters: usize, mut f: F) -> Duration
where
    F: FnMut(),
{
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed()
}

fn ns_per_op(d: Duration, iters: usize) -> f64 {
    d.as_secs_f64() * 1_000_000_000.0 / iters as f64
}

fn print_line(label: &str, d: Duration, iters: usize) {
    println!("{label:<34} {:>12.2} ns/op", ns_per_op(d, iters));
}

fn main() {
    let iters = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100_000);

    println!("BigInt conversion micro-bench");
    println!("iterations: {iters}\n");

    Python::attach(|py| {
        let builtins = py.import("builtins").expect("import builtins");
        let int_ctor = builtins.getattr("int").expect("builtins.int");

        // Use several magnitudes to observe scaling.
        for bits in [128_u32, 512_u32, 2048_u32] {
            println!("== {}-bit integer ==", bits);

            let n: BigInt = (BigInt::from(1_u8) << bits) + BigInt::from(123_456_789_u64);
            let py_n = (&n).into_pyobject(py).expect("BigInt -> PyInt").unbind();
            let py_int = py_n.bind(py).cast::<PyInt>().expect("cast PyInt");

            // Warm-up
            for _ in 0..10_000 {
                let v = Value::from_pyobject(py, py_n.bind(py).as_any()).expect("from_pyobject");
                if v.is_bigint() {
                    v.decref();
                }
                let obj = Value::from_bigint(n.clone()).to_pyobject(py);
                black_box(obj.bind(py).as_ptr());
            }

            // from_pyobject: current native path through Value::from_pyobject
            let t_from_native = measure(iters, || {
                let v = Value::from_pyobject(py, py_n.bind(py).as_any()).expect("from_pyobject");
                black_box(v.bits());
                if v.is_bigint() {
                    v.decref();
                }
            });

            // from_pyobject: old path (string roundtrip parse)
            let t_from_string = measure(iters, || {
                let s: String = py_n
                    .bind(py)
                    .str()
                    .expect("str")
                    .extract()
                    .expect("extract str");
                let parsed = s.parse::<BigInt>().expect("parse BigInt");
                black_box(parsed);
            });

            // to_pyobject: current native path via Value::to_pyobject
            let v_big = Value::from_bigint(n.clone());
            let t_to_native = measure(iters, || {
                let obj = v_big.to_pyobject(py);
                black_box(obj.bind(py).as_ptr());
            });
            v_big.decref();

            // to_pyobject: old path (to_string + builtins.int)
            let t_to_string = measure(iters, || {
                let s = n.to_string();
                let obj = int_ctor.call1((s,)).expect("int(str)");
                black_box(obj.as_ptr());
            });

            print_line("from_pyobject native", t_from_native, iters);
            print_line("from_pyobject string(parse)", t_from_string, iters);
            print_line("to_pyobject native", t_to_native, iters);
            print_line("to_pyobject string(int(str))", t_to_string, iters);

            let from_speedup = t_from_string.as_secs_f64() / t_from_native.as_secs_f64();
            let to_speedup = t_to_string.as_secs_f64() / t_to_native.as_secs_f64();
            println!(
                "speedup: from x{:.2}, to x{:.2}\n",
                from_speedup, to_speedup
            );

            // Keep cast in scope/use to avoid optimizer deleting assumptions.
            black_box(py_int.as_ptr());
        }
    });
}

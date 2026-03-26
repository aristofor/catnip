// FILE: catnip_rs/tests/run_worker_pool.rs
//! Integration tests for WorkerPool (batch execution across worker processes).

mod common;

use _rs::nd::worker_pool::WorkerPool;
use catnip_core::freeze::{self, FrozenValue};
use catnip_core::ir::{IR, IROpCode};

fn set_worker_bin() {
    let bin = common::run_binary();
    std::env::set_var("CATNIP_WORKER_BIN", bin.to_str().unwrap());
}

#[test]
fn test_pool_creation() {
    set_worker_bin();
    let pool = WorkerPool::new(2);
    assert!(pool.is_ok());
    assert_eq!(pool.unwrap().size(), 2);
}

#[test]
fn test_pool_submit_batch_simple() {
    set_worker_bin();
    let mut pool = WorkerPool::new(2).unwrap();

    // Body: n * 2
    let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
    let encoded_ir = freeze::encode(&vec![body]).unwrap();

    let seeds = vec![
        FrozenValue::Int(1),
        FrozenValue::Int(2),
        FrozenValue::Int(3),
        FrozenValue::Int(4),
        FrozenValue::Int(5),
    ];

    let results = pool.submit_batch(&encoded_ir, &[], &["n".into()], &seeds).unwrap();

    assert_eq!(results.len(), 5);
    for (i, result) in results.iter().enumerate() {
        let expected = (i as i64 + 1) * 2;
        match result {
            FrozenValue::Int(v) => assert_eq!(*v, expected, "seed {} gave wrong result", i + 1),
            other => panic!("expected Int({}), got {:?}", expected, other),
        }
    }
}

#[test]
fn test_pool_submit_batch_with_captures() {
    set_worker_bin();
    let mut pool = WorkerPool::new(2).unwrap();

    // Body: n + offset
    let body = IR::op(
        IROpCode::Add,
        vec![IR::Identifier("n".into()), IR::Identifier("offset".into())],
    );
    let encoded_ir = freeze::encode(&vec![body]).unwrap();

    let captures = vec![("offset".into(), FrozenValue::Int(100))];
    let seeds = vec![FrozenValue::Int(1), FrozenValue::Int(2), FrozenValue::Int(3)];

    let results = pool
        .submit_batch(&encoded_ir, &captures, &["n".into()], &seeds)
        .unwrap();

    assert_eq!(
        results,
        vec![FrozenValue::Int(101), FrozenValue::Int(102), FrozenValue::Int(103),]
    );
}

#[test]
fn test_pool_submit_batch_larger_than_workers() {
    set_worker_bin();
    let mut pool = WorkerPool::new(2).unwrap();

    // Body: n * n
    let body = IR::op(
        IROpCode::Mul,
        vec![IR::Identifier("n".into()), IR::Identifier("n".into())],
    );
    let encoded_ir = freeze::encode(&vec![body]).unwrap();

    let seeds: Vec<_> = (1..=10).map(FrozenValue::Int).collect();

    let results = pool.submit_batch(&encoded_ir, &[], &["n".into()], &seeds).unwrap();

    let expected: Vec<_> = (1..=10).map(|i| FrozenValue::Int(i * i)).collect();
    assert_eq!(results, expected);
}

#[test]
fn test_pool_reuse_across_batches() {
    set_worker_bin();
    let mut pool = WorkerPool::new(2).unwrap();

    let body = IR::op(IROpCode::Add, vec![IR::Identifier("n".into()), IR::Int(1)]);
    let encoded_ir = freeze::encode(&vec![body]).unwrap();

    // First batch
    let results1 = pool
        .submit_batch(&encoded_ir, &[], &["n".into()], &[FrozenValue::Int(10)])
        .unwrap();
    assert_eq!(results1, vec![FrozenValue::Int(11)]);

    // Second batch reuses the same workers
    let results2 = pool
        .submit_batch(
            &encoded_ir,
            &[],
            &["n".into()],
            &[FrozenValue::Int(20), FrozenValue::Int(30)],
        )
        .unwrap();
    assert_eq!(results2, vec![FrozenValue::Int(21), FrozenValue::Int(31)]);
}

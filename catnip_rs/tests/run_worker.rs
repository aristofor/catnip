// FILE: catnip_rs/tests/run_worker.rs
//! Integration tests for the catnip worker subprocess.

mod common;

use catnip_core::freeze::worker::{WorkerCommand, WorkerResult, read_message, write_message};
use catnip_core::freeze::{self, FrozenValue};
use catnip_core::ir::{IR, IROpCode};
use std::io::{BufReader, BufWriter};
use std::process::{Command, Stdio};

fn spawn_worker() -> (
    BufWriter<std::process::ChildStdin>,
    BufReader<std::process::ChildStdout>,
    std::process::Child,
) {
    let bin = common::run_binary();
    let mut child = Command::new(&bin)
        .arg("worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {:?}: {}", bin, e));
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    (BufWriter::new(stdin), BufReader::new(stdout), child)
}

#[test]
fn test_worker_ping_pong() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    write_message(&mut writer, &WorkerCommand::Ping).unwrap();
    let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
    assert!(matches!(result, WorkerResult::Pong));

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn test_worker_execute_simple_int() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    // Lambda body: n * 2 (but we just send IR::Int(42) as body for simplicity)
    // Actually, let's send a body that is just "n" -- the seed itself
    let body_ir = freeze::encode(&vec![IR::Identifier("n".into())]).unwrap();

    write_message(
        &mut writer,
        &WorkerCommand::Execute {
            encoded_ir: body_ir,
            captures: vec![],
            param_names: vec!["n".into()],
            seed: FrozenValue::Int(42),
        },
    )
    .unwrap();

    let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
    match result {
        WorkerResult::Ok(FrozenValue::Int(42)) => {} // expected
        other => panic!("expected Ok(Int(42)), got {:?}", other),
    }

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    child.wait().unwrap();
}

#[test]
fn test_worker_execute_arithmetic() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    // Body: n * 2
    let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(2)]);
    let body_ir = freeze::encode(&vec![body]).unwrap();

    write_message(
        &mut writer,
        &WorkerCommand::Execute {
            encoded_ir: body_ir,
            captures: vec![],
            param_names: vec!["n".into()],
            seed: FrozenValue::Int(5),
        },
    )
    .unwrap();

    let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
    match result {
        WorkerResult::Ok(FrozenValue::Int(10)) => {}
        other => panic!("expected Ok(Int(10)), got {:?}", other),
    }

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    child.wait().unwrap();
}

#[test]
fn test_worker_execute_with_captures() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    // Body: n + factor (where factor is a captured variable)
    let body = IR::op(
        IROpCode::Add,
        vec![IR::Identifier("n".into()), IR::Identifier("factor".into())],
    );
    let body_ir = freeze::encode(&vec![body]).unwrap();

    write_message(
        &mut writer,
        &WorkerCommand::Execute {
            encoded_ir: body_ir,
            captures: vec![("factor".into(), FrozenValue::Int(100))],
            param_names: vec!["n".into()],
            seed: FrozenValue::Int(7),
        },
    )
    .unwrap();

    let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
    match result {
        WorkerResult::Ok(FrozenValue::Int(107)) => {}
        other => panic!("expected Ok(Int(107)), got {:?}", other),
    }

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    child.wait().unwrap();
}

#[test]
fn test_worker_multiple_execute() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    let body = IR::op(IROpCode::Mul, vec![IR::Identifier("n".into()), IR::Int(3)]);
    let body_ir = freeze::encode(&vec![body]).unwrap();

    for i in 1..=5 {
        write_message(
            &mut writer,
            &WorkerCommand::Execute {
                encoded_ir: body_ir.clone(),
                captures: vec![],
                param_names: vec!["n".into()],
                seed: FrozenValue::Int(i),
            },
        )
        .unwrap();

        let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
        match result {
            WorkerResult::Ok(FrozenValue::Int(v)) => assert_eq!(v, i * 3),
            other => panic!("iteration {}: expected Ok(Int({})), got {:?}", i, i * 3, other),
        }
    }

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    child.wait().unwrap();
}

#[test]
fn test_worker_float_result() {
    let (mut writer, mut reader, mut child) = spawn_worker();

    // Body: n / 2 (true division returns float)
    let body = IR::op(IROpCode::Div, vec![IR::Identifier("n".into()), IR::Int(2)]);
    let body_ir = freeze::encode(&vec![body]).unwrap();

    write_message(
        &mut writer,
        &WorkerCommand::Execute {
            encoded_ir: body_ir,
            captures: vec![],
            param_names: vec!["n".into()],
            seed: FrozenValue::Int(7),
        },
    )
    .unwrap();

    let result: WorkerResult = read_message(&mut reader).unwrap().unwrap();
    match result {
        WorkerResult::Ok(FrozenValue::Float(f)) => assert!((f - 3.5).abs() < 1e-10),
        other => panic!("expected Ok(Float(3.5)), got {:?}", other),
    }

    write_message(&mut writer, &WorkerCommand::Shutdown).unwrap();
    child.wait().unwrap();
}

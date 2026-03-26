// FILE: catnip_core/src/freeze/worker.rs
//! IPC protocol for ND worker processes.
//!
//! Length-prefixed bincode messages over stdin/stdout pipes.
//! Used by `catnip worker` subcommand.

use super::FrozenValue;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Max allowed message size on the wire (64 MB).
const MAX_MESSAGE_SIZE: u32 = 64 * 1024 * 1024;

// -- Protocol messages --

/// Command sent from parent to worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerCommand {
    /// Execute a frozen lambda body with captures and a seed value.
    /// `encoded_ir` contains raw bincode-encoded `Vec<IR>` (no .catf header).
    /// `param_names` are the lambda's parameter names (e.g. ["n", "recur"]).
    /// The first param is bound to `seed`, the second (if any, `recur`) is unused
    /// in the worker (ND recursion is handled sequentially in worker processes).
    Execute {
        encoded_ir: Vec<u8>,
        captures: Vec<(String, FrozenValue)>,
        param_names: Vec<String>,
        seed: FrozenValue,
    },
    /// Graceful shutdown.
    Shutdown,
    /// Liveness check.
    Ping,
}

/// Result sent from worker to parent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerResult {
    /// Successful execution with frozen return value.
    Ok(FrozenValue),
    /// Execution error (message string).
    Err(String),
    /// Response to Ping.
    Pong,
}

// -- Framing: [u32 LE length][bincode payload] --

/// Write a length-prefixed bincode message.
pub fn write_message<W: Write, T: Serialize>(writer: &mut W, msg: &T) -> io::Result<()> {
    let payload = bincode::serde::encode_to_vec(msg, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

/// Read a length-prefixed bincode message. Returns None on EOF.
pub fn read_message<R: Read, T: for<'de> Deserialize<'de>>(reader: &mut R) -> io::Result<Option<T>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_MESSAGE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {} bytes (max {})", len, MAX_MESSAGE_SIZE),
        ));
    }
    let len = len as usize;

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;

    let (msg, _): (T, _) = bincode::serde::decode_from_slice(&payload, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::IR;
    use std::io::Cursor;

    #[test]
    fn test_roundtrip_ping_pong() {
        let mut buf = Vec::new();
        write_message(&mut buf, &WorkerCommand::Ping).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg: WorkerCommand = read_message(&mut cursor).unwrap().unwrap();
        assert!(matches!(msg, WorkerCommand::Ping));
    }

    #[test]
    fn test_roundtrip_execute() {
        // Create frozen IR bytes via encode (raw bincode, no header)
        let ir = vec![
            IR::Int(42),
            IR::op(crate::ir::IROpCode::Add, vec![IR::Int(1), IR::Int(2)]),
        ];
        let encoded_ir = crate::freeze::encode(&ir).unwrap();

        let cmd = WorkerCommand::Execute {
            encoded_ir,
            captures: vec![("x".into(), FrozenValue::Int(10))],
            param_names: vec!["n".into(), "recur".into()],
            seed: FrozenValue::Float(3.14),
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &cmd).unwrap();
        let mut cursor = Cursor::new(buf);
        let decoded: WorkerCommand = read_message(&mut cursor).unwrap().unwrap();
        match decoded {
            WorkerCommand::Execute {
                encoded_ir,
                captures,
                param_names,
                seed,
            } => {
                assert!(!encoded_ir.is_empty());
                assert_eq!(captures.len(), 1);
                assert_eq!(captures[0].0, "x");
                assert_eq!(param_names, vec!["n", "recur"]);
                assert!(matches!(seed, FrozenValue::Float(f) if (f - 3.14).abs() < 1e-10));
            }
            _ => panic!("expected Execute"),
        }
    }

    #[test]
    fn test_roundtrip_result_ok() {
        let result = WorkerResult::Ok(FrozenValue::List(vec![
            FrozenValue::Int(1),
            FrozenValue::String("hello".into()),
        ]));
        let mut buf = Vec::new();
        write_message(&mut buf, &result).unwrap();
        let mut cursor = Cursor::new(buf);
        let decoded: WorkerResult = read_message(&mut cursor).unwrap().unwrap();
        assert!(matches!(decoded, WorkerResult::Ok(FrozenValue::List(_))));
    }

    #[test]
    fn test_roundtrip_result_err() {
        let result = WorkerResult::Err("something failed".into());
        let mut buf = Vec::new();
        write_message(&mut buf, &result).unwrap();
        let mut cursor = Cursor::new(buf);
        let decoded: WorkerResult = read_message(&mut cursor).unwrap().unwrap();
        match decoded {
            WorkerResult::Err(msg) => assert_eq!(msg, "something failed"),
            _ => panic!("expected Err"),
        }
    }

    #[test]
    fn test_roundtrip_shutdown() {
        let mut buf = Vec::new();
        write_message(&mut buf, &WorkerCommand::Shutdown).unwrap();
        let mut cursor = Cursor::new(buf);
        let msg: WorkerCommand = read_message(&mut cursor).unwrap().unwrap();
        assert!(matches!(msg, WorkerCommand::Shutdown));
    }

    #[test]
    fn test_eof_returns_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let result: Option<WorkerCommand> = read_message(&mut cursor).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_multiple_messages() {
        let mut buf = Vec::new();
        write_message(&mut buf, &WorkerCommand::Ping).unwrap();
        let frozen = crate::freeze::encode(&vec![IR::Int(1)]).unwrap();
        write_message(
            &mut buf,
            &WorkerCommand::Execute {
                encoded_ir: frozen,
                captures: vec![],
                param_names: vec!["n".into()],
                seed: FrozenValue::None,
            },
        )
        .unwrap();
        write_message(&mut buf, &WorkerCommand::Shutdown).unwrap();

        let mut cursor = Cursor::new(buf);
        assert!(matches!(
            read_message::<_, WorkerCommand>(&mut cursor).unwrap().unwrap(),
            WorkerCommand::Ping
        ));
        assert!(matches!(
            read_message::<_, WorkerCommand>(&mut cursor).unwrap().unwrap(),
            WorkerCommand::Execute { .. }
        ));
        assert!(matches!(
            read_message::<_, WorkerCommand>(&mut cursor).unwrap().unwrap(),
            WorkerCommand::Shutdown
        ));
        assert!(read_message::<_, WorkerCommand>(&mut cursor).unwrap().is_none());
    }
}

// FILE: catnip_core/src/freeze/mod.rs
//! Binary freeze/thaw for Catnip IR and data values.
//!
//! Format `.catf`: fixed header + bincode v2 payload.
//! Header is manually serialized (stable across bincode versions).
//! Stale files (opcode mismatch) are silently rejected, not errors.

pub mod value;
pub mod worker;

pub use value::FrozenValue;

use crate::ir::{IR, IROpCode};
use std::fmt;

// -- Constants --

pub const MAGIC: [u8; 4] = *b"CATF";
pub const KIND_CODE: u8 = 0;
pub const KIND_DATA: u8 = 1;

/// Bump when serialization semantics change without modifying the opcode enum.
const FORMAT_SALT: u32 = 1;

const FLAG_HAS_SOURCE_HASH: u8 = 1;

// Header sizes
const HEADER_BASE: usize = 4 + 1 + 4 + 1; // magic + kind + opcode_hash + flags = 10
const SOURCE_HASH_SIZE: usize = 8;
const LEVEL_SIZE: usize = 1;

// -- Errors --

#[derive(Debug)]
pub enum FreezeError {
    InvalidMagic,
    TruncatedHeader,
    WrongKind { expected: u8, got: u8 },
    SerializationError(String),
}

impl fmt::Display for FreezeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "not a .catf file"),
            Self::TruncatedHeader => write!(f, "truncated .catf header"),
            Self::WrongKind { expected, got } => {
                write!(f, "expected kind {expected}, got {got}")
            }
            Self::SerializationError(msg) => write!(f, "serialization error: {msg}"),
        }
    }
}

impl std::error::Error for FreezeError {}

// -- Results --

#[derive(Debug)]
pub enum ThawResult {
    Ok { ir: Vec<IR>, level: u8 },
    Stale,
}

#[derive(Debug)]
pub enum ThawDataResult {
    Ok(FrozenValue),
    Stale,
}

// -- Opcode hash --

/// Compute opcode hash for format validation.
/// Changes when opcodes are added/removed (IROpCode::MAX) or
/// when FORMAT_SALT is bumped for semantic changes.
pub fn compute_opcode_hash() -> u32 {
    IROpCode::MAX as u32 + FORMAT_SALT
}

// -- Header --

fn write_header(buf: &mut Vec<u8>, kind: u8, source_hash: Option<u64>, level: u8) {
    buf.extend_from_slice(&MAGIC);
    buf.push(kind);
    buf.extend_from_slice(&compute_opcode_hash().to_le_bytes());
    let flags = if source_hash.is_some() { FLAG_HAS_SOURCE_HASH } else { 0 };
    buf.push(flags);
    if let Some(hash) = source_hash {
        buf.extend_from_slice(&hash.to_le_bytes());
    }
    buf.push(level);
}

struct HeaderInfo {
    kind: u8,
    opcode_hash: u32,
    source_hash: Option<u64>,
    level: u8,
    payload_offset: usize,
}

fn read_header(data: &[u8]) -> Result<HeaderInfo, FreezeError> {
    if data.len() < HEADER_BASE {
        return Err(FreezeError::TruncatedHeader);
    }
    if data[0..4] != MAGIC {
        return Err(FreezeError::InvalidMagic);
    }

    let kind = data[4];
    let opcode_hash = u32::from_le_bytes(data[5..9].try_into().unwrap());
    let flags = data[9];

    let mut offset = HEADER_BASE;
    let source_hash = if flags & FLAG_HAS_SOURCE_HASH != 0 {
        if data.len() < offset + SOURCE_HASH_SIZE {
            return Err(FreezeError::TruncatedHeader);
        }
        let hash = u64::from_le_bytes(data[offset..offset + SOURCE_HASH_SIZE].try_into().unwrap());
        offset += SOURCE_HASH_SIZE;
        Some(hash)
    } else {
        None
    };

    if data.len() < offset + LEVEL_SIZE {
        return Err(FreezeError::TruncatedHeader);
    }
    let level = data[offset];
    offset += LEVEL_SIZE;

    Ok(HeaderInfo {
        kind,
        opcode_hash,
        source_hash,
        level,
        payload_offset: offset,
    })
}

// -- Raw bincode (no header) --
// Used for transport (IPC workers, in-memory). No `.catf` header overhead.

/// Encode any serializable value to bincode bytes.
pub fn encode<T: serde::Serialize + ?Sized>(val: &T) -> Result<Vec<u8>, FreezeError> {
    bincode::serde::encode_to_vec(val, bincode::config::standard())
        .map_err(|e| FreezeError::SerializationError(e.to_string()))
}

/// Decode bincode bytes to a value.
pub fn decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, FreezeError> {
    let (val, _): (T, _) = bincode::serde::decode_from_slice(data, bincode::config::standard())
        .map_err(|e| FreezeError::SerializationError(e.to_string()))?;
    Ok(val)
}

// -- .catf file format (header + bincode payload) --
// Used for disk persistence (cache, memoization). Header enables stale detection.

/// Freeze IR code to `.catf` binary (header + bincode).
pub fn freeze_code(ir: &[IR], source: &str, level: u8) -> Result<Vec<u8>, FreezeError> {
    let source_hash = xxhash_rust::xxh64::xxh64(source.as_bytes(), 0);
    let mut buf = Vec::new();
    write_header(&mut buf, KIND_CODE, Some(source_hash), level);
    buf.extend_from_slice(&encode(ir)?);
    Ok(buf)
}

/// Thaw IR code from `.catf` binary. Returns Stale on opcode hash mismatch.
pub fn thaw_code(data: &[u8]) -> Result<ThawResult, FreezeError> {
    let header = read_header(data)?;
    if header.kind != KIND_CODE {
        return Err(FreezeError::WrongKind {
            expected: KIND_CODE,
            got: header.kind,
        });
    }
    if header.opcode_hash != compute_opcode_hash() {
        return Ok(ThawResult::Stale);
    }
    let ir: Vec<IR> = decode(&data[header.payload_offset..])?;
    Ok(ThawResult::Ok {
        ir,
        level: header.level,
    })
}

/// Freeze a value to `.catf` binary (header + bincode).
pub fn freeze_data(value: &FrozenValue) -> Result<Vec<u8>, FreezeError> {
    let mut buf = Vec::new();
    write_header(&mut buf, KIND_DATA, None, 0);
    buf.extend_from_slice(&encode(value)?);
    Ok(buf)
}

/// Thaw a value from `.catf` binary. Returns Stale on opcode hash mismatch.
pub fn thaw_data(data: &[u8]) -> Result<ThawDataResult, FreezeError> {
    let header = read_header(data)?;
    if header.kind != KIND_DATA {
        return Err(FreezeError::WrongKind {
            expected: KIND_DATA,
            got: header.kind,
        });
    }
    if header.opcode_hash != compute_opcode_hash() {
        return Ok(ThawDataResult::Stale);
    }
    let value: FrozenValue = decode(&data[header.payload_offset..])?;
    Ok(ThawDataResult::Ok(value))
}

// -- Utilities --

/// Check if frozen data matches a source text (without full deserialization).
pub fn source_matches(data: &[u8], source: &str) -> Result<bool, FreezeError> {
    let header = read_header(data)?;
    match header.source_hash {
        Some(stored) => {
            let expected = xxhash_rust::xxh64::xxh64(source.as_bytes(), 0);
            Ok(stored == expected)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::IROpCode;

    fn sample_ir() -> Vec<IR> {
        vec![IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)])]
    }

    #[test]
    fn test_encode_decode_ir() {
        let ir = sample_ir();
        let bytes = encode(&ir).unwrap();
        let decoded: Vec<IR> = decode(&bytes).unwrap();
        assert_eq!(decoded, ir);
    }

    #[test]
    fn test_encode_slice_vs_vec() {
        let ir = vec![IR::Int(42)];
        let from_vec = encode(&ir).unwrap();
        let from_slice = encode(ir.as_slice()).unwrap();
        assert_eq!(from_vec, from_slice, "slice and vec should encode identically");
        let decoded: Vec<IR> = decode(&from_slice).unwrap();
        assert_eq!(decoded, ir);
    }

    #[test]
    fn test_encode_array_vs_vec_differ() {
        // bincode 2 encodes [T; N] without length prefix (size known at compile time)
        // but Vec<T> with a varint length prefix. Always use Vec for decode::<Vec<T>> compat.
        let ir = vec![IR::Identifier("n".into())];
        let bytes_vec = encode(&ir).unwrap();
        let bytes_array = encode(&[IR::Identifier("n".into())]).unwrap();
        assert_ne!(
            bytes_vec.len(),
            bytes_array.len(),
            "Vec and [T;1] should differ in size"
        );
        // Vec roundtrips correctly
        let decoded: Vec<IR> = decode(&bytes_vec).unwrap();
        assert_eq!(decoded, ir);
    }

    #[test]
    fn test_encode_decode_frozen_value() {
        let val = FrozenValue::List(vec![FrozenValue::Int(1), FrozenValue::String("x".into())]);
        let bytes = encode(&val).unwrap();
        let decoded: FrozenValue = decode(&bytes).unwrap();
        assert_eq!(decoded, val);
    }

    #[test]
    fn test_roundtrip_code() {
        let ir = sample_ir();
        let frozen = freeze_code(&ir, "2 + 3", 2).unwrap();
        match thaw_code(&frozen).unwrap() {
            ThawResult::Ok { ir: thawed, level } => {
                assert_eq!(thawed, ir);
                assert_eq!(level, 2);
            }
            ThawResult::Stale => panic!("unexpected Stale"),
        }
    }

    #[test]
    fn test_roundtrip_data() {
        let value = FrozenValue::Dict(vec![
            (FrozenValue::String("x".into()), FrozenValue::Int(42)),
            (
                FrozenValue::String("y".into()),
                FrozenValue::List(vec![FrozenValue::Float(1.5), FrozenValue::None]),
            ),
        ]);
        let frozen = freeze_data(&value).unwrap();
        match thaw_data(&frozen).unwrap() {
            ThawDataResult::Ok(thawed) => assert_eq!(thawed, value),
            ThawDataResult::Stale => panic!("unexpected Stale"),
        }
    }

    #[test]
    fn test_stale_opcode_hash() {
        let ir = sample_ir();
        let mut frozen = freeze_code(&ir, "2 + 3", 1).unwrap();
        // Corrupt opcode hash (bytes 5..9)
        frozen[5] = 0xFF;
        frozen[6] = 0xFF;
        assert!(matches!(thaw_code(&frozen).unwrap(), ThawResult::Stale));
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"NOPE_extra_bytes_here";
        assert!(matches!(thaw_code(data), Err(FreezeError::InvalidMagic)));
    }

    #[test]
    fn test_truncated_header() {
        assert!(matches!(thaw_code(b"CAT"), Err(FreezeError::TruncatedHeader)));
    }

    #[test]
    fn test_wrong_kind() {
        let frozen = freeze_data(&FrozenValue::Int(1)).unwrap();
        match thaw_code(&frozen) {
            Err(FreezeError::WrongKind { expected, got }) => {
                assert_eq!(expected, KIND_CODE);
                assert_eq!(got, KIND_DATA);
            }
            other => panic!("expected WrongKind, got {:?}", other),
        }
    }

    #[test]
    fn test_source_matches() {
        let ir = sample_ir();
        let frozen = freeze_code(&ir, "2 + 3", 1).unwrap();
        assert!(source_matches(&frozen, "2 + 3").unwrap());
        assert!(!source_matches(&frozen, "2 + 4").unwrap());
    }

    #[test]
    fn test_source_matches_data_returns_false() {
        let frozen = freeze_data(&FrozenValue::None).unwrap();
        assert!(!source_matches(&frozen, "anything").unwrap());
    }

    #[test]
    fn test_opcode_hash_deterministic() {
        assert_eq!(compute_opcode_hash(), compute_opcode_hash());
    }

    #[test]
    fn test_empty_ir() {
        let frozen = freeze_code(&[], "", 1).unwrap();
        match thaw_code(&frozen).unwrap() {
            ThawResult::Ok { ir, level } => {
                assert!(ir.is_empty());
                assert_eq!(level, 1);
            }
            ThawResult::Stale => panic!("unexpected Stale"),
        }
    }

    #[test]
    fn test_complex_ir() {
        let ir = vec![
            IR::op(
                IROpCode::SetLocals,
                vec![
                    IR::List(vec![IR::Ref("x".into(), 0, 1)]),
                    IR::op(IROpCode::Add, vec![IR::Int(2), IR::Int(3)]),
                    IR::Bool(false),
                ],
            ),
            IR::op(
                IROpCode::OpIf,
                vec![
                    IR::Tuple(vec![IR::Tuple(vec![IR::Bool(true), IR::String("yes".into())])]),
                    IR::String("no".into()),
                ],
            ),
        ];
        let frozen = freeze_code(&ir, "x = 2 + 3; if (true) { \"yes\" } else { \"no\" }", 2).unwrap();
        match thaw_code(&frozen).unwrap() {
            ThawResult::Ok { ir: thawed, level } => {
                assert_eq!(thawed, ir);
                assert_eq!(level, 2);
            }
            ThawResult::Stale => panic!("unexpected Stale"),
        }
    }

    #[test]
    fn test_data_no_source_hash_in_header() {
        let frozen = freeze_data(&FrozenValue::Int(42)).unwrap();
        let header = read_header(&frozen).unwrap();
        assert_eq!(header.kind, KIND_DATA);
        assert!(header.source_hash.is_none());
        assert_eq!(header.level, 0);
    }

    #[test]
    fn test_code_has_source_hash_in_header() {
        let frozen = freeze_code(&[], "test", 1).unwrap();
        let header = read_header(&frozen).unwrap();
        assert_eq!(header.kind, KIND_CODE);
        assert!(header.source_hash.is_some());
        assert_eq!(header.level, 1);
    }
}

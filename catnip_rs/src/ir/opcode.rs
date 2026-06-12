// FILE: catnip_rs/src/ir/opcode.rs
pub use catnip_core::ir::opcode::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PY_MOD_SEMANTIC_OPCODE;
    use pyo3::prelude::*;

    /// The generated Python OpCode enum must stay in sync with IROpCode.
    /// If this test fails, run: make gen-opcodes
    #[test]
    fn test_opcode_values_match_python() {
        let pairs: &[(&str, IROpCode)] = &[
            ("PRAGMA", IROpCode::Pragma),
            ("GETATTR", IROpCode::GetAttr),
            ("SETATTR", IROpCode::SetAttr),
            ("SETITEM", IROpCode::SetItem),
            ("SET_LOCALS", IROpCode::SetLocals),
            ("OP_IF", IROpCode::OpIf),
            ("OP_WHILE", IROpCode::OpWhile),
            ("OP_FOR", IROpCode::OpFor),
            ("OP_MATCH", IROpCode::OpMatch),
            ("OP_LAMBDA", IROpCode::OpLambda),
            ("OP_BLOCK", IROpCode::OpBlock),
            ("OP_RETURN", IROpCode::OpReturn),
            ("FSTRING", IROpCode::Fstring),
            ("CALL", IROpCode::Call),
        ];
        Python::initialize();
        Python::attach(|py| {
            let opcode_class = py.import(PY_MOD_SEMANTIC_OPCODE).unwrap().getattr("OpCode").unwrap();
            for (name, opcode) in pairs {
                assert_eq!(
                    opcode_class.getattr(*name).unwrap().extract::<i32>().unwrap(),
                    *opcode as i32,
                    "{} opcode mismatch -- run `make gen-opcodes`",
                    name
                );
            }
        });
    }
}

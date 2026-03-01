// FILE: catnip_tools/src/token.rs
/// A token extracted from the parse tree for formatting
#[derive(Clone, Debug)]
pub struct Token {
    pub type_: String,
    pub value: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

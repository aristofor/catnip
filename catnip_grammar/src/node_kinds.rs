// FILE: catnip_grammar/src/node_kinds.rs
//
// Centralized tree-sitter node kind constants. Avoids hardcoded string
// literals scattered across crates and catches grammar renames at compile
// time (one place to update).

// Statements
pub const SOURCE_FILE: &str = "source_file";
pub const ASSIGNMENT: &str = "assignment";
pub const STRUCT_STMT: &str = "struct_stmt";
pub const TRAIT_STMT: &str = "trait_stmt";
pub const FOR_STMT: &str = "for_stmt";
pub const WHILE_STMT: &str = "while_stmt";
pub const RETURN_STMT: &str = "return_stmt";
pub const IF_EXPR: &str = "if_expr";
pub const ELIF_CLAUSE: &str = "elif_clause";
pub const ELSE_CLAUSE: &str = "else_clause";
pub const MATCH_EXPR: &str = "match_expr";
pub const MATCH_CASE: &str = "match_case";
pub const BLOCK: &str = "block";
pub const STATEMENT: &str = "statement";

// Struct/trait
pub const STRUCT_IMPLEMENTS: &str = "struct_implements";
pub const STRUCT_EXTENDS: &str = "struct_extends";
pub const TRAIT_EXTENDS: &str = "trait_extends";

// Functions
pub const LAMBDA_EXPR: &str = "lambda_expr";
pub const LAMBDA_PARAMS: &str = "lambda_params";
pub const LAMBDA_PARAM: &str = "lambda_param";
pub const CALL: &str = "call";

// Access
pub const CHAINED: &str = "chained";
pub const GETATTR: &str = "getattr";
pub const CALLATTR: &str = "callattr";
pub const SETATTR: &str = "setattr";
pub const INDEX: &str = "index";

// Patterns
pub const PATTERN: &str = "pattern";
pub const PATTERN_VAR: &str = "pattern_var";
pub const PATTERN_LITERAL: &str = "pattern_literal";
pub const PATTERN_WILDCARD: &str = "pattern_wildcard";
pub const PATTERN_OR: &str = "pattern_or";
pub const PATTERN_TUPLE: &str = "pattern_tuple";
pub const PATTERN_STAR: &str = "pattern_star";
pub const PATTERN_STRUCT: &str = "pattern_struct";

// Literals & identifiers
pub const IDENTIFIER: &str = "identifier";
pub const LITERAL: &str = "literal";
pub const TRUE: &str = "true";
pub const FALSE: &str = "false";
pub const NONE: &str = "none";
pub const INTEGER: &str = "integer";
pub const FLOAT: &str = "float";
pub const DECIMAL: &str = "decimal";
pub const IMAGINARY: &str = "imaginary";
pub const STRING: &str = "string";
pub const FSTRING: &str = "fstring";
pub const BSTRING: &str = "bstring";
pub const COMMENT: &str = "comment";

// Misc
pub const DECORATOR: &str = "decorator";
pub const KWARG: &str = "kwarg";
pub const DICT_KWARG: &str = "dict_kwarg";
pub const ARGUMENTS: &str = "arguments";
pub const VARIADIC_PARAM: &str = "variadic_param";
pub const COMPARISON: &str = "comparison";
pub const COMP_OP: &str = "comp_op";

// Unpack
pub const LVALUE: &str = "lvalue";
pub const UNPACK_TARGET: &str = "unpack_target";
pub const UNPACK_TUPLE: &str = "unpack_tuple";
pub const UNPACK_SEQUENCE: &str = "unpack_sequence";
pub const UNPACK_ITEMS: &str = "unpack_items";

// Operators (non-terminal node kinds used by formatter)
pub const ADD_SUB_OP: &str = "add_sub_op";
pub const MUL_DIV_OP: &str = "mul_div_op";
pub const SHIFT_OP: &str = "shift_op";

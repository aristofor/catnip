// FILE: catnip_core/src/parser/pure_transforms.rs
//! Pure Rust transformers - Tree-sitter → IR (no Python dependencies)
//!
//! Port of the existing transformers for the standalone pipeline.

use super::utils::{named_children, node_text, unescape_bytes, unescape_string};
use crate::ir::{BroadcastType, IR, IROpCode};
use indexmap::IndexMap;
use tree_sitter::Node;

/// Result type for transform operations
pub type TransformResult = Result<IR, String>;

/// Parse arguments from an `arguments` tree-sitter node into (args, kwargs).
///
/// Handles all grammar node types: args, kwargs, args_kwargs, kwarg, keyword_argument.
fn parse_arguments(arguments_node: &Node, source: &str) -> Result<(Vec<IR>, IndexMap<String, IR>), String> {
    let mut args = Vec::new();
    let mut kwargs = IndexMap::new();

    for arg_child in named_children(arguments_node) {
        let arg_kind = arg_child.kind();

        if arg_kind == "args_kwargs" {
            for item in named_children(&arg_child) {
                let item_kind = item.kind();
                if item_kind == "kwarg" || item_kind == "keyword_argument" {
                    let grandchildren: Vec<_> = named_children(&item);
                    if grandchildren.len() == 2 {
                        let key = node_text(&grandchildren[0], source).to_string();
                        let value = transform(grandchildren[1], source)?;
                        kwargs.insert(key, value);
                    }
                } else {
                    args.push(transform(item, source)?);
                }
            }
        } else if arg_kind == "kwargs" {
            for kwarg_item in named_children(&arg_child) {
                if kwarg_item.kind() == "kwarg" || kwarg_item.kind() == "keyword_argument" {
                    let grandchildren: Vec<_> = named_children(&kwarg_item);
                    if grandchildren.len() == 2 {
                        let key = node_text(&grandchildren[0], source).to_string();
                        let value = transform(grandchildren[1], source)?;
                        kwargs.insert(key, value);
                    }
                }
            }
        } else if arg_kind == "kwarg" || arg_kind == "keyword_argument" {
            let grandchildren: Vec<_> = named_children(&arg_child);
            if grandchildren.len() == 2 {
                let key = node_text(&grandchildren[0], source).to_string();
                let value = transform(grandchildren[1], source)?;
                kwargs.insert(key, value);
            }
        } else if arg_kind == "args" {
            for inner_arg in named_children(&arg_child) {
                args.push(transform(inner_arg, source)?);
            }
        } else {
            args.push(transform(arg_child, source)?);
        }
    }

    Ok((args, kwargs))
}

/// Main transform dispatcher (standalone)
pub fn transform(node: Node, source: &str) -> TransformResult {
    let kind = node.kind();

    match kind {
        // Literals
        "number" | "integer" => transform_number(node, source),
        "string" => transform_string(node, source),
        "bstring" => transform_bstring(node, source),
        "fstring" => transform_fstring(node, source),
        "true" | "false" => transform_bool(node, source),
        "none" => Ok(IR::None),
        "nd_empty_topos" => transform_nd_empty_topos(node, source),

        // Operators MVP
        "additive" => transform_additive(node, source),
        "multiplicative" => transform_multiplicative(node, source),
        "exponent" => transform_exponent(node, source),
        "unary" => transform_unary(node, source),
        "comparison" => transform_comparison(node, source),
        "bool_and" => transform_bool_and(node, source),
        "bool_or" => transform_bool_or(node, source),
        "null_coalesce" => transform_null_coalesce(node, source),
        "bool_not" => transform_bool_not(node, source),
        "bit_and" => transform_bit_and(node, source),
        "bit_or" => transform_bit_or(node, source),
        "bit_xor" => transform_bit_xor(node, source),
        "bit_not" => transform_bit_not(node, source),
        "shift" => transform_shift(node, source),

        // Control flow MVP
        "if_expr" => transform_if_expr(node, source),
        "elif_clause" => transform_elif_clause(node, source),
        "else_clause" => transform_else_clause(node, source),
        "while_stmt" => transform_while_stmt(node, source),
        "for_stmt" => transform_for_stmt(node, source),
        "return_stmt" => transform_return_stmt(node, source),
        "break_stmt" => transform_break_stmt(node, source),
        "continue_stmt" => transform_continue_stmt(node, source),
        "match_expr" => transform_match_expr(node, source),
        "match_case" => transform_match_case(node, source),

        // Error handling
        "try_stmt" => transform_try_stmt(node, source),
        "raise_stmt" => transform_raise_stmt(node, source),

        // Context managers
        "with_stmt" => transform_with_stmt(node, source),

        // Pattern matching
        "pattern" => transform_pattern(node, source),
        "pattern_literal" => transform_pattern_literal(node, source),
        "pattern_var" => transform_pattern_var(node, source),
        "pattern_wildcard" => transform_pattern_wildcard(node, source),
        "pattern_or" => transform_pattern_or(node, source),
        "pattern_tuple" => transform_pattern_tuple(node, source),
        "pattern_star" => transform_pattern_star(node, source),
        "pattern_struct" => transform_pattern_struct(node, source),
        "pattern_enum" => transform_pattern_enum(node, source),

        // Variables
        "identifier" => transform_identifier(node, source),
        "assignment" => transform_assignment(node, source),
        "setattr" => transform_setattr_lvalue(node, source),
        "variadic_param" => transform_variadic_param(node, source),
        "unpack_items" => transform_unpack_items(node, source),

        // ND operations
        "nd_recursion" => transform_nd_recursion(node, source),
        "nd_map" => transform_nd_map(node, source),

        // Collections
        "bracket_list" => transform_collection_ir(node, source, IROpCode::ListLiteral, "__catnip_spread_list"),
        "bracket_dict" => transform_bracket_dict(node, source),
        "list_literal" => transform_collection_ir(node, source, IROpCode::ListLiteral, "__catnip_spread_list"),
        "tuple_literal" => transform_collection_ir(node, source, IROpCode::TupleLiteral, "__catnip_spread_tuple"),
        "dict_literal" => transform_dict_literal(node, source),
        "set_literal" => transform_collection_ir(node, source, IROpCode::SetLiteral, "__catnip_spread_set"),

        // Function call & Lambda
        "call" => transform_call(node, source),
        "lambda_expr" => transform_lambda_expr(node, source),

        // Pragma directive
        "pragma_stmt" => transform_pragma_stmt(node, source),
        "pragma_qualified" => transform_pragma_qualified(node, source),

        // Struct
        "struct_stmt" => transform_struct_stmt(node, source),

        // Trait
        "trait_stmt" => transform_trait_stmt(node, source),

        // Enum
        "enum_stmt" => transform_enum_stmt(node, source),

        // Union (tagged union / ADT)
        "union_stmt" => transform_union_stmt(node, source),

        // Block
        "block" => transform_block(node, source),

        // Access & Broadcasting
        "index" => transform_index(node, source),
        "slice_range" => transform_slice_range(node, source),
        "fullslice" => transform_fullslice(node, source),
        "broadcast" => transform_broadcast(node, source),

        // Chained operations (call_member, getattr, index, broadcast)
        "chained" => transform_chained(node, source),

        // Statement (detect import statement pattern)
        "statement" => transform_statement(node, source),

        // Source file
        "source_file" => transform_source_file(node, source),

        // Fallback: transform children
        _ => transform_children(node, source),
    }
}

mod access;
mod broadcast;
mod calls;
mod collections;
mod control_flow;
mod definitions;
mod literals;
mod operators;
mod patterns;
mod source_file;
mod variables;
mod with_stmt;
use access::*;
use broadcast::*;
use calls::*;
use collections::*;
use control_flow::*;
use definitions::*;
use literals::*;
use operators::*;
use patterns::*;
use source_file::*;
use variables::*;
use with_stmt::*;

#[cfg(test)]
mod tests;

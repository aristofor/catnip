// FILE: catnip_core/src/parser/pure_transforms/source_file.rs
use super::*;

// ============================================================================
// Source file
// ============================================================================

pub(crate) fn transform_source_file(node: Node, source: &str) -> TransformResult {
    let children = named_children(&node);
    let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();

    // Return as Program (top-level statement sequence)
    Ok(IR::Program(transformed?))
}

// ============================================================================
// Helpers
// ============================================================================

pub(crate) fn transform_children(node: Node, source: &str) -> TransformResult {
    // named_children already filters out comments
    let children = named_children(&node);

    if children.is_empty() {
        Ok(IR::None)
    } else if children.len() == 1 {
        transform(children[0], source)
    } else {
        let transformed: Result<Vec<_>, _> = children.iter().map(|c| transform(*c, source)).collect();
        Ok(IR::Tuple(transformed?))
    }
}

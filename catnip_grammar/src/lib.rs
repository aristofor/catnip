use tree_sitter::Language;

extern "C" {
    fn tree_sitter_catnip() -> Language;
}

/// Get the Catnip tree-sitter Language.
pub fn get_language() -> Language {
    unsafe { tree_sitter_catnip() }
}

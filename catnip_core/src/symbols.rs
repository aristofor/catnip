// FILE: catnip_core/src/symbols.rs
//! Bidirectional string interning and shared enum helpers.

use std::collections::HashMap;

/// Maximum number of variants per enum type (u16 range).
pub const MAX_VARIANTS_PER_ENUM: usize = u16::MAX as usize;

/// Build a qualified enum variant name: "EnumName.variant".
#[inline]
pub fn qualified_name(enum_name: &str, variant_name: &str) -> String {
    format!("{}.{}", enum_name, variant_name)
}

/// Global symbol table for interning strings to u32 indices.
#[derive(Debug, Default)]
pub struct SymbolTable {
    names: Vec<String>,
    index: HashMap<String, u32>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a string, returning its stable index.
    #[inline]
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(&idx) = self.index.get(name) {
            return idx;
        }
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.index.insert(name.to_string(), idx);
        idx
    }

    /// Look up a name, returning its index if already interned.
    #[inline]
    pub fn lookup(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Resolve an index back to its name.
    #[inline]
    pub fn resolve(&self, idx: u32) -> Option<&str> {
        self.names.get(idx as usize).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_table_intern() {
        let mut st = SymbolTable::new();
        let a = st.intern("Color.red");
        let b = st.intern("Color.blue");
        let c = st.intern("Color.red");
        assert_eq!(a, c);
        assert_ne!(a, b);
        assert_eq!(st.resolve(a), Some("Color.red"));
        assert_eq!(st.resolve(b), Some("Color.blue"));
    }

    #[test]
    fn test_qualified_name() {
        assert_eq!(qualified_name("Color", "red"), "Color.red");
        assert_eq!(qualified_name("Direction", "up"), "Direction.up");
    }
}

// FILE: catnip_vm/src/vm/enums.rs
//! Enum type system for PureVM.

use std::collections::HashMap;

use crate::value::Value;

pub use catnip_core::symbols::SymbolTable;
use catnip_core::symbols::{MAX_VARIANTS_PER_ENUM, qualified_name};

/// A single enum type definition.
#[derive(Debug, Clone)]
pub struct PureEnumType {
    pub id: u32,
    pub name: String,
    /// (variant_name, symbol_id) in declaration order.
    pub variants: Vec<(String, u32)>,
}

impl PureEnumType {
    #[inline]
    pub fn variant_symbol(&self, variant_name: &str) -> Option<u32> {
        self.variants
            .iter()
            .find(|(n, _)| n == variant_name)
            .map(|(_, sid)| *sid)
    }
}

/// Registry of all enum types with reverse symbol lookup.
#[derive(Debug, Default)]
pub struct PureEnumRegistry {
    types: Vec<PureEnumType>,
    symbol_to_enum: HashMap<u32, (u32, u16)>,
    name_to_id: HashMap<String, u32>,
}

impl PureEnumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: &str, variant_names: &[String], symbols: &mut SymbolTable) -> u32 {
        assert!(
            variant_names.len() <= MAX_VARIANTS_PER_ENUM,
            "enum '{}' has {} variants, max is {}",
            name,
            variant_names.len(),
            MAX_VARIANTS_PER_ENUM,
        );
        let type_id = self.types.len() as u32;
        let mut variants = Vec::with_capacity(variant_names.len());

        for (i, vname) in variant_names.iter().enumerate() {
            let qname = qualified_name(name, vname);
            let symbol_id = symbols.intern(&qname);
            variants.push((vname.clone(), symbol_id));
            self.symbol_to_enum.insert(symbol_id, (type_id, i as u16));
        }

        let ety = PureEnumType {
            id: type_id,
            name: name.to_string(),
            variants,
        };
        self.types.push(ety);
        self.name_to_id.insert(name.to_string(), type_id);
        type_id
    }

    pub fn get_type(&self, id: u32) -> Option<&PureEnumType> {
        self.types.get(id as usize)
    }

    pub fn find_by_name(&self, name: &str) -> Option<&PureEnumType> {
        self.name_to_id.get(name).and_then(|&id| self.get_type(id))
    }

    pub fn lookup_symbol(&self, symbol_id: u32) -> Option<(u32, u16)> {
        self.symbol_to_enum.get(&symbol_id).copied()
    }

    /// Format a symbol as "EnumName.variant" for display.
    pub fn format_symbol(&self, symbol_id: u32, symbols: &SymbolTable) -> Option<String> {
        symbols.resolve(symbol_id).map(|s| s.to_string())
    }

    /// GetAttr dispatch: resolve `attr_name` on an enum type, return symbol Value.
    #[inline]
    pub fn get_variant_value(&self, type_id: u32, attr_name: &str) -> Option<Value> {
        let ety = self.get_type(type_id)?;
        let sym_id = ety.variant_symbol(attr_name)?;
        Some(Value::from_symbol(sym_id))
    }
}

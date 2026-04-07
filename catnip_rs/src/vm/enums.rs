// FILE: catnip_rs/src/vm/enums.rs
//! Enum type system: EnumRegistry + Python-visible CatnipEnumType.
//!
//! Enum variants are represented as Symbol values (tag 3, u32 payload).
//! The SymbolTable (from catnip_core) interns variant names. The EnumRegistry
//! stores type definitions and maps symbol_id -> (enum_type_id, variant_id).

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyString;

pub use catnip_core::symbols::SymbolTable;
use catnip_core::symbols::{MAX_VARIANTS_PER_ENUM, qualified_name};

// ---------------------------------------------------------------------------
// EnumRegistry
// ---------------------------------------------------------------------------

/// A single enum type definition.
#[derive(Debug, Clone)]
pub struct EnumType {
    pub id: u32,
    pub name: String,
    /// (variant_name, symbol_id) pairs, in declaration order.
    pub variants: Vec<(String, u32)>,
}

impl EnumType {
    /// Find symbol_id for a variant name.
    #[inline]
    pub fn variant_symbol(&self, variant_name: &str) -> Option<u32> {
        self.variants
            .iter()
            .find(|(n, _)| n == variant_name)
            .map(|(_, sid)| *sid)
    }
}

/// Registry of all enum types, with reverse lookup from symbol to enum info.
#[derive(Debug, Default)]
pub struct EnumRegistry {
    types: Vec<EnumType>,
    /// symbol_id -> (enum_type_id, variant_index)
    symbol_to_enum: HashMap<u32, (u32, u16)>,
    /// enum name -> type_id
    name_to_id: HashMap<String, u32>,
}

impl EnumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new enum type with its variant names.
    /// Interns all variant names as qualified symbols ("EnumName.variant").
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

        let ety = EnumType {
            id: type_id,
            name: name.to_string(),
            variants,
        };
        self.types.push(ety);
        self.name_to_id.insert(name.to_string(), type_id);
        type_id
    }

    pub fn get_type(&self, id: u32) -> Option<&EnumType> {
        self.types.get(id as usize)
    }

    pub fn find_by_name(&self, name: &str) -> Option<&EnumType> {
        self.name_to_id.get(name).and_then(|&id| self.get_type(id))
    }

    /// Reverse lookup: given a symbol_id, find its enum type and variant index.
    pub fn lookup_symbol(&self, symbol_id: u32) -> Option<(u32, u16)> {
        self.symbol_to_enum.get(&symbol_id).copied()
    }

    /// Format a symbol as "EnumName.variant" if it belongs to an enum.
    pub fn format_symbol(&self, symbol_id: u32, symbols: &SymbolTable) -> Option<String> {
        symbols.resolve(symbol_id).map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// CatnipEnumType -- Python-visible marker for enum types
// ---------------------------------------------------------------------------

/// Python object representing an enum type. Used as a global binding
/// so that `EnumName.variant` resolves via __getattr__.
#[pyclass(name = "CatnipEnumType")]
#[derive(Debug)]
pub struct CatnipEnumType {
    pub name: String,
    pub type_id: u32,
    pub variant_names: Vec<String>,
}

impl CatnipEnumType {
    pub fn new(name: String, type_id: u32, variant_names: &[String]) -> Self {
        Self {
            name,
            type_id,
            variant_names: variant_names.to_vec(),
        }
    }
}

#[pymethods]
impl CatnipEnumType {
    fn __repr__(&self) -> String {
        format!("<enum '{}'>", self.name)
    }

    fn __str__(&self) -> String {
        self.name.clone()
    }

    fn __getattr__(&self, py: Python<'_>, attr: &Bound<'_, PyString>) -> PyResult<Py<PyAny>> {
        let attr_str = attr.to_str()?;
        if self.variant_names.contains(&attr_str.to_string()) {
            let variant = CatnipEnumVariant {
                enum_name: self.name.clone(),
                variant_name: attr_str.to_string(),
                qualified: qualified_name(&self.name, attr_str),
            };
            Ok(Py::new(py, variant)?.into_any())
        } else {
            Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "enum '{}' has no variant '{}'",
                self.name, attr_str
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// CatnipEnumVariant -- opaque Python object for enum variant values
// ---------------------------------------------------------------------------

/// Opaque Python object representing an enum variant.
/// Equality is by (enum_name, variant_name), not by string content.
#[pyclass(name = "CatnipEnumVariant", frozen, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct CatnipEnumVariant {
    #[pyo3(get)]
    pub enum_name: String,
    #[pyo3(get)]
    pub variant_name: String,
    qualified: String,
}

impl CatnipEnumVariant {
    pub fn new_from_parts(enum_name: String, variant_name: String, qualified: String) -> Self {
        Self {
            enum_name,
            variant_name,
            qualified,
        }
    }
}

#[pymethods]
impl CatnipEnumVariant {
    fn __repr__(&self) -> String {
        self.qualified.clone()
    }

    fn __str__(&self) -> String {
        self.qualified.clone()
    }

    fn __bool__(&self) -> bool {
        true
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_variant) = other.cast::<CatnipEnumVariant>() {
            let o = other_variant.borrow();
            Ok(self.enum_name == o.enum_name && self.variant_name == o.variant_name)
        } else {
            Ok(false)
        }
    }

    fn __ne__(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(!self.__eq__(other)?)
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.enum_name.hash(&mut hasher);
        self.variant_name.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enum_registry() {
        let mut symbols = SymbolTable::new();
        let mut reg = EnumRegistry::new();

        let tid = reg.register("Color", &["red".into(), "green".into(), "blue".into()], &mut symbols);
        assert_eq!(tid, 0);

        let ety = reg.get_type(tid).unwrap();
        assert_eq!(ety.name, "Color");
        assert_eq!(ety.variants.len(), 3);

        let red_sym = ety.variant_symbol("red").unwrap();
        assert_eq!(symbols.resolve(red_sym), Some("Color.red"));

        let (type_id, variant_id) = reg.lookup_symbol(red_sym).unwrap();
        assert_eq!(type_id, 0);
        assert_eq!(variant_id, 0);

        let blue_sym = ety.variant_symbol("blue").unwrap();
        let (_, vid) = reg.lookup_symbol(blue_sym).unwrap();
        assert_eq!(vid, 2);
    }

    #[test]
    fn test_find_by_name() {
        let mut symbols = SymbolTable::new();
        let mut reg = EnumRegistry::new();
        reg.register("Direction", &["up".into(), "down".into()], &mut symbols);
        assert!(reg.find_by_name("Direction").is_some());
        assert!(reg.find_by_name("Missing").is_none());
    }
}

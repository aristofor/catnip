// FILE: catnip_rs/src/vm/unions.rs
//! Tagged union (ADT) runtime support.
//!
//! A `union Name { Variant(field); Nullary; }` declaration produces a
//! `CatnipUnionType` Python object stored under `Name` in the local scope.
//!
//! Each variant is materialized as one of two backing objects:
//! - **With payload**: a `CatnipStructType` whose qualified name is
//!   `"Name.Variant"`. Calling `Name.Variant(args...)` produces a
//!   `CatnipStructProxy` of that type, matched by `Name.Variant{field}`
//!   patterns.
//! - **Nullary**: a `CatnipEnumVariant` with the qualified name as well.
//!   Matched by `Name.Variant` patterns (`pattern_enum` shape).
//!
//! The two variant flavors share a qualified name space and a single
//! `__getattr__` entry point, but reuse the existing struct / enum
//! machinery rather than introducing a third value flavor.

use std::sync::Arc;

use indexmap::IndexMap;
use pyo3::prelude::*;
use pyo3::types::PyString;

use crate::vm::enums::CatnipEnumVariant;
use crate::vm::structs::CatnipStructType;
use catnip_core::symbols::qualified_name;

/// One variant binding stored in a `CatnipUnionType`.
///
/// `WithPayload(struct_type)` covers variants declared with parentheses
/// (`Some(value)`), and `Nullary(variant)` covers bare identifiers
/// (`None`).
#[derive(Debug)]
pub enum UnionVariantBinding {
    WithPayload(Py<CatnipStructType>),
    Nullary(Py<CatnipEnumVariant>),
}

impl UnionVariantBinding {
    fn clone_ref(&self, py: Python<'_>) -> Self {
        match self {
            Self::WithPayload(st) => Self::WithPayload(st.clone_ref(py)),
            Self::Nullary(v) => Self::Nullary(v.clone_ref(py)),
        }
    }
}

/// Python-visible namespace object for a tagged union.
///
/// Holds an ordered map of variant names so the linter and pattern
/// exhaustiveness check can enumerate variants in declaration order.
#[pyclass(name = "CatnipUnionType")]
pub struct CatnipUnionType {
    pub name: String,
    /// Variant names in declaration order, used for exhaustiveness checking
    /// and stable iteration.
    pub variant_names: Vec<String>,
    /// Type parameters declared on the union (e.g. `[T]`). Parsed but not
    /// enforced in the MVP -- kept for diagnostics and future type checker.
    pub type_params: Vec<String>,
    variants: Arc<IndexMap<String, UnionVariantBinding>>,
}

impl CatnipUnionType {
    pub fn new(name: String, type_params: Vec<String>, variants: IndexMap<String, UnionVariantBinding>) -> Self {
        let variant_names = variants.keys().cloned().collect();
        Self {
            name,
            variant_names,
            type_params,
            variants: Arc::new(variants),
        }
    }
}

#[pymethods]
impl CatnipUnionType {
    fn __repr__(&self) -> String {
        if self.type_params.is_empty() {
            format!("<union '{}'>", self.name)
        } else {
            format!("<union '{}[{}]'>", self.name, self.type_params.join(", "))
        }
    }

    fn __str__(&self) -> String {
        self.name.clone()
    }

    fn __getattr__(&self, py: Python<'_>, attr: &Bound<'_, PyString>) -> PyResult<Py<PyAny>> {
        let attr_str = attr.to_str()?;
        match self.variants.get(attr_str) {
            Some(UnionVariantBinding::WithPayload(st)) => Ok(st.clone_ref(py).into_any()),
            Some(UnionVariantBinding::Nullary(v)) => Ok(v.clone_ref(py).into_any()),
            None => Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "union '{}' has no variant '{}'",
                self.name, attr_str
            ))),
        }
    }

    /// Return variant names in declaration order. Used by the linter
    /// exhaustiveness check.
    fn variants(&self) -> Vec<String> {
        self.variant_names.clone()
    }
}

/// Build the qualified name for a union variant.
///
/// Centralized so the runtime and the linter agree on the format.
pub fn variant_qualified_name(union_name: &str, variant_name: &str) -> String {
    qualified_name(union_name, variant_name)
}

/// Helper: clone a variant binding (used when exposing bindings to the
/// scope without taking ownership).
pub fn clone_binding(py: Python<'_>, b: &UnionVariantBinding) -> UnionVariantBinding {
    b.clone_ref(py)
}

/// Materialize a `CatnipUnionType` from raw metadata.
///
/// Shared between the AST executor (`op_union` in the registry) and the
/// VM bytecode handler (`MakeUnion`), so they agree on:
/// - how nullary variants become `CatnipEnumVariant` singletons,
/// - how payload variants become `CatnipStructType` constructors named
///   `"Union.Variant"`,
/// - and the duplicate-variant check.
///
/// `variants` is an ordered list of `(variant_name, field_names)`; a
/// variant has payload iff its `field_names` slice is non-empty.
pub fn build_union_type(
    py: Python<'_>,
    name: &str,
    type_params: Vec<String>,
    variants: &[(String, Vec<String>)],
) -> PyResult<Py<CatnipUnionType>> {
    use crate::vm::structs::CatnipStructType;
    use std::collections::HashSet;

    let mut built: IndexMap<String, UnionVariantBinding> = IndexMap::new();
    for (variant_name, field_names) in variants {
        if built.contains_key(variant_name) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "union '{}': duplicate variant '{}'",
                name, variant_name
            )));
        }

        let binding = if field_names.is_empty() {
            // Nullary -- materialize as a CatnipEnumVariant so it lines up
            // with pattern_enum matching (`Option.None`).
            let qname = variant_qualified_name(name, variant_name);
            let v = CatnipEnumVariant::new_from_parts(name.to_string(), variant_name.clone(), qname);
            UnionVariantBinding::Nullary(Py::new(py, v)?)
        } else {
            // Payload-bearing -- materialize as a CatnipStructType named
            // `"Union.Variant"`. Pattern matching against `Union.Variant{...}`
            // compares this exact qualified name.
            let qualified = qualified_name(name, variant_name);
            let n = field_names.len();
            let st = CatnipStructType {
                name: qualified.clone(),
                field_names: field_names.clone(),
                field_defaults: (0..n).map(|_| None).collect(),
                methods: IndexMap::new(),
                static_methods: IndexMap::new(),
                init_fn: None,
                parent_names: Vec::new(),
                mro: vec![qualified],
                abstract_methods: HashSet::new(),
            };
            UnionVariantBinding::WithPayload(Py::new(py, st)?)
        };
        built.insert(variant_name.clone(), binding);
    }

    Py::new(py, CatnipUnionType::new(name.to_string(), type_params, built))
}

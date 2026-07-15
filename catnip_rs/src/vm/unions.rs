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
use pyo3::PyTraverseError;
use pyo3::gc::PyVisit;
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

    /// `(variant_name, marker pointer)` for each payload-bearing variant. The
    /// pointer keys `struct_type_map` so `Union.Variant(...)` builds a native
    /// struct instance via the fast path instead of round-tripping through the
    /// Python `__call__` (which orphans an ObjectTable handle on the variant
    /// type, pinning the union's methods and thus the Context).
    pub fn payload_variant_ptrs(&self) -> Vec<(String, usize)> {
        self.variants
            .iter()
            .filter_map(|(vname, b)| match b {
                UnionVariantBinding::WithPayload(st) => Some((vname.clone(), st.as_ptr() as usize)),
                UnionVariantBinding::Nullary(_) => None,
            })
            .collect()
    }
}

#[pymethods]
impl CatnipUnionType {
    /// Participate in CPython's cyclic GC. The union is stored in `ctx.globals`
    /// and its `variants` hold the payload-variant `CatnipStructType`s (and
    /// nullary `CatnipEnumVariant`s) that carry the union's methods -- callables
    /// reaching the registry and thus the context back. Without surfacing them,
    /// a union declaring a method leaks its context through this opaque pyclass.
    /// `try_get_mut`-free: only the shared `Arc` contents are read.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for binding in self.variants.values() {
            match binding {
                UnionVariantBinding::WithPayload(st) => visit.call(st)?,
                UnionVariantBinding::Nullary(v) => visit.call(v)?,
            }
        }
        Ok(())
    }

    /// Break the union's reference cycles by dropping its variant bindings.
    /// Only called by the GC on an otherwise-unreachable union; replacing the
    /// `Arc` releases this holder's strong reference to the variant types.
    fn __clear__(&mut self) {
        self.variants = Arc::new(IndexMap::new());
    }

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
///
/// `methods` is declared once on the union and shared by every variant:
/// payload variants carry it on their struct type, nullary variants on the
/// `CatnipEnumVariant` itself. `self` receives whichever variant the method
/// is called on; the body discriminates with `match`.
pub fn build_union_type(
    py: Python<'_>,
    name: &str,
    type_params: Vec<String>,
    variants: &[(String, Vec<String>, Vec<String>)],
    methods: IndexMap<String, Py<PyAny>>,
    // Weakref to the AST context, mirroring `op_struct`: present when the union
    // is AST-defined so a payload variant's `__call__` enforces its concrete
    // field types. `None` for the VM path (payload variants are proxies; the
    // native struct types carry the checks).
    ctx_weakref: Option<Py<PyAny>>,
) -> PyResult<Py<CatnipUnionType>> {
    use crate::vm::structs::CatnipStructType;
    use std::collections::HashSet;

    let nullary_methods = Arc::new(
        methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect::<IndexMap<String, Py<PyAny>>>(),
    );

    let mut built: IndexMap<String, UnionVariantBinding> = IndexMap::new();
    for (variant_name, field_names, field_types) in variants {
        if built.contains_key(variant_name) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "union '{}': duplicate variant '{}'",
                name, variant_name
            )));
        }
        if methods.contains_key(variant_name) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "union '{}': method '{}' collides with a variant of the same name",
                name, variant_name
            )));
        }

        let binding = if field_names.is_empty() {
            // Nullary -- materialize as a CatnipEnumVariant so it lines up
            // with pattern_enum matching (`Option.None`).
            let qname = variant_qualified_name(name, variant_name);
            if !nullary_methods.is_empty() {
                // The VM round-trips nullary variants through TAG_SYMBOL,
                // which cannot carry methods: register them so the symbol
                // -> variant reconstruction restores the bindings.
                crate::vm::value::register_union_nullary_methods(&qname, Arc::clone(&nullary_methods));
            }
            let v = CatnipEnumVariant::new_with_methods(
                name.to_string(),
                variant_name.clone(),
                qname,
                Arc::clone(&nullary_methods),
            );
            UnionVariantBinding::Nullary(Py::new(py, v)?)
        } else {
            // Payload-bearing -- materialize as a CatnipStructType named
            // `"Union.Variant"`. Pattern matching against `Union.Variant{...}`
            // compares this exact qualified name.
            let qualified = qualified_name(name, variant_name);
            let n = field_names.len();
            // Payload-field templates: each field is classified against the
            // union's type parameters (`value: T` -> Param(k)), combined with the
            // use-site type arguments at the `CheckGeneric` boundary.
            let field_templates: Vec<catnip_core::vm::opcode::FieldTemplate> = field_names
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let ftext = field_types.get(i).map(String::as_str).filter(|s| !s.is_empty());
                    catnip_core::vm::opcode::compute_field_template(&type_params, ftext)
                })
                .collect();
            // A concrete field (`A(x: int)`) is enforced at construction (needs the
            // ctx weakref, like a struct); a type-parameter field (`Some(value: T)`)
            // is inert here (deferred to the generic boundary). One entry per field
            // so `__call__` indexes it positionally.
            let field_checks: Vec<catnip_core::vm::opcode::ParamCheck> =
                field_templates.iter().map(|t| t.construction_check()).collect();
            let st = CatnipStructType {
                name: qualified.clone(),
                field_names: field_names.clone(),
                field_defaults: (0..n).map(|_| None).collect(),
                field_checks,
                field_templates,
                ctx_weakref: ctx_weakref.as_ref().map(|w| w.clone_ref(py)),
                methods: methods.iter().map(|(k, v)| (k.clone(), v.clone_ref(py))).collect(),
                static_methods: IndexMap::new(),
                init_fn: None,
                parent_names: Vec::new(),
                mro: vec![qualified],
                implements: Vec::new(),
                abstract_methods: HashSet::new(),
            };
            UnionVariantBinding::WithPayload(Py::new(py, st)?)
        };
        built.insert(variant_name.clone(), binding);
    }

    Py::new(py, CatnipUnionType::new(name.to_string(), type_params, built))
}

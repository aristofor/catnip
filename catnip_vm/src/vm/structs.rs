// FILE: catnip_vm/src/vm/structs.rs
//! Native struct and trait types for PureVM -- pure Rust, no PyO3.
//!
//! Registry-based storage: struct instances are stored in a flat Vec
//! with index-based NaN-box references (TAG_STRUCT = 5). Single-threaded,
//! no atomic refcounting needed.

use crate::error::{VMError, VMResult};
use crate::value::Value;
use indexmap::IndexMap;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

/// Resolved trait data: (fields, methods, statics, abstract_methods).
pub type ResolvedTraitData = (
    Vec<PureTraitField>,
    IndexMap<String, u32>,
    IndexMap<String, u32>,
    HashSet<String>,
);

// ---------------------------------------------------------------------------
// Struct types and instances
// ---------------------------------------------------------------------------

/// A field in a struct type definition.
#[derive(Debug, Clone)]
pub struct PureStructField {
    pub name: String,
    pub has_default: bool,
    /// Index into `PureStructType::defaults` if `has_default` is true.
    pub default_slot: Option<usize>,
    /// Runtime type contract from the field annotation (`x: int`), classified
    /// once at `MakeStruct`. `ParamCheck::None` for unannotated or unenforceable
    /// fields. Travels with the field across trait/inheritance merges (the field
    /// is cloned), so a subtype's inherited field keeps its parent's contract.
    pub check: catnip_core::vm::opcode::ParamCheck,
}

/// A struct type registered in the VM.
#[derive(Debug, Clone)]
pub struct PureStructType {
    pub id: u32,
    pub name: String,
    pub fields: Vec<PureStructField>,
    /// Default values for fields with defaults, in order of default_slot.
    pub defaults: Vec<Value>,
    /// Instance methods: name -> func_table index.
    pub methods: IndexMap<String, u32>,
    /// Static methods: name -> func_table index.
    pub static_methods: IndexMap<String, u32>,
    /// init method func_table index, if any.
    pub init_fn: Option<u32>,
    /// Trait names this struct implements.
    pub implements: Vec<String>,
    /// Method resolution order (C3 linearization) -- names for display.
    pub mro: Vec<String>,
    /// Method resolution order by type id -- stable across redefinitions.
    pub mro_ids: Vec<u32>,
    /// Direct parent struct names (from extends).
    pub parent_names: Vec<String>,
    /// Abstract methods that remain unimplemented.
    pub abstract_methods: HashSet<String>,
}

impl PureStructType {
    /// Find field index by name.
    #[inline]
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// Number of required (non-default) fields.
    pub fn required_field_count(&self) -> usize {
        self.fields.iter().filter(|f| !f.has_default).count()
    }

    /// Collect all accessible names (fields + methods + statics).
    pub fn available_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.fields.iter().map(|f| f.name.as_str()).collect();
        names.extend(self.methods.keys().map(|k| k.as_str()));
        names.extend(self.static_methods.keys().map(|k| k.as_str()));
        names
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only count of live `StructCell`s on this thread (created minus
    /// dropped).
    ///
    /// A struct instance is alive from `StructCell::new` until its backing `Arc`
    /// drops to zero. Function-scoped programs must return this to their
    /// pre-execution baseline; unbounded growth is exactly the leak the Arc model
    /// closes. Thread-local (not a global atomic) so parallel tests never race on
    /// it, and correct because a `catnip_vm` struct never crosses threads
    /// (single-threaded VM; broadcast shares no instances). Gated to test builds
    /// -- like `bigint_strong_count` -- so production carries no per-alloc bump;
    /// the only readers are the leak oracles.
    static LIVE_STRUCT_INSTANCES: Cell<usize> = const { Cell::new(0) };
}

/// Test-only: number of live struct instances on the current thread (Arc-backed
/// `StructCell`s not yet dropped). Read by the leak oracles.
#[cfg(test)]
#[inline]
pub fn live_struct_instances() -> usize {
    LIVE_STRUCT_INSTANCES.with(Cell::get)
}

/// A live struct instance: fields by position behind per-slot `Cell`s.
///
/// Stored as `Arc<StructCell>` and referenced by a NaN-boxed pointer
/// (`TAG_STRUCT`), exactly like the native collections. The Arc strong count is
/// the instance refcount -- `Value::clone_refcount`/`decref` manage it, so an
/// instance is freed as soon as no `Value` references it, and containers cascade
/// through this `Drop`. Fields use `Cell` (not `RefCell`) so shared mutation
/// through the Arc never risks a re-entrant borrow panic (e.g. a self-referential
/// struct `p.next = p`), which the old index model could not hit.
pub struct StructCell {
    pub type_id: u32,
    pub fields: Box<[Cell<Value>]>,
    /// Set once the instance is hashed (used as a dict/set key). Further field
    /// mutation is rejected to keep hash/eq stable, mirroring the PyO3 runtime.
    pub frozen: Cell<bool>,
}

impl StructCell {
    /// Build a fresh instance owning the incoming field refcounts (the caller
    /// transfers ownership, matching the old `create_instance` contract).
    pub fn new(type_id: u32, fields: Vec<Value>) -> Self {
        #[cfg(test)]
        LIVE_STRUCT_INSTANCES.with(|c| c.set(c.get() + 1));
        let fields: Box<[Cell<Value>]> = fields.into_iter().map(Cell::new).collect();
        StructCell {
            type_id,
            fields,
            frozen: Cell::new(false),
        }
    }

    /// Number of fields.
    #[inline]
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Read a field by position (refcount-neutral copy of the NaN-box bits).
    #[inline]
    pub fn field(&self, i: usize) -> Value {
        self.fields[i].get()
    }

    /// Snapshot all fields as owned `Value`s (refcount-neutral copies, like the
    /// old `PureStructInstance::fields.clone()`).
    #[inline]
    pub fn field_values(&self) -> Vec<Value> {
        self.fields.iter().map(Cell::get).collect()
    }
}

impl Drop for StructCell {
    fn drop(&mut self) {
        #[cfg(test)]
        LIVE_STRUCT_INSTANCES.with(|c| {
            let n = c.get();
            debug_assert!(n > 0, "struct live-count underflow");
            c.set(n.saturating_sub(1));
        });
        for c in self.fields.iter() {
            c.get().decref();
        }
    }
}

// ---------------------------------------------------------------------------
// Trait types
// ---------------------------------------------------------------------------

/// A trait field definition.
#[derive(Debug, Clone)]
pub struct PureTraitField {
    pub name: String,
    pub has_default: bool,
    pub default: Value,
}

/// A trait definition registered in the VM.
#[derive(Debug, Clone)]
pub struct PureTraitDef {
    pub name: String,
    pub extends: Vec<String>,
    pub fields: Vec<PureTraitField>,
    /// Instance methods: name -> func_table index.
    pub methods: IndexMap<String, u32>,
    /// Static methods: name -> func_table index.
    pub static_methods: IndexMap<String, u32>,
    /// Abstract method names (no body).
    pub abstract_methods: HashSet<String>,
}

// ---------------------------------------------------------------------------
// PureStructRegistry
// ---------------------------------------------------------------------------

/// Registry for struct *types*. Owned by PureVM.
///
/// Instances no longer live here -- they are `Arc<StructCell>` referenced by
/// NaN-boxed pointer, self-managing their lifetime via `Value` refcounts. The
/// registry keeps only the type table, which is long-lived and index-stable.
pub struct PureStructRegistry {
    types: Vec<PureStructType>,
    type_name_map: HashMap<String, u32>,
    /// Per-variant payload-field templates for generic unions, keyed by the
    /// variant's type_id. Only populated for union payload variant types
    /// (`Option.Some`); a plain struct has no entry. Read at the generic-nominal
    /// boundary (`CheckGeneric`) to substitute the use-site type arguments into
    /// the payload-field contracts. Stored beside the type (not on it) to avoid
    /// threading a field through every `PureStructType` construction site.
    variant_templates: HashMap<u32, Vec<catnip_core::vm::opcode::FieldTemplate>>,
}

impl Drop for PureStructRegistry {
    fn drop(&mut self) {
        // Each type owns one ref per heap field default (evaluated at the
        // struct definition; inherited/trait/transplanted copies take their
        // own ref at the copy). `Value` is `Copy` with manual refcounting, so
        // dropping the Vec raw would leak them -- this is their only release
        // path. Self-contained decref (Arc model): a struct-tagged default
        // cascades through its own StructCell, independent of this registry.
        for ty in &self.types {
            for &d in &ty.defaults {
                d.decref();
            }
        }
    }
}

impl PureStructRegistry {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            type_name_map: HashMap::new(),
            variant_templates: HashMap::new(),
        }
    }

    /// Register a new struct type. Returns the type_id.
    pub fn register_type(&mut self, mut ty: PureStructType) -> u32 {
        let id = self.types.len() as u32;
        ty.id = id;
        self.type_name_map.insert(ty.name.clone(), id);
        self.types.push(ty);
        id
    }

    /// Record the payload-field templates for a union variant type (built at
    /// `MakeUnion`). Keyed by the variant's type_id.
    pub fn set_variant_templates(&mut self, type_id: u32, templates: Vec<catnip_core::vm::opcode::FieldTemplate>) {
        if !templates.is_empty() {
            self.variant_templates.insert(type_id, templates);
        }
    }

    /// The payload-field templates for a union variant type, if any.
    #[inline]
    pub fn variant_templates(&self, type_id: u32) -> Option<&[catnip_core::vm::opcode::FieldTemplate]> {
        self.variant_templates.get(&type_id).map(Vec::as_slice)
    }

    /// Number of registered struct types.
    #[inline]
    pub fn type_count(&self) -> u32 {
        self.types.len() as u32
    }

    /// Get a struct type by id.
    #[inline]
    pub fn get_type(&self, id: u32) -> Option<&PureStructType> {
        self.types.get(id as usize)
    }

    /// Get a mutable struct type by id.
    #[inline]
    pub fn get_type_mut(&mut self, id: u32) -> Option<&mut PureStructType> {
        self.types.get_mut(id as usize)
    }

    /// Find a struct type by name.
    #[inline]
    pub fn find_type_by_name(&self, name: &str) -> Option<&PureStructType> {
        self.type_name_map.get(name).and_then(|&id| self.get_type(id))
    }

    /// Find type id by name.
    #[inline]
    pub fn find_type_id(&self, name: &str) -> Option<u32> {
        self.type_name_map.get(name).copied()
    }

    /// True if the type resolves a custom `op_eq` method. Such a type is
    /// unhashable (no structural hash would stay consistent with the custom
    /// equality), matching the PyO3 runtime's "op_eq without op_hash" rule.
    /// Mirrors the dispatch path of `struct_binary_op` (direct `methods` lookup).
    #[inline]
    pub fn type_defines_op_eq(&self, type_id: u32) -> bool {
        self.get_type(type_id)
            .is_some_and(|ty| ty.methods.contains_key("op_eq"))
    }

    /// Func-table index of the type's custom `op_hash`, if it defines one.
    /// Mirrors `type_defines_op_eq`'s lookup path.
    #[inline]
    pub fn type_op_hash_func(&self, type_id: u32) -> Option<u32> {
        self.get_type(type_id).and_then(|ty| ty.methods.get("op_hash").copied())
    }

    /// Format a struct instance as "TypeName(field1=val1, field2=val2)".
    /// Uses `value_repr` callback for recursive struct display.
    pub fn display_instance(&self, cell: &StructCell, value_repr: impl Fn(&Value) -> String) -> String {
        let Some(ty) = self.get_type(cell.type_id) else {
            return "<unknown struct>".to_string();
        };
        let fields: Vec<String> = ty
            .fields
            .iter()
            .zip(cell.fields.iter())
            .map(|(f, c)| format!("{}={}", f.name, value_repr(&c.get())))
            .collect();
        format!("{}({})", ty.name, fields.join(", "))
    }

    /// Get the display name for a type id.
    pub fn type_name(&self, type_id: u32) -> &str {
        self.get_type(type_id).map(|ty| ty.name.as_str()).unwrap_or("struct")
    }

    /// Clear all types (for pipeline reset). Instances are Arc-managed and
    /// release themselves when their `Value`s die -- nothing to clear here.
    pub fn clear(&mut self) {
        self.types.clear();
        self.type_name_map.clear();
    }
}

impl Default for PureStructRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Register the 12 built-in exception struct types (with hierarchy).
/// Returns (ExceptionKind, type_id) pairs for injecting into globals.
/// ALL is ordered parents-before-children so find_type_by_name works for MRO.
pub fn register_builtin_exceptions(
    registry: &mut PureStructRegistry,
) -> Vec<(catnip_core::exception::ExceptionKind, u32)> {
    use catnip_core::exception::ExceptionKind;

    let mut mapping = Vec::with_capacity(ExceptionKind::ALL.len());
    for kind in ExceptionKind::ALL {
        let mro = kind.mro();
        let mro_ids: Vec<u32> = mro.iter().skip(1).filter_map(|n| registry.find_type_id(n)).collect();
        let ty = PureStructType {
            id: 0,
            name: kind.name().to_string(),
            fields: vec![PureStructField {
                name: "message".to_string(),
                has_default: true,
                default_slot: Some(0),
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            defaults: vec![Value::from_str("")],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro,
            mro_ids,
            parent_names: kind.parent().map(|p| vec![p.name().to_string()]).unwrap_or_default(),
            abstract_methods: HashSet::new(),
        };
        let type_id = registry.register_type(ty);
        mapping.push((kind, type_id));
    }
    mapping
}

// ---------------------------------------------------------------------------
// PureTraitRegistry
// ---------------------------------------------------------------------------

/// Registry for trait definitions. Owned by PureVM.
pub struct PureTraitRegistry {
    traits: IndexMap<String, PureTraitDef>,
}

impl Drop for PureTraitRegistry {
    fn drop(&mut self) {
        // Same contract as PureStructRegistry: the trait owns one ref per heap
        // field default; a struct implementing the trait took its own ref when
        // the field was merged in.
        for def in self.traits.values() {
            for f in &def.fields {
                if f.has_default {
                    f.default.decref();
                }
            }
        }
    }
}

impl PureTraitRegistry {
    pub fn new() -> Self {
        Self {
            traits: IndexMap::new(),
        }
    }

    /// Register a trait definition.
    pub fn register_trait(&mut self, def: PureTraitDef) {
        self.traits.insert(def.name.clone(), def);
    }

    /// Look up a trait by name.
    pub fn get_trait(&self, name: &str) -> Option<&PureTraitDef> {
        self.traits.get(name)
    }

    /// Resolve traits for a struct: linearize trait DAG, merge fields and methods.
    /// Returns (merged_fields, merged_methods, merged_statics, abstract_methods).
    pub fn resolve_for_struct(&self, trait_names: &[String]) -> VMResult<ResolvedTraitData> {
        let order = self.linearize_traits(trait_names)?;

        let mut fields: Vec<PureTraitField> = Vec::new();
        let mut field_names: HashSet<String> = HashSet::new();
        let mut methods: IndexMap<String, u32> = IndexMap::new();
        let mut statics: IndexMap<String, u32> = IndexMap::new();
        let mut abstract_set: HashSet<String> = HashSet::new();

        for name in &order {
            let Some(trait_def) = self.get_trait(name) else {
                return Err(VMError::RuntimeError(format!("trait '{}' not found", name)));
            };
            for f in &trait_def.fields {
                if field_names.insert(f.name.clone()) {
                    fields.push(f.clone());
                }
            }
            // Last-wins for methods
            for (k, &v) in &trait_def.methods {
                methods.insert(k.clone(), v);
            }
            for (k, &v) in &trait_def.static_methods {
                statics.insert(k.clone(), v);
            }
            for a in &trait_def.abstract_methods {
                abstract_set.insert(a.clone());
            }
        }

        Ok((fields, methods, statics, abstract_set))
    }

    /// Post-order linearization of trait DAG.
    fn linearize_traits(&self, names: &[String]) -> VMResult<Vec<String>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        for name in names {
            self.linearize_rec(name, &mut result, &mut visited, &mut in_stack)?;
        }
        Ok(result)
    }

    fn linearize_rec(
        &self,
        name: &str,
        result: &mut Vec<String>,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
    ) -> VMResult<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if !in_stack.insert(name.to_string()) {
            return Err(VMError::RuntimeError(format!("circular trait dependency: '{}'", name)));
        }
        if let Some(t) = self.get_trait(name) {
            for parent in &t.extends {
                self.linearize_rec(parent, result, visited, in_stack)?;
            }
        }
        in_stack.remove(name);
        visited.insert(name.to_string());
        result.push(name.to_string());
        Ok(())
    }

    /// Clear all traits (for pipeline reset).
    pub fn clear(&mut self) {
        self.traits.clear();
    }
}

impl Default for PureTraitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_type() {
        let mut reg = PureStructRegistry::new();
        let ty = PureStructType {
            id: 0,
            name: "Point".to_string(),
            fields: vec![
                PureStructField {
                    name: "x".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            defaults: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro: vec!["Point".to_string()],
            mro_ids: vec![],
            parent_names: vec![],
            abstract_methods: HashSet::new(),
        };
        let id = reg.register_type(ty);
        assert_eq!(id, 0);
        assert_eq!(reg.get_type(id).unwrap().name, "Point");
        assert_eq!(reg.find_type_by_name("Point").unwrap().id, 0);
    }

    #[test]
    fn test_create_and_access_instance() {
        let mut reg = PureStructRegistry::new();
        let ty = PureStructType {
            id: 0,
            name: "Point".to_string(),
            fields: vec![
                PureStructField {
                    name: "x".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            defaults: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro: vec!["Point".to_string()],
            mro_ids: vec![],
            parent_names: vec![],
            abstract_methods: HashSet::new(),
        };
        let type_id = reg.register_type(ty);
        let cell = StructCell::new(type_id, vec![Value::from_int(1), Value::from_int(2)]);

        assert_eq!(cell.type_id, type_id);
        assert_eq!(cell.field(0).as_int(), Some(1));
        assert_eq!(cell.field(1).as_int(), Some(2));
        assert_eq!(cell.field_count(), 2);
    }

    #[test]
    fn test_field_index() {
        let ty = PureStructType {
            id: 0,
            name: "Point".to_string(),
            fields: vec![
                PureStructField {
                    name: "x".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            defaults: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro: vec![],
            mro_ids: vec![],
            parent_names: vec![],
            abstract_methods: HashSet::new(),
        };
        assert_eq!(ty.field_index("x"), Some(0));
        assert_eq!(ty.field_index("y"), Some(1));
        assert_eq!(ty.field_index("z"), None);
    }

    #[test]
    fn test_refcount_lifecycle() {
        let mut reg = PureStructRegistry::new();
        let ty = PureStructType {
            id: 0,
            name: "T".to_string(),
            fields: vec![PureStructField {
                name: "v".to_string(),
                has_default: false,
                default_slot: None,
                check: catnip_core::vm::opcode::ParamCheck::None,
            }],
            defaults: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro: vec![],
            mro_ids: vec![],
            parent_names: vec![],
            abstract_methods: HashSet::new(),
        };
        let type_id = reg.register_type(ty);

        // The Arc strong count is the instance refcount; the live-count tracks
        // it. An alias keeps the instance; the last decref frees it.
        let base = live_struct_instances();
        let v = Value::from_struct_instance(StructCell::new(type_id, vec![Value::from_int(42)]));
        assert_eq!(live_struct_instances(), base + 1);

        v.clone_refcount(); // alias
        v.decref(); // drop the alias -- instance still referenced by `v`
        assert_eq!(live_struct_instances(), base + 1);

        v.decref(); // last reference -- instance freed
        assert_eq!(live_struct_instances(), base);
    }

    #[test]
    fn test_drop_cascades_to_fields() {
        // A struct owning a struct field releases it on drop (the container /
        // field cascade the index model could not reach).
        let base = live_struct_instances();
        let inner = Value::from_struct_instance(StructCell::new(0, vec![]));
        let outer = Value::from_struct_instance(StructCell::new(1, vec![inner])); // owns `inner`
        assert_eq!(live_struct_instances(), base + 2);

        outer.decref(); // StructCell::drop decrefs the inner field -> both freed
        assert_eq!(live_struct_instances(), base);
    }

    #[test]
    fn test_display_instance() {
        let mut reg = PureStructRegistry::new();
        let ty = PureStructType {
            id: 0,
            name: "Point".to_string(),
            fields: vec![
                PureStructField {
                    name: "x".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
                    check: catnip_core::vm::opcode::ParamCheck::None,
                },
            ],
            defaults: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            init_fn: None,
            implements: vec![],
            mro: vec![],
            mro_ids: vec![],
            parent_names: vec![],
            abstract_methods: HashSet::new(),
        };
        let type_id = reg.register_type(ty);
        let cell = StructCell::new(type_id, vec![Value::from_int(3), Value::from_int(4)]);
        assert_eq!(reg.display_instance(&cell, |v| v.repr_string()), "Point(x=3, y=4)");
    }

    #[test]
    fn test_trait_linearization() {
        let mut reg = PureTraitRegistry::new();
        reg.register_trait(PureTraitDef {
            name: "A".to_string(),
            extends: vec![],
            fields: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            abstract_methods: HashSet::new(),
        });
        reg.register_trait(PureTraitDef {
            name: "B".to_string(),
            extends: vec!["A".to_string()],
            fields: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            abstract_methods: HashSet::new(),
        });

        let order = reg.linearize_traits(&["B".to_string()]).unwrap();
        assert_eq!(order, vec!["A", "B"]);
    }

    #[test]
    fn test_trait_circular_detection() {
        let mut reg = PureTraitRegistry::new();
        reg.register_trait(PureTraitDef {
            name: "A".to_string(),
            extends: vec!["B".to_string()],
            fields: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            abstract_methods: HashSet::new(),
        });
        reg.register_trait(PureTraitDef {
            name: "B".to_string(),
            extends: vec!["A".to_string()],
            fields: vec![],
            methods: IndexMap::new(),
            static_methods: IndexMap::new(),
            abstract_methods: HashSet::new(),
        });

        let result = reg.linearize_traits(&["A".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_register_builtin_exceptions() {
        let mut registry = PureStructRegistry::new();
        let mapping = super::register_builtin_exceptions(&mut registry);

        assert_eq!(mapping.len(), 12);
        // type_ids are sequential starting from 0
        for (i, (kind, type_id)) in mapping.iter().enumerate() {
            assert_eq!(*type_id, i as u32);
            let ty = registry.get_type(*type_id).unwrap();
            assert_eq!(ty.name, kind.name());
            // Verify MRO matches ExceptionKind::mro()
            assert_eq!(ty.mro, kind.mro(), "MRO mismatch for {}", kind.name());
        }
    }

    #[test]
    fn test_exception_struct_has_message_field() {
        let mut registry = PureStructRegistry::new();
        let mapping = super::register_builtin_exceptions(&mut registry);

        for (_kind, type_id) in &mapping {
            let ty = registry.get_type(*type_id).unwrap();
            assert_eq!(ty.fields.len(), 1);
            assert_eq!(ty.fields[0].name, "message");
            assert!(ty.fields[0].has_default);
        }
    }

    #[test]
    fn test_exception_type_in_globals() {
        use crate::host::VmHost;

        let mut registry = PureStructRegistry::new();
        let mapping = super::register_builtin_exceptions(&mut registry);

        let host = crate::host::PureHost::with_builtins();
        for (kind, type_id) in &mapping {
            host.store_global(kind.name(), Value::from_struct_type(*type_id));
        }

        // Verify globals contain the exception types
        let globals = host.globals();
        let g = globals.borrow();
        for (kind, type_id) in &mapping {
            let val = g.get(kind.name()).expect("exception type not in globals");
            assert!(val.is_struct_type());
            assert_eq!(val.as_struct_type_id(), Some(*type_id));
        }
    }
}

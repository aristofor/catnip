// FILE: catnip_vm/src/vm/structs.rs
//! Native struct and trait types for PureVM -- pure Rust, no PyO3.
//!
//! Registry-based storage: struct instances are stored in a flat Vec
//! with index-based NaN-box references (TAG_STRUCT = 5). Single-threaded,
//! no atomic refcounting needed.

use crate::error::{VMError, VMResult};
use crate::value::Value;
use indexmap::IndexMap;
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

/// A live struct instance with field values stored by position.
#[derive(Debug, Clone)]
pub struct PureStructInstance {
    pub type_id: u32,
    pub fields: Vec<Value>,
}

/// Slot wrapper for refcount tracking in PureStructRegistry.
#[derive(Debug)]
struct InstanceSlot {
    instance: PureStructInstance,
    refcount: u32,
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

/// Registry for struct types and instances. Owned by PureVM.
pub struct PureStructRegistry {
    types: Vec<PureStructType>,
    instances: Vec<Option<InstanceSlot>>,
    free_list: Vec<u32>,
    type_name_map: HashMap<String, u32>,
}

impl PureStructRegistry {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            instances: Vec::new(),
            free_list: Vec::new(),
            type_name_map: HashMap::new(),
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

    /// Create a new struct instance. Returns instance index.
    pub fn create_instance(&mut self, type_id: u32, fields: Vec<Value>) -> u32 {
        let instance = PureStructInstance { type_id, fields };
        let slot = InstanceSlot { instance, refcount: 1 };
        if let Some(idx) = self.free_list.pop() {
            self.instances[idx as usize] = Some(slot);
            idx
        } else {
            let idx = self.instances.len() as u32;
            self.instances.push(Some(slot));
            idx
        }
    }

    /// Increment refcount for an instance.
    #[inline]
    pub fn incref(&mut self, idx: u32) {
        if let Some(Some(slot)) = self.instances.get_mut(idx as usize) {
            slot.refcount += 1;
        }
    }

    /// Decrement refcount. Returns freed field values for cascade cleanup.
    pub fn decref(&mut self, idx: u32) -> Option<Vec<Value>> {
        let slot = self.instances.get_mut(idx as usize)?.as_mut()?;
        debug_assert!(slot.refcount > 0, "struct decref underflow on instance #{}", idx);
        slot.refcount = slot.refcount.saturating_sub(1);
        if slot.refcount == 0 {
            let freed = self.instances[idx as usize].take().unwrap();
            self.free_list.push(idx);
            Some(freed.instance.fields)
        } else {
            None
        }
    }

    /// Get a reference to an instance.
    #[inline]
    pub fn get_instance(&self, idx: u32) -> Option<&PureStructInstance> {
        self.instances
            .get(idx as usize)
            .and_then(|s| s.as_ref())
            .map(|s| &s.instance)
    }

    /// Get a mutable reference to an instance.
    #[inline]
    pub fn get_instance_mut(&mut self, idx: u32) -> Option<&mut PureStructInstance> {
        self.instances
            .get_mut(idx as usize)
            .and_then(|s| s.as_mut())
            .map(|s| &mut s.instance)
    }

    /// Get the refcount for an instance (for debugging).
    #[inline]
    pub fn refcount(&self, idx: u32) -> u32 {
        self.instances
            .get(idx as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.refcount)
            .unwrap_or(0)
    }

    /// Format a struct instance as "TypeName(field1=val1, field2=val2)".
    /// Uses `value_repr` callback for recursive struct display.
    pub fn display_instance(&self, idx: u32, value_repr: impl Fn(&Value) -> String) -> String {
        let Some(inst) = self.get_instance(idx) else {
            return "<freed struct>".to_string();
        };
        let Some(ty) = self.get_type(inst.type_id) else {
            return "<unknown struct>".to_string();
        };
        let fields: Vec<String> = ty
            .fields
            .iter()
            .zip(inst.fields.iter())
            .map(|(f, v)| format!("{}={}", f.name, value_repr(v)))
            .collect();
        format!("{}({})", ty.name, fields.join(", "))
    }

    /// Get the type name for an instance.
    pub fn instance_type_name(&self, idx: u32) -> &str {
        self.get_instance(idx)
            .and_then(|inst| self.get_type(inst.type_id))
            .map(|ty| ty.name.as_str())
            .unwrap_or("struct")
    }

    /// Clear all types and instances (for pipeline reset).
    pub fn clear(&mut self) {
        self.types.clear();
        self.instances.clear();
        self.free_list.clear();
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
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
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
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
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
        let idx = reg.create_instance(type_id, vec![Value::from_int(1), Value::from_int(2)]);

        let inst = reg.get_instance(idx).unwrap();
        assert_eq!(inst.type_id, type_id);
        assert_eq!(inst.fields[0].as_int(), Some(1));
        assert_eq!(inst.fields[1].as_int(), Some(2));
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
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
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
        let idx = reg.create_instance(type_id, vec![Value::from_int(42)]);

        assert_eq!(reg.refcount(idx), 1);
        reg.incref(idx);
        assert_eq!(reg.refcount(idx), 2);

        // First decref: refcount -> 1, no free
        assert!(reg.decref(idx).is_none());
        assert_eq!(reg.refcount(idx), 1);

        // Second decref: refcount -> 0, freed
        let freed = reg.decref(idx).unwrap();
        assert_eq!(freed.len(), 1);
        assert_eq!(freed[0].as_int(), Some(42));
        assert!(reg.get_instance(idx).is_none());
    }

    #[test]
    fn test_free_list_recycling() {
        let mut reg = PureStructRegistry::new();
        let ty = PureStructType {
            id: 0,
            name: "T".to_string(),
            fields: vec![],
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

        let idx0 = reg.create_instance(type_id, vec![]);
        let idx1 = reg.create_instance(type_id, vec![]);
        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);

        // Free slot 0
        reg.decref(idx0);

        // Next allocation should reuse slot 0
        let idx2 = reg.create_instance(type_id, vec![]);
        assert_eq!(idx2, 0);
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
                },
                PureStructField {
                    name: "y".to_string(),
                    has_default: false,
                    default_slot: None,
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
        let idx = reg.create_instance(type_id, vec![Value::from_int(3), Value::from_int(4)]);
        assert_eq!(reg.display_instance(idx, |v| v.repr_string()), "Point(x=3, y=4)");
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

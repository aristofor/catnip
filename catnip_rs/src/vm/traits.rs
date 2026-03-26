// FILE: catnip_rs/src/vm/traits.rs
//! Trait system: registration, stable topological linearization, conflict detection.

use indexmap::IndexMap;
use pyo3::prelude::*;
use std::collections::{HashMap, HashSet};

use super::structs::MethodKey;
use super::value::Value;

/// A trait field with optional default.
#[derive(Debug, Clone)]
pub struct TraitField {
    pub name: String,
    pub has_default: bool,
    pub default: Value,
}

/// Trait definition with fields, method bodies, and parent traits.
#[derive(Debug)]
pub struct TraitDef {
    pub name: String,
    pub extends: Vec<String>,
    pub fields: Vec<TraitField>,
    /// Actual callables for each method (name -> PyObject)
    pub method_bodies: IndexMap<String, Py<PyAny>>,
    /// Static methods (no self binding).
    pub static_methods: IndexMap<String, Py<PyAny>>,
    /// Abstract methods declared in this trait.
    pub abstract_methods: HashSet<MethodKey>,
}

impl TraitDef {
    pub fn new(
        name: String,
        extends: Vec<String>,
        fields: Vec<TraitField>,
        method_bodies: IndexMap<String, Py<PyAny>>,
        abstract_methods: HashSet<MethodKey>,
        static_methods: IndexMap<String, Py<PyAny>>,
    ) -> Self {
        Self {
            name,
            extends,
            fields,
            method_bodies,
            static_methods,
            abstract_methods,
        }
    }
}

/// Resolved trait composition for a struct.
#[derive(Debug)]
pub struct ResolvedTraits {
    /// Linearization order (trait names, deduplicated)
    pub linearization: Vec<String>,
    /// Merged fields from traits (in MRO order)
    pub fields: Vec<TraitField>,
    /// Merged methods from traits (name -> callable)
    pub methods: IndexMap<String, Py<PyAny>>,
    /// Merged static methods from traits (name -> callable)
    pub static_methods: IndexMap<String, Py<PyAny>>,
    /// Abstract methods from traits that remain unimplemented.
    pub abstract_methods: HashSet<MethodKey>,
}

/// Registry for trait definitions.
#[pyclass]
pub struct TraitRegistry {
    traits: IndexMap<String, TraitDef>,
}

#[pymethods]
impl TraitRegistry {
    #[new]
    pub fn new() -> Self {
        Self {
            traits: IndexMap::new(),
        }
    }

    /// Clear all trait definitions.
    pub fn clear(&mut self) {
        self.traits.clear();
    }

    /// Get number of registered traits.
    pub fn len(&self) -> usize {
        self.traits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.traits.is_empty()
    }
}

impl TraitRegistry {
    /// Register a new trait definition.
    pub fn register_trait(&mut self, def: TraitDef) {
        self.traits.insert(def.name.clone(), def);
    }

    /// Find a trait by name.
    pub fn find_trait(&self, name: &str) -> Option<&TraitDef> {
        self.traits.get(name)
    }

    /// Post-order linearization with integrated cycle detection.
    /// Parents before children, last occurrence wins on merge.
    #[cfg(test)]
    fn linearize(&self, name: &str) -> Result<Vec<String>, String> {
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        self.linearize_rec(name, &mut visiting, &mut visited, &mut result)?;
        Ok(result)
    }

    fn linearize_rec(
        &self,
        name: &str,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) -> Result<(), String> {
        if visited.contains(name) {
            return Ok(()); // dedupe
        }
        if visiting.contains(name) {
            return Err(format!("Cycle detected in trait hierarchy involving '{}'", name));
        }

        let extends = if let Some(t) = self.traits.get(name) {
            t.extends.clone()
        } else {
            return Err(format!("Trait '{}' not found", name));
        };

        visiting.insert(name.to_string());
        // Recurse parents first (post-order)
        for parent in &extends {
            self.linearize_rec(parent, visiting, visited, result)?;
        }
        visiting.remove(name);
        visited.insert(name.to_string());
        result.push(name.to_string());
        Ok(())
    }

    /// Check if `ancestor` is an ancestor of `descendant` in the extends graph.
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let mut stack = vec![descendant.to_string()];
        let mut seen = HashSet::new();
        while let Some(current) = stack.pop() {
            if let Some(t) = self.traits.get(&current) {
                for parent in &t.extends {
                    if parent == ancestor {
                        return true;
                    }
                    if seen.insert(parent.clone()) {
                        stack.push(parent.clone());
                    }
                }
            }
        }
        false
    }

    /// Resolve traits for a struct implementing a list of traits.
    ///
    /// Returns merged fields and methods, detecting conflicts.
    /// `struct_method_names`: methods defined on the struct itself (for conflict resolution).
    pub fn resolve_for_struct(
        &self,
        py: Python<'_>,
        implements: &[String],
        struct_method_names: &HashSet<String>,
    ) -> Result<ResolvedTraits, String> {
        // Linearize all implemented traits (post-order, cycle detection integrated)
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut linearization = Vec::new();
        for trait_name in implements {
            if self.traits.get(trait_name).is_none() {
                return Err(format!("Trait '{}' not found", trait_name));
            }
            self.linearize_rec(trait_name, &mut visiting, &mut visited, &mut linearization)?;
        }

        // Merge fields and methods in linearization order (last-wins)
        let mut merged_fields = Vec::new();
        let mut merged_methods: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut merged_static: IndexMap<String, Py<PyAny>> = IndexMap::new();
        let mut merged_abstract: HashSet<MethodKey> = HashSet::new();
        let mut method_source: HashMap<String, String> = HashMap::new();
        let mut static_source: HashMap<String, String> = HashMap::new();
        let mut field_index: HashMap<String, usize> = HashMap::new();

        for trait_name in &linearization {
            let t = self.traits.get(trait_name).unwrap();

            // Merge fields: track position, overwrite default at existing index
            for field in &t.fields {
                if let Some(&idx) = field_index.get(&field.name) {
                    merged_fields[idx] = field.clone();
                } else {
                    field_index.insert(field.name.clone(), merged_fields.len());
                    merged_fields.push(field.clone());
                }
            }

            // Merge methods (last-wins with strict conflict detection)
            for (mname, callable) in &t.method_bodies {
                if let Some(prev_source) = method_source.get(mname) {
                    if prev_source != trait_name && !struct_method_names.contains(mname) {
                        // Child overriding parent is legitimate
                        if !self.is_ancestor(prev_source, trait_name) {
                            return Err(format!(
                                "Method '{}' has conflicting definitions from traits '{}' and '{}'",
                                mname, prev_source, trait_name
                            ));
                        }
                    }
                }

                // Concrete method removes abstract requirement
                let key = MethodKey {
                    name: mname.clone(),
                    kind: super::structs::MethodKind::Instance,
                };
                merged_abstract.remove(&key);

                // Always overwrite (last-wins)
                method_source.insert(mname.clone(), trait_name.clone());
                merged_methods.insert(mname.clone(), callable.clone_ref(py));
            }

            // Merge static methods (last-wins with conflict detection)
            for (mname, callable) in &t.static_methods {
                if let Some(prev_source) = static_source.get(mname) {
                    if prev_source != trait_name
                        && !struct_method_names.contains(mname)
                        && !self.is_ancestor(prev_source, trait_name)
                    {
                        return Err(format!(
                            "Static method '{}' has conflicting definitions from traits '{}' and '{}'",
                            mname, prev_source, trait_name
                        ));
                    }
                }

                // Concrete static method removes abstract requirement
                let key = MethodKey {
                    name: mname.clone(),
                    kind: super::structs::MethodKind::Static,
                };
                merged_abstract.remove(&key);

                static_source.insert(mname.clone(), trait_name.clone());
                merged_static.insert(mname.clone(), callable.clone_ref(py));
            }

            // Propagate abstract methods (not in method_bodies or static_methods)
            for key in &t.abstract_methods {
                match key.kind {
                    super::structs::MethodKind::Instance => {
                        if !merged_methods.contains_key(&key.name) {
                            merged_abstract.insert(key.clone());
                        }
                    }
                    super::structs::MethodKind::Static => {
                        if !merged_static.contains_key(&key.name) {
                            merged_abstract.insert(key.clone());
                        }
                    }
                }
            }
        }

        Ok(ResolvedTraits {
            linearization,
            fields: merged_fields,
            methods: merged_methods,
            static_methods: merged_static,
            abstract_methods: merged_abstract,
        })
    }
}

impl Default for TraitRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trait(name: &str, extends: Vec<&str>, field_names: Vec<&str>) -> TraitDef {
        TraitDef {
            name: name.to_string(),
            extends: extends.into_iter().map(|s| s.to_string()).collect(),
            fields: field_names
                .into_iter()
                .map(|n| TraitField {
                    name: n.to_string(),
                    has_default: false,
                    default: Value::NIL,
                })
                .collect(),
            method_bodies: IndexMap::new(),
            static_methods: IndexMap::new(),
            abstract_methods: HashSet::new(),
        }
    }

    #[test]
    fn test_simple_linearize() {
        let mut reg = TraitRegistry::new();
        reg.register_trait(make_trait("A", vec![], vec!["x"]));
        reg.register_trait(make_trait("B", vec!["A"], vec!["y"]));

        let lin = reg.linearize("B").unwrap();
        assert_eq!(lin, vec!["A", "B"]);
    }

    #[test]
    fn test_diamond_dedupe() {
        let mut reg = TraitRegistry::new();
        reg.register_trait(make_trait("Base", vec![], vec!["x"]));
        reg.register_trait(make_trait("L", vec!["Base"], vec![]));
        reg.register_trait(make_trait("R", vec!["Base"], vec![]));

        // Linearize with L first, then R
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        reg.linearize_rec("L", &mut visiting, &mut visited, &mut result)
            .unwrap();
        reg.linearize_rec("R", &mut visiting, &mut visited, &mut result)
            .unwrap();
        // Post-order: Base first, then L, then R (Base deduped)
        assert_eq!(result, vec!["Base", "L", "R"]);
    }

    #[test]
    fn test_cycle_detection() {
        let mut reg = TraitRegistry::new();
        reg.register_trait(make_trait("A", vec!["B"], vec![]));
        reg.register_trait(make_trait("B", vec!["A"], vec![]));

        let err = reg.linearize("A").unwrap_err();
        assert!(err.contains("Cycle"));
    }

    #[test]
    fn test_is_ancestor() {
        let mut reg = TraitRegistry::new();
        reg.register_trait(make_trait("A", vec![], vec![]));
        reg.register_trait(make_trait("B", vec!["A"], vec![]));
        reg.register_trait(make_trait("C", vec!["B"], vec![]));
        reg.register_trait(make_trait("D", vec![], vec![]));

        assert!(reg.is_ancestor("A", "B"));
        assert!(reg.is_ancestor("A", "C"));
        assert!(reg.is_ancestor("B", "C"));
        assert!(!reg.is_ancestor("C", "A"));
        assert!(!reg.is_ancestor("D", "C"));
        assert!(!reg.is_ancestor("A", "D"));
    }
}

// FILE: catnip_rs/src/vm/structs.rs
//! Native struct types for the Catnip VM.
//!
//! Provides O(1) field access by position instead of Python getattr.

use super::value::Value;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use std::collections::HashMap;

pub type StructTypeId = u32;

/// A field in a struct type definition.
#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub has_default: bool,
    pub default: Value,
}

/// A struct type registered in the VM.
pub struct StructType {
    pub id: StructTypeId,
    pub name: String,
    pub fields: Vec<StructField>,
    /// Raw VMFunction callables keyed by method name.
    pub methods: HashMap<String, Py<PyAny>>,
    /// List of trait names this struct implements.
    pub implements: Vec<String>,
    /// Method resolution order (MRO) for trait composition.
    pub mro: Vec<String>,
    /// Parent methods from extends (for super access).
    pub parent_methods: HashMap<String, Py<PyAny>>,
    /// Name of the parent type (from extends), for super chain resolution.
    pub parent_type_name: Option<String>,
}

impl StructType {
    /// Find field index by name.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }
}

// Manual Debug since Py<PyAny> doesn't implement Debug
impl std::fmt::Debug for StructType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StructType")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("fields", &self.fields)
            .field("methods", &format!("<{} methods>", self.methods.len()))
            .field("implements", &self.implements)
            .field("mro", &self.mro)
            .finish()
    }
}

/// A live struct instance with field values stored by position.
#[derive(Debug, Clone)]
pub struct StructInstance {
    pub type_id: StructTypeId,
    pub fields: Vec<Value>,
}

/// Python-visible proxy for struct instances returned by to_pyobject.
#[pyclass(module = "catnip._rs", name = "CatnipStruct")]
pub struct CatnipStructProxy {
    pub type_name: String,
    pub field_names: Vec<String>,
    pub field_values: Vec<Py<PyAny>>,
    /// Raw VMFunction callables (not yet bound).
    pub methods: HashMap<String, Py<PyAny>>,
    /// Back-reference to the CatnipStructType (AST mode). None for VM-created proxies.
    pub struct_type: Option<Py<CatnipStructType>>,
}

#[pymethods]
impl CatnipStructProxy {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        // Check fields first
        for (i, fname) in self.field_names.iter().enumerate() {
            if fname == name {
                return Ok(self.field_values[i].clone_ref(py));
            }
        }
        // Then methods - create BoundCatnipMethod on the fly
        if let Some(func) = self.methods.get(name) {
            let instance: Py<PyAny> = Py::new(
                py,
                CatnipStructProxy {
                    type_name: self.type_name.clone(),
                    field_names: self.field_names.clone(),
                    field_values: self.field_values.iter().map(|v| v.clone_ref(py)).collect(),
                    methods: self
                        .methods
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                        .collect(),
                    struct_type: self.struct_type.as_ref().map(|st| st.clone_ref(py)),
                },
            )?
            .into_any();
            let bound = Py::new(
                py,
                crate::core::BoundCatnipMethod {
                    func: func.clone_ref(py),
                    instance,
                    super_source_type: None,
                    native_instance_idx: None,
                },
            )?;
            return Ok(bound.into_any());
        }
        Err(pyo3::exceptions::PyAttributeError::new_err(format!(
            "'{}' has no attribute '{name}'",
            self.type_name
        )))
    }

    fn __setattr__(&mut self, name: &str, value: Py<PyAny>) -> PyResult<()> {
        for (i, fname) in self.field_names.iter().enumerate() {
            if fname == name {
                self.field_values[i] = value;
                return Ok(());
            }
        }
        Err(pyo3::exceptions::PyAttributeError::new_err(format!(
            "'{}' has no field '{name}'",
            self.type_name
        )))
    }

    fn __str__(&self, py: Python<'_>) -> String {
        self.__repr__(py)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let fields: Vec<String> = self
            .field_names
            .iter()
            .zip(&self.field_values)
            .map(|(n, v)| {
                let repr = v
                    .bind(py)
                    .repr()
                    .map(|r| r.to_string())
                    .unwrap_or_else(|_| "?".into());
                format!("{n}={repr}")
            })
            .collect();
        format!("{}({})", self.type_name, fields.join(", "))
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        if let Ok(other_struct) = other.cast::<CatnipStructProxy>() {
            let other_ref = other_struct.borrow();
            if self.type_name != other_ref.type_name {
                return Ok(false);
            }
            if self.field_values.len() != other_ref.field_values.len() {
                return Ok(false);
            }
            for (a, b) in self.field_values.iter().zip(&other_ref.field_values) {
                if !a.bind(py).eq(b.bind(py))? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Callable struct type for AST mode. Replaces `dataclasses.make_dataclass`.
///
/// Acts as the constructor: `Point(1, 2)` calls `CatnipStructType.__call__`
/// which creates a `CatnipStructProxy` with the field values.
#[pyclass(module = "catnip._rs", name = "CatnipStructType")]
pub struct CatnipStructType {
    pub name: String,
    pub field_names: Vec<String>,
    /// None = required field, Some(value) = default value
    pub field_defaults: Vec<Option<Py<PyAny>>>,
    /// Method callables keyed by name (raw VMFunction/RustLambda)
    pub methods: HashMap<String, Py<PyAny>>,
    /// Post-constructor init function (if defined)
    pub init_fn: Option<Py<PyAny>>,
    /// Parent methods for super resolution
    pub super_methods: HashMap<String, Py<PyAny>>,
    /// Parent type name for super chain resolution
    pub parent_type_name: Option<String>,
}

#[pymethods]
impl CatnipStructType {
    #[pyo3(signature = (*args, **kwargs))]
    fn __call__(
        slf: &Bound<'_, Self>,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let this = slf.borrow();
        let n_fields = this.field_names.len();
        let n_args = args.len();

        if n_args > n_fields {
            return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "{}() takes at most {} argument(s) ({} given)",
                this.name, n_fields, n_args
            )));
        }

        // Build field values: positional args first, then kwargs, then defaults
        let mut field_values: Vec<Option<Py<PyAny>>> = (0..n_fields).map(|_| None).collect();

        // Apply positional args
        for i in 0..n_args {
            field_values[i] = Some(args.get_item(i)?.unbind());
        }

        // Apply kwargs
        if let Some(kw) = kwargs {
            for (key, value) in kw.iter() {
                let key_str: String = key.extract()?;
                let idx = this
                    .field_names
                    .iter()
                    .position(|n| n == &key_str)
                    .ok_or_else(|| {
                        pyo3::exceptions::PyTypeError::new_err(format!(
                            "{}() got unexpected keyword argument '{}'",
                            this.name, key_str
                        ))
                    })?;
                if field_values[idx].is_some() {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "{}() got multiple values for argument '{}'",
                        this.name, key_str
                    )));
                }
                field_values[idx] = Some(value.unbind());
            }
        }

        // Apply defaults for missing fields
        for i in 0..n_fields {
            if field_values[i].is_none() {
                if let Some(ref default) = this.field_defaults[i] {
                    field_values[i] = Some(default.clone_ref(py));
                }
            }
        }

        // Validate all fields are set
        let mut final_values = Vec::with_capacity(n_fields);
        for (i, v) in field_values.into_iter().enumerate() {
            match v {
                Some(val) => final_values.push(val),
                None => {
                    return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                        "{}() missing required argument: '{}'",
                        this.name, this.field_names[i]
                    )));
                }
            }
        }

        // Clone methods and type info while borrow is active
        let methods: HashMap<String, Py<PyAny>> = this
            .methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let type_name = this.name.clone();
        let field_names = this.field_names.clone();
        drop(this); // release borrow before unbind

        let st_py: Py<CatnipStructType> = slf.clone().unbind();

        let proxy = Py::new(
            py,
            CatnipStructProxy {
                type_name,
                field_names,
                field_values: final_values,
                methods,
                struct_type: Some(st_py),
            },
        )?;

        Ok(proxy.into_any())
    }

    fn __repr__(&self) -> String {
        format!("<struct '{}'>", self.name)
    }

    /// Expose name as a Python attribute for introspection
    #[getter]
    fn __name__(&self) -> &str {
        &self.name
    }
}

/// Proxy for `super` keyword: provides access to parent methods bound to the current instance.
#[pyclass(module = "catnip._rs", name = "SuperProxy")]
pub struct SuperProxy {
    /// Parent methods (unbound VMFunction callables).
    pub methods: HashMap<String, Py<PyAny>>,
    /// The current instance (to bind as self).
    pub instance: Py<PyAny>,
    /// The type name whose methods are in this proxy (for chain resolution).
    pub source_type_name: String,
    /// Native struct instance index (if available, for VM round-trip).
    pub native_instance_idx: Option<u32>,
}

#[pymethods]
impl SuperProxy {
    fn __getattr__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        match self.methods.get(name) {
            Some(func) => {
                let bound = Py::new(
                    py,
                    crate::core::BoundCatnipMethod {
                        func: func.clone_ref(py),
                        instance: self.instance.clone_ref(py),
                        super_source_type: Some(self.source_type_name.clone()),
                        native_instance_idx: self.native_instance_idx,
                    },
                )?;
                Ok(bound.into_any())
            }
            None => Err(pyo3::exceptions::PyAttributeError::new_err(format!(
                "super has no method '{name}'"
            ))),
        }
    }

    fn __repr__(&self) -> String {
        format!("<SuperProxy ({} methods)>", self.methods.len())
    }
}

/// Append-only registry for struct types and instances.
pub struct StructRegistry {
    types: Vec<StructType>,
    instances: Vec<StructInstance>,
}

impl StructRegistry {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            instances: Vec::new(),
        }
    }

    /// Register a new struct type. Returns its unique id.
    pub fn register_type(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: HashMap<String, Py<PyAny>>,
        implements: Vec<String>,
        mro: Vec<String>,
    ) -> StructTypeId {
        self.register_type_with_parents(
            name,
            fields,
            methods,
            implements,
            mro,
            HashMap::new(),
            None,
        )
    }

    /// Register a new struct type with parent methods (for super). Returns its unique id.
    pub fn register_type_with_parents(
        &mut self,
        name: String,
        fields: Vec<StructField>,
        methods: HashMap<String, Py<PyAny>>,
        implements: Vec<String>,
        mro: Vec<String>,
        parent_methods: HashMap<String, Py<PyAny>>,
        parent_type_name: Option<String>,
    ) -> StructTypeId {
        let id = self.types.len() as StructTypeId;
        self.types.push(StructType {
            id,
            name,
            fields,
            methods,
            implements,
            mro,
            parent_methods,
            parent_type_name,
        });
        id
    }

    /// Look up a type by id.
    pub fn get_type(&self, id: StructTypeId) -> Option<&StructType> {
        self.types.get(id as usize)
    }

    /// Look up a type by name. Returns the most recent definition (last registered).
    pub fn find_type_by_name(&self, name: &str) -> Option<&StructType> {
        // Search backwards to find the most recent definition
        self.types.iter().rev().find(|t| t.name == name)
    }

    /// Create an instance. Returns its index in the instance pool.
    pub fn create_instance(&mut self, type_id: StructTypeId, field_values: Vec<Value>) -> u32 {
        let idx = self.instances.len() as u32;
        self.instances.push(StructInstance {
            type_id,
            fields: field_values,
        });
        idx
    }

    pub fn get_instance(&self, idx: u32) -> Option<&StructInstance> {
        self.instances.get(idx as usize)
    }

    pub fn get_instance_mut(&mut self, idx: u32) -> Option<&mut StructInstance> {
        self.instances.get_mut(idx as usize)
    }

    /// Convert a native struct instance to a CatnipStructProxy Python object.
    pub fn instance_to_pyobject(&self, py: Python<'_>, idx: u32) -> PyResult<Py<PyAny>> {
        let inst = self.get_instance(idx).ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("struct instance #{idx} not found"))
        })?;
        let ty = self.get_type(inst.type_id).ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "struct type #{} not found",
                inst.type_id
            ))
        })?;

        let field_names: Vec<String> = ty.fields.iter().map(|f| f.name.clone()).collect();
        let field_values: Vec<Py<PyAny>> = inst.fields.iter().map(|v| v.to_pyobject(py)).collect();
        let methods: HashMap<String, Py<PyAny>> = ty
            .methods
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();

        let proxy = Py::new(
            py,
            CatnipStructProxy {
                type_name: ty.name.clone(),
                field_names,
                field_values,
                methods,
                struct_type: None,
            },
        )?;
        Ok(proxy.into_any())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_type() {
        let mut reg = StructRegistry::new();
        let id = reg.register_type(
            "Point".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );
        assert_eq!(id, 0);
        let ty = reg.get_type(id).unwrap();
        assert_eq!(ty.name, "Point");
        assert_eq!(ty.fields.len(), 2);
        assert_eq!(ty.fields[0].name, "x");
        assert_eq!(ty.fields[1].name, "y");
    }

    #[test]
    fn test_create_instance() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Vec2".into(),
            vec![
                StructField {
                    name: "x".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "y".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],               // implements
            vec!["Point".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(10), Value::from_int(20)]);
        let inst = reg.get_instance(idx).unwrap();
        assert_eq!(inst.type_id, tid);
        assert_eq!(inst.fields[0].as_int(), Some(10));
        assert_eq!(inst.fields[1].as_int(), Some(20));
    }

    #[test]
    fn test_field_access() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Pair".into(),
            vec![
                StructField {
                    name: "a".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],              // implements
            vec!["Pair".into()], // mro
        );
        let idx = reg.create_instance(tid, vec![Value::from_int(1), Value::from_int(2)]);

        // read
        assert_eq!(reg.get_instance(idx).unwrap().fields[0].as_int(), Some(1));

        // write
        reg.get_instance_mut(idx).unwrap().fields[1] = Value::from_int(99);
        assert_eq!(reg.get_instance(idx).unwrap().fields[1].as_int(), Some(99));
    }

    #[test]
    fn test_multiple_types() {
        let mut reg = StructRegistry::new();
        let id0 = reg.register_type(
            "A".into(),
            vec![StructField {
                name: "x".into(),
                has_default: false,
                default: Value::NIL,
            }],
            HashMap::new(),
            vec![],           // implements
            vec!["A".into()], // mro
        );
        let id1 = reg.register_type(
            "B".into(),
            vec![StructField {
                name: "y".into(),
                has_default: false,
                default: Value::NIL,
            }],
            HashMap::new(),
            vec![],           // implements
            vec!["B".into()], // mro
        );
        assert_ne!(id0, id1);
        assert_eq!(reg.get_type(id0).unwrap().name, "A");
        assert_eq!(reg.get_type(id1).unwrap().name, "B");
    }

    #[test]
    fn test_default_fields() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "Config".into(),
            vec![
                StructField {
                    name: "name".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "debug".into(),
                    has_default: true,
                    default: Value::FALSE,
                },
                StructField {
                    name: "level".into(),
                    has_default: true,
                    default: Value::from_int(1),
                },
            ],
            HashMap::new(),
            vec![],                // implements
            vec!["Config".into()], // mro
        );
        let ty = reg.get_type(tid).unwrap();
        assert!(!ty.fields[0].has_default);
        assert!(ty.fields[1].has_default);
        assert_eq!(ty.fields[1].default.as_bool(), Some(false));
        assert!(ty.fields[2].has_default);
        assert_eq!(ty.fields[2].default.as_int(), Some(1));
    }

    #[test]
    fn test_field_index() {
        let mut reg = StructRegistry::new();
        let tid = reg.register_type(
            "RGB".into(),
            vec![
                StructField {
                    name: "r".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "g".into(),
                    has_default: false,
                    default: Value::NIL,
                },
                StructField {
                    name: "b".into(),
                    has_default: false,
                    default: Value::NIL,
                },
            ],
            HashMap::new(),
            vec![],             // implements
            vec!["RGB".into()], // mro
        );
        let ty = reg.get_type(tid).unwrap();
        assert_eq!(ty.field_index("r"), Some(0));
        assert_eq!(ty.field_index("g"), Some(1));
        assert_eq!(ty.field_index("b"), Some(2));
        assert_eq!(ty.field_index("a"), None);
    }
}

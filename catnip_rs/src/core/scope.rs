// FILE: catnip_rs/src/core/scope.rs
//! Shared scope implementation for both AST and VM modes.
//!
//! Scope provides O(1) variable lookup with push/pop semantics.

use indexmap::IndexMap;
use pyo3::PyTraverseError;
use pyo3::exceptions::PyNameError;
use pyo3::gc::PyVisit;
use pyo3::prelude::*;
use pyo3::types::PyList;
use std::collections::HashSet;

/// Flat scope with O(1) lookup and push/pop frame support.
///
/// Instead of chaining scope objects (O(n) lookup), we maintain a single
/// HashMap and track which names were introduced at each "frame" level
/// for proper cleanup on pop.
#[pyclass(name = "Scope", module = "catnip._rs")]
pub struct Scope {
    /// All symbols in current scope chain (flat)
    symbols: IndexMap<String, Py<PyAny>>,
    /// Stack of names introduced at each frame level
    frame_names: Vec<HashSet<String>>,
    /// Previous values for shadowed variables (for restore on pop)
    shadow_stack: Vec<Vec<(String, Option<Py<PyAny>>)>>,
    /// Names modified in each frame
    modified_names: Vec<HashSet<String>>,
    /// Parameter names bound in each frame
    param_names: Vec<HashSet<String>>,
    /// Whether each frame is isolated (function) vs transparent (loop/block)
    frame_isolated: Vec<bool>,
    /// Monotonic id per frame (never reused), to tell apart functions
    /// defined in the current frame from ones captured elsewhere
    frame_tokens: Vec<u64>,
    /// Per-frame set of module-global names READ from inside the frame's
    /// function; drives the global write-through rule in `_set` (Mutex for
    /// the Sync bound, uncontended -- marked from `&self` resolution paths).
    global_reads: std::sync::Mutex<Vec<std::collections::HashSet<String>>>,

    next_frame_token: u64,
}

#[pymethods]
impl Scope {
    /// Participate in CPython's cyclic GC. `ctx.locals` is a `Scope`, and its
    /// `symbols` hold whatever the program binds -- including functions/lambdas
    /// whose `registry` references the context back, closing a
    /// `ctx -> locals(Scope) -> value -> registry -> ctx` cycle the collector
    /// cannot see (a Rust pyclass is opaque to it). Without this, any session
    /// that leaves a context-referencing binding in scope leaks its context.
    /// Both the live `symbols` and the `shadow_stack` (saved values for
    /// restore-on-pop) own strong references, so both are visited.
    ///
    /// PyO3 guards `__traverse__` with a non-blocking borrow, so a re-entrant GC
    /// fired mid-mutation simply skips this scope rather than aliasing `&mut`.
    fn __traverse__(&self, visit: PyVisit<'_>) -> Result<(), PyTraverseError> {
        for v in self.symbols.values() {
            visit.call(v)?;
        }
        for frame in &self.shadow_stack {
            for (_, slot) in frame {
                if let Some(ref v) = slot {
                    visit.call(v)?;
                }
            }
        }
        Ok(())
    }

    /// Break cycles by dropping the strong references the scope holds. Only
    /// called by the GC on an otherwise-unreachable scope (its owning context is
    /// being collected), so the scope is never used again afterwards.
    fn __clear__(&mut self) {
        self.symbols.clear();
        for frame in &mut self.shadow_stack {
            frame.clear();
        }
    }

    /// Create a new empty scope.
    #[new]
    #[pyo3(signature = (symbols=None))]
    pub fn new(py: Python<'_>, symbols: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<Self> {
        let mut scope = Self {
            symbols: IndexMap::new(),
            frame_names: vec![HashSet::new()],
            shadow_stack: vec![Vec::new()],
            modified_names: vec![HashSet::new()],
            param_names: vec![HashSet::new()],
            frame_isolated: vec![false],
            frame_tokens: vec![1],
            global_reads: std::sync::Mutex::new(vec![std::collections::HashSet::new()]),
            next_frame_token: 1,
        };

        if let Some(dict) = symbols {
            for (key, value) in dict.iter() {
                let name: String = key.extract()?;
                scope.set(py, name, value.unbind());
            }
        }

        Ok(scope)
    }

    /// Push a new frame (entering a function/block).
    pub fn push_frame(&mut self) {
        self.frame_names.push(HashSet::new());
        self.shadow_stack.push(Vec::new());
        self.modified_names.push(HashSet::new());
        self.param_names.push(HashSet::new());
        self.frame_isolated.push(false);
        self.next_frame_token += 1;
        self.frame_tokens.push(self.next_frame_token);
        self.global_reads.lock().unwrap().push(std::collections::HashSet::new());
    }

    /// Monotonic id of the current frame.
    pub fn current_frame_token(&self) -> u64 {
        *self.frame_tokens.last().unwrap_or(&1)
    }

    /// Names introduced in the current frame.
    pub fn current_frame_names(&self) -> Vec<String> {
        self.frame_names
            .last()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Pop a frame (exiting a function/block).
    ///
    /// Removes all names introduced in this frame and restores shadowed values.
    pub fn pop_frame(&mut self) {
        if self.frame_names.len() <= 1 {
            return; // Don't pop the global frame
        }

        // Remove names introduced in this frame
        if let Some(names) = self.frame_names.pop() {
            for name in names {
                self.symbols.swap_remove(&name);
            }
        }

        // Restore shadowed values
        if let Some(shadows) = self.shadow_stack.pop() {
            for (name, old_value) in shadows {
                match old_value {
                    Some(v) => {
                        self.symbols.insert(name, v);
                    }
                    None => {
                        self.symbols.swap_remove(&name);
                    }
                }
            }
        }

        self.modified_names.pop();
        self.param_names.pop();
        self.frame_isolated.pop();
        self.frame_tokens.pop();
        self.global_reads.lock().unwrap().pop();
    }

    /// Frame owning the VISIBLE binding of `name` (the highest frame that
    /// introduced it -- the flat `symbols` map holds that binding's value).
    fn owner_frame(&self, name: &str) -> Option<usize> {
        (0..self.frame_names.len())
            .rev()
            .find(|&i| self.frame_names[i].contains(name))
    }

    /// Record a module-global read from inside a function (feeds the
    /// write-through rule below). The mark lives on the ISOLATED frame (the
    /// function): the body's transparent block frames come and go, the
    /// function frame is the stable scope of the rule.
    fn mark_global_read(&self, name: &str) {
        if self.owner_frame(name) != Some(0) {
            return;
        }
        if let Some(iso) = self.find_isolating_frame(name) {
            self.global_reads.lock().unwrap()[iso].insert(name.to_string());
        }
    }

    /// Get a symbol value. Returns None if not found.
    fn get(&self, py: Python<'_>, name: &str) -> Option<Py<PyAny>> {
        let v = self.symbols.get(name).map(|v| v.clone_ref(py));
        if v.is_some() {
            self.mark_global_read(name);
        }
        v
    }

    /// Resolve a symbol, raising NameError if not found.
    fn resolve(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let v = self.symbols.get(name).map(|v| v.clone_ref(py));
        if v.is_some() {
            self.mark_global_read(name);
        }
        v.ok_or_else(|| PyNameError::new_err(catnip_core::constants::format_name_error(name)))
    }

    /// Set a symbol in the current frame.
    ///
    /// If the symbol exists in an outer frame, it shadows the outer value.
    pub fn set(&mut self, py: Python<'_>, name: String, value: Py<PyAny>) {
        let current_frame = self.frame_names.len() - 1;

        // Check if this name was NOT introduced in current frame
        if !self.frame_names[current_frame].contains(&name) {
            // Save old value for restoration on pop
            let old_value = self.symbols.get(&name).map(|v| v.clone_ref(py));
            if let Some(shadows) = self.shadow_stack.last_mut() {
                shadows.push((name.clone(), old_value));
            }
            // Mark as introduced in current frame
            self.frame_names[current_frame].insert(name.clone());
        }

        self.symbols.insert(name, value);
    }

    /// Set a symbol, updating in place if it exists anywhere in scope.
    ///
    /// This mimics the Catnip/Python scoping rule where assignment to an
    /// existing variable updates it in place rather than shadowing.
    fn set_existing(&mut self, name: String, value: Py<PyAny>) -> bool {
        if let indexmap::map::Entry::Occupied(mut e) = self.symbols.entry(name) {
            e.insert(value);
            true
        } else {
            false
        }
    }

    /// Check if a symbol exists.
    fn contains(&self, name: &str) -> bool {
        self.symbols.contains_key(name)
    }

    /// Get all symbols as a dict.
    fn items(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        for (k, v) in &self.symbols {
            dict.set_item(k, v)?;
        }
        Ok(dict.into())
    }

    /// Update scope with dict items.
    fn update(&mut self, py: Python<'_>, other: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        for (key, value) in other.iter() {
            let name: String = key.extract()?;
            self.set(py, name, value.unbind());
        }
        Ok(())
    }

    /// Get current frame depth.
    pub fn depth(&self) -> usize {
        self.frame_names.len()
    }

    /// Snapshot current scope state for closure capture.
    ///
    /// Returns a dict of all current symbols. The closure can use this
    /// to restore captured variables when called.
    pub fn snapshot(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        for (k, v) in &self.symbols {
            // Module globals are NOT captured: a closure resolves them live
            // at call time (top-level late binding, wip/CLOSURE_SEMANTICS.md
            // -- the VM's MakeFunction suppresses them the same way). Only
            // function-scope bindings are copied.
            if self.owner_frame(k) == Some(0) {
                continue;
            }
            dict.set_item(k, v)?;
        }
        Ok(dict.into())
    }

    /// Push a new frame with captured variables from a closure.
    ///
    /// The captured dict contains variables that should be available
    /// in this frame (from the closure's creation context).
    /// The dict is stored for later sync_to_captures() call.
    pub fn push_frame_with_captures(
        &mut self,
        py: Python<'_>,
        captured: &Bound<'_, pyo3::types::PyDict>,
    ) -> PyResult<()> {
        self.push_frame();
        // Function frames are isolated: locals shadow parent variables
        self.mark_frame_isolated();
        // Restore captured variables into the new frame
        for (key, value) in captured.iter() {
            let name: String = key.extract()?;
            // Use set() to properly track in frame_names
            self.set(py, name, value.unbind());
        }
        Ok(())
    }

    /// Sync modified variables back to the captured dict before pop.
    ///
    /// Call this before pop_frame() to persist closure state.
    pub fn sync_to_captures(&self, py: Python<'_>, captured: &Bound<'_, pyo3::types::PyDict>) -> PyResult<()> {
        // Sync captured variables back to the closure_scope dict. Variables in
        // closure_scope are intentional captures and must propagate. The
        // shadow/restore mechanism in _set/find_isolating_frame handles
        // isolation for non-captured name collisions.
        //
        // Exception: a captured name that is also a parameter of the current
        // frame is frame-local and must NOT sync back -- doing so would
        // overwrite the closure/parent binding with the call-local parameter
        // value (scope leak when param name == captured name).
        let params = self.param_names.last();
        for key in captured.keys() {
            let name: String = key.extract()?;
            if params.is_some_and(|p| p.contains(&name)) {
                continue;
            }
            if let Some(value) = self.symbols.get(&name) {
                captured.set_item(&name, value.clone_ref(py))?;
            }
        }
        Ok(())
    }

    // --- Compatibility with Cython Scope interface ---

    /// Resolve a symbol (Cython Scope compatibility).
    fn _resolve(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.resolve(py, name)
    }

    /// Set a symbol with Catnip scoping semantics (Cython Scope compatibility).
    ///
    /// If the symbol exists anywhere, update it in place.
    /// Otherwise, create it in the current frame.
    fn _set(&mut self, py: Python<'_>, name: String, value: Py<PyAny>) {
        let current_frame = self.frame_names.len() - 1;
        if current_frame > 0 {
            if let Some(modified) = self.modified_names.last_mut() {
                modified.insert(name.clone());
            }
        }

        if self.symbols.contains_key(&name) {
            // Check if any frame from current down to the variable's owner is isolated
            if let Some(iso_frame) = self.find_isolating_frame(&name) {
                // A module global READ from this function writes through to
                // the live binding (the top-level late-binding contract,
                // wip/CLOSURE_SEMANTICS.md); a write-only global name shadows
                // into a local (scope isolation -- a function's `a = ...`
                // must not clobber the caller's `a`). The VM's own behavior
                // on the write-only corner is order-and-shape dependent
                // (traced in CLOSURE_SEMANTICS.md as an open corner); the
                // read-based rule is the coherent one.
                let reads_it = {
                    let reads = self.global_reads.lock().unwrap();
                    reads[iso_frame].contains(&name)
                };
                if reads_it && self.owner_frame(&name) == Some(0) {
                    self.symbols.insert(name, value);
                } else {
                    // Variable is from before an isolated frame: shadow
                    let old_value = self.symbols.get(&name).map(|v| v.clone_ref(py));
                    self.shadow_stack[iso_frame].push((name.clone(), old_value));
                    self.frame_names[iso_frame].insert(name.clone());
                    self.symbols.insert(name, value);
                }
            } else {
                // No isolation boundary: update in place
                self.symbols.insert(name, value);
            }
        } else {
            // Create in current frame
            self.set(py, name, value);
        }
    }

    /// Set a function parameter (always shadows parent frame variables).
    ///
    /// Unlike _set, this always creates a new binding in the current frame,
    /// ensuring each function call has its own copy of parameters.
    fn _set_param(&mut self, py: Python<'_>, name: String, value: Py<PyAny>) {
        let current_frame = self.frame_names.len() - 1;

        if current_frame > 0 {
            if let Some(params) = self.param_names.last_mut() {
                params.insert(name.clone());
            }
        }

        // Check if this name was introduced in current frame
        if self.frame_names[current_frame].contains(&name) {
            // Already in current frame: update in place
            self.symbols.insert(name, value);
        } else if self.symbols.contains_key(&name) {
            // Exists in parent frame: shadow it (save old value for restoration)
            let old_value = self.symbols.get(&name).map(|v| v.clone_ref(py));
            if let Some(shadows) = self.shadow_stack.last_mut() {
                shadows.push((name.clone(), old_value));
            }
            self.frame_names[current_frame].insert(name.clone());
            self.symbols.insert(name, value);
        } else {
            // New variable: create in current frame
            self.set(py, name, value);
        }
    }

    /// Get symbols dict (Cython Scope compatibility).
    #[getter]
    fn _symbols(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.items(py)
    }

    fn _modified_names(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.modified_names.is_empty() {
            return Ok(PyList::empty(py).into());
        }

        let current_frame = self.modified_names.len() - 1;
        let mut names: Vec<String> = self.modified_names[current_frame].iter().cloned().collect();
        if let Some(params) = self.param_names.get(current_frame) {
            names.retain(|name| !params.contains(name));
        }
        Ok(PyList::new(py, names)?.into())
    }

    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        self.resolve(py, name)
    }

    fn __setitem__(&mut self, py: Python<'_>, name: &str, value: Py<PyAny>) {
        self._set(py, name.to_string(), value);
    }

    /// Pickle support: get state for serialization.
    fn __getstate__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let dict = pyo3::types::PyDict::new(py);

        // Serialize symbols as dict
        let symbols_dict = pyo3::types::PyDict::new(py);
        for (k, v) in &self.symbols {
            symbols_dict.set_item(k, v)?;
        }
        dict.set_item("symbols", symbols_dict)?;

        // Serialize frame_names as list of lists
        let frame_names_list = PyList::new(
            py,
            self.frame_names
                .iter()
                .map(|s| PyList::new(py, s.iter().cloned()).unwrap())
                .collect::<Vec<_>>(),
        )?;
        dict.set_item("frame_names", frame_names_list)?;

        // Serialize shadow_stack as list of lists of tuples
        let shadow_stack_list = PyList::new(
            py,
            self.shadow_stack
                .iter()
                .map(|shadows| {
                    PyList::new(
                        py,
                        shadows
                            .iter()
                            .map(|(name, opt_value)| {
                                let tuple_val = match opt_value {
                                    Some(v) => (name.clone(), v.clone_ref(py)).into_pyobject(py).unwrap(),
                                    None => (name.clone(), py.None()).into_pyobject(py).unwrap(),
                                };
                                tuple_val
                            })
                            .collect::<Vec<_>>(),
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>(),
        )?;
        dict.set_item("shadow_stack", shadow_stack_list)?;

        // Serialize modified_names as list of lists
        let modified_names_list = PyList::new(
            py,
            self.modified_names
                .iter()
                .map(|s| PyList::new(py, s.iter().cloned()).unwrap())
                .collect::<Vec<_>>(),
        )?;
        dict.set_item("modified_names", modified_names_list)?;

        // Serialize param_names as list of lists
        let param_names_list = PyList::new(
            py,
            self.param_names
                .iter()
                .map(|s| PyList::new(py, s.iter().cloned()).unwrap())
                .collect::<Vec<_>>(),
        )?;
        dict.set_item("param_names", param_names_list)?;

        Ok(dict.into())
    }

    /// Pickle support: restore state from serialization.
    fn __setstate__(&mut self, _py: Python<'_>, state: &Bound<'_, pyo3::types::PyAny>) -> PyResult<()> {
        let dict: &Bound<'_, pyo3::types::PyDict> = state.cast()?;

        // Restore symbols
        self.symbols.clear();
        let symbols_item = dict.get_item("symbols")?.unwrap();
        let symbols_dict: &Bound<'_, pyo3::types::PyDict> = symbols_item.cast()?;
        for (key, value) in symbols_dict.iter() {
            let name: String = key.extract()?;
            self.symbols.insert(name, value.unbind());
        }

        // Restore frame_names
        self.frame_names.clear();
        let frame_names_item = dict.get_item("frame_names")?.unwrap();
        let frame_names_list: &Bound<'_, PyList> = frame_names_item.cast()?;
        for frame in frame_names_list.iter() {
            let names_list: &Bound<'_, PyList> = frame.cast()?;
            let mut set = HashSet::new();
            for name in names_list.iter() {
                set.insert(name.extract::<String>()?);
            }
            self.frame_names.push(set);
        }

        // Restore shadow_stack
        self.shadow_stack.clear();
        let shadow_stack_item = dict.get_item("shadow_stack")?.unwrap();
        let shadow_stack_list: &Bound<'_, PyList> = shadow_stack_item.cast()?;
        for shadows in shadow_stack_list.iter() {
            let shadows_list: &Bound<'_, PyList> = shadows.cast()?;
            let mut vec = Vec::new();
            for shadow in shadows_list.iter() {
                let tuple: &Bound<'_, pyo3::types::PyTuple> = shadow.cast()?;
                let name: String = tuple.get_item(0)?.extract()?;
                let value_obj = tuple.get_item(1)?;
                let opt_value = if value_obj.is_none() {
                    None
                } else {
                    Some(value_obj.unbind())
                };
                vec.push((name, opt_value));
            }
            self.shadow_stack.push(vec);
        }

        // Restore modified_names
        self.modified_names.clear();
        let modified_names_item = dict.get_item("modified_names")?.unwrap();
        let modified_names_list: &Bound<'_, PyList> = modified_names_item.cast()?;
        for frame in modified_names_list.iter() {
            let names_list: &Bound<'_, PyList> = frame.cast()?;
            let mut set = HashSet::new();
            for name in names_list.iter() {
                set.insert(name.extract::<String>()?);
            }
            self.modified_names.push(set);
        }

        // Restore param_names
        self.param_names.clear();
        let param_names_item = dict.get_item("param_names")?.unwrap();
        let param_names_list: &Bound<'_, PyList> = param_names_item.cast()?;
        for frame in param_names_list.iter() {
            let names_list: &Bound<'_, PyList> = frame.cast()?;
            let mut set = HashSet::new();
            for name in names_list.iter() {
                set.insert(name.extract::<String>()?);
            }
            self.param_names.push(set);
        }

        Ok(())
    }
}

impl Scope {
    /// Find the nearest isolated frame that separates the current frame
    /// from the frame owning `name`. Returns Some(frame_index) if the variable
    /// should be shadowed (it's from before an isolation boundary), None otherwise.
    fn find_isolating_frame(&self, name: &str) -> Option<usize> {
        let current = self.frame_names.len() - 1;
        // Walk from current frame downward looking for the frame that owns this name
        // and checking if any isolated frame exists between current and owner
        for i in (0..=current).rev() {
            if self.frame_names[i].contains(name) {
                // Variable is owned by frame i, no isolation between i and current
                return None;
            }
            if self.frame_isolated[i] {
                // Found an isolated frame before finding the owner
                return Some(i);
            }
        }
        None
    }

    /// Mark current frame as isolated (function scope).
    /// Isolated frames shadow parent variables on _set/set_catnip.
    pub fn mark_frame_isolated(&mut self) {
        if let Some(last) = self.frame_isolated.last_mut() {
            *last = true;
        }
    }

    /// Public version of _set for Rust callers (Catnip scoping semantics).
    ///
    /// If the symbol exists anywhere, update it in place.
    /// Otherwise, create it in the current frame.
    pub fn set_catnip(&mut self, py: Python<'_>, name: String, value: Py<PyAny>) {
        // One store semantics: delegate to `_set` (this used to be a drifted
        // copy -- the global write-through rule only landed in one of them).
        self._set(py, name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::Scope;
    use pyo3::prelude::*;

    #[test]
    fn scope_push_pop_restores() {
        Python::attach(|py| {
            let mut scope = Scope::new(py, None).unwrap();
            scope.set(py, "a".to_string(), 1i64.into_pyobject(py).unwrap().into_any().unbind());

            scope.push_frame();
            scope.set(py, "a".to_string(), 2i64.into_pyobject(py).unwrap().into_any().unbind());
            scope.set(py, "b".to_string(), 3i64.into_pyobject(py).unwrap().into_any().unbind());

            assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 2);
            assert_eq!(scope.resolve(py, "b").unwrap().extract::<i64>(py).unwrap(), 3);

            scope.pop_frame();
            assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 1);
            assert!(scope.get(py, "b").is_none());
        });
    }

    #[test]
    fn scope_set_existing_updates_in_place() {
        Python::attach(|py| {
            let mut scope = Scope::new(py, None).unwrap();
            scope.set(
                py,
                "x".to_string(),
                10i64.into_pyobject(py).unwrap().into_any().unbind(),
            );

            assert!(scope.set_existing("x".to_string(), 20i64.into_pyobject(py).unwrap().into_any().unbind()));
            assert_eq!(scope.resolve(py, "x").unwrap().extract::<i64>(py).unwrap(), 20);
            assert!(!scope.set_existing(
                "missing".to_string(),
                30i64.into_pyobject(py).unwrap().into_any().unbind()
            ));
        });
    }
}
#[test]
fn scope_parent_lookup() {
    // Variable définie dans scope parent est accessible
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();
        scope.set(
            py,
            "parent_var".to_string(),
            100i64.into_pyobject(py).unwrap().into_any().unbind(),
        );

        scope.push_frame();
        // Variable parent accessible depuis child scope
        assert_eq!(
            scope.resolve(py, "parent_var").unwrap().extract::<i64>(py).unwrap(),
            100
        );
    });
}

#[test]
fn scope_shadowing() {
    // Variable locale masque variable parent
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();
        scope.set(
            py,
            "x".to_string(),
            10i64.into_pyobject(py).unwrap().into_any().unbind(),
        );

        scope.push_frame();
        scope.set(
            py,
            "x".to_string(),
            20i64.into_pyobject(py).unwrap().into_any().unbind(),
        );

        // Child scope voit sa propre valeur
        assert_eq!(scope.resolve(py, "x").unwrap().extract::<i64>(py).unwrap(), 20);

        scope.pop_frame();
        // Parent scope voit sa valeur originale
        assert_eq!(scope.resolve(py, "x").unwrap().extract::<i64>(py).unwrap(), 10);
    });
}

#[test]
fn scope_nested_three_levels() {
    // 3 niveaux de scopes imbriqués
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();

        // Level 0
        scope.set(py, "a".to_string(), 1i64.into_pyobject(py).unwrap().into_any().unbind());

        // Level 1
        scope.push_frame();
        scope.set(py, "b".to_string(), 2i64.into_pyobject(py).unwrap().into_any().unbind());

        // Level 2
        scope.push_frame();
        scope.set(py, "c".to_string(), 3i64.into_pyobject(py).unwrap().into_any().unbind());

        // Toutes les variables accessibles depuis level 2
        assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 1);
        assert_eq!(scope.resolve(py, "b").unwrap().extract::<i64>(py).unwrap(), 2);
        assert_eq!(scope.resolve(py, "c").unwrap().extract::<i64>(py).unwrap(), 3);

        // Pop level 2
        scope.pop_frame();
        assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 1);
        assert_eq!(scope.resolve(py, "b").unwrap().extract::<i64>(py).unwrap(), 2);
        assert!(scope.get(py, "c").is_none());

        // Pop level 1
        scope.pop_frame();
        assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 1);
        assert!(scope.get(py, "b").is_none());
        assert!(scope.get(py, "c").is_none());
    });
}

#[test]
fn scope_multiple_variables() {
    // Plusieurs variables dans scope, accès O(1)
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();

        // Set 10 variables
        for i in 0..10i64 {
            scope.set(
                py,
                format!("var_{}", i),
                (i * 10).into_pyobject(py).unwrap().into_any().unbind(),
            );
        }

        // Verify all accessible
        for i in 0..10 {
            assert_eq!(
                scope
                    .resolve(py, &format!("var_{}", i))
                    .unwrap()
                    .extract::<i64>(py)
                    .unwrap(),
                i * 10
            );
        }
    });
}

#[test]
fn scope_resolve_missing_variable() {
    // Accès à variable inexistante retourne None
    Python::attach(|py| {
        let scope = Scope::new(py, None).unwrap();
        assert!(scope.get(py, "missing").is_none());
        assert!(scope.resolve(py, "missing").is_err());
    });
}

#[test]
fn scope_set_existing_in_parent() {
    // set_existing update la variable dans le parent scope
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();
        scope.set(
            py,
            "x".to_string(),
            10i64.into_pyobject(py).unwrap().into_any().unbind(),
        );

        scope.push_frame();
        // Update parent variable depuis child
        assert!(scope.set_existing("x".to_string(), 99i64.into_pyobject(py).unwrap().into_any().unbind()));

        assert_eq!(scope.resolve(py, "x").unwrap().extract::<i64>(py).unwrap(), 99);

        scope.pop_frame();
        // Parent voit la modification
        assert_eq!(scope.resolve(py, "x").unwrap().extract::<i64>(py).unwrap(), 99);
    });
}

#[test]
fn scope_shadowing_no_leak() {
    // Variable locale ne leake pas dans parent après pop
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();

        scope.push_frame();
        scope.set(
            py,
            "local_only".to_string(),
            42i64.into_pyobject(py).unwrap().into_any().unbind(),
        );
        assert_eq!(scope.resolve(py, "local_only").unwrap().extract::<i64>(py).unwrap(), 42);

        scope.pop_frame();
        // Variable disparaît après pop
        assert!(scope.get(py, "local_only").is_none());
    });
}

#[test]
fn scope_deep_nesting() {
    // 5 niveaux de scopes
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();

        for i in 0..5i64 {
            scope.push_frame();
            scope.set(
                py,
                format!("level_{}", i),
                i.into_pyobject(py).unwrap().into_any().unbind(),
            );
        }

        // Toutes les variables accessibles
        for i in 0..5 {
            assert_eq!(
                scope
                    .resolve(py, &format!("level_{}", i))
                    .unwrap()
                    .extract::<i64>(py)
                    .unwrap(),
                i
            );
        }

        // Pop tous les scopes
        for _ in 0..5 {
            scope.pop_frame();
        }

        // Toutes les variables disparues
        for i in 0..5 {
            assert!(scope.get(py, &format!("level_{}", i)).is_none());
        }
    });
}

#[test]
fn scope_set_catnip_shadows_parent_variable() {
    // _set / set_catnip sur une variable du parent doit shadow, pas écraser
    Python::attach(|py| {
        let mut scope = Scope::new(py, None).unwrap();
        // Frame 0 : a = [1, 2]
        scope.set(
            py,
            "a".to_string(),
            vec![1i64, 2i64].into_pyobject(py).unwrap().into_any().unbind(),
        );

        // Frame 1 (lambda avec param a) - isolated function frame
        scope.push_frame();
        scope.mark_frame_isolated();
        scope._set_param(
            py,
            "a".to_string(),
            vec![10i64, 20i64].into_pyobject(py).unwrap().into_any().unbind(),
        );

        // Frame 2 (fonction interne qui utilise _set pour a = calcul) - isolated
        scope.push_frame();
        scope.mark_frame_isolated();
        scope._set(
            py,
            "a".to_string(),
            999i64.into_pyobject(py).unwrap().into_any().unbind(),
        );

        // Frame 2 voit 999
        assert_eq!(scope.resolve(py, "a").unwrap().extract::<i64>(py).unwrap(), 999);

        // Pop frame 2 : restaure a du frame 1
        scope.pop_frame();
        let a_val: Vec<i64> = scope.resolve(py, "a").unwrap().extract::<Vec<i64>>(py).unwrap();
        assert_eq!(a_val, vec![10, 20]);

        // Pop frame 1 : restaure a du frame 0
        scope.pop_frame();
        let a_val: Vec<i64> = scope.resolve(py, "a").unwrap().extract::<Vec<i64>>(py).unwrap();
        assert_eq!(a_val, vec![1, 2]);
    });
}

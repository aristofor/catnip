// FILE: catnip_rs/src/nd/vm_decl.rs
//! Standalone wrapper for ND declaration form.
//!
//! In standalone mode (no Python context), wraps the ND lambda so that
//! calling `f(seed)` dispatches `lambda(seed, recur)` -- a dedicated
//! recur handle with memoization and depth limiting.
//!
//! Thread-local `ND_ABORT` flag enables fast error unwinding: when the depth
//! guard fires, each intermediate VMFunction.__call__ checks the flag and
//! returns immediately instead of creating a new VM (~200ms/level savings).

use crate::constants::ND_MAX_RECURSION_DEPTH;
use crate::vm::frame::{CodeObject, NativeClosureScope, VMFunction};
use pyo3::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// Thread-local abort flag for fast ND recursion error unwinding.
// Set by NDVmRecur when depth limit is hit; checked by VMFunction.__call__
// to skip expensive VM creation during stack unwinding.
thread_local! {
    static ND_ABORT: Cell<bool> = const { Cell::new(false) };
}

/// Set the ND abort flag (called when depth limit is exceeded).
pub fn set_nd_abort() {
    ND_ABORT.with(|f| f.set(true));
}

/// Check and clear the ND abort flag.
/// Returns true if abort was requested (caller should return error immediately).
pub fn check_nd_abort() -> bool {
    ND_ABORT.with(|f| f.get())
}

/// Clear the ND abort flag (called when ND recursion completes).
pub fn clear_nd_abort() {
    ND_ABORT.with(|f| f.set(false));
}

/// Standalone ND declaration wrapper.
///
/// Created by the VM when encountering `f = ~~lambda` in standalone mode
/// (no context available for NDDeclaration). Calling `f(seed)` invokes
/// `lambda(seed, recur)` via an NDVmRecur handle.
#[pyclass(name = "NDVmDecl", module = "catnip._rs")]
pub struct NDVmDecl {
    nd_lambda: Py<PyAny>,
    memoize: bool,
}

impl NDVmDecl {
    pub fn new(nd_lambda: Py<PyAny>) -> Self {
        Self {
            nd_lambda,
            memoize: true,
        }
    }

    pub fn with_memoize(nd_lambda: Py<PyAny>, memoize: bool) -> Self {
        Self { nd_lambda, memoize }
    }
}

#[pymethods]
impl NDVmDecl {
    fn __call__(self_: &Bound<'_, Self>, py: Python<'_>, seed: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let this = self_.borrow();
        let lambda = this.nd_lambda.clone_ref(py);
        let memoize = this.memoize;
        drop(this);

        // Extract VMFunction internals for VM fast path (frame push instead of new VM)
        let lambda_bound = lambda.bind(py);
        let recur = if let Ok(vm_func) = lambda_bound.cast::<VMFunction>() {
            let r = vm_func.borrow();
            let code = Arc::clone(&r.vm_code.borrow(py).inner);
            let closure = r.native_closure.clone();
            drop(r);
            Py::new(
                py,
                NDVmRecur::with_vm_internals(lambda.clone_ref(py), code, closure, memoize),
            )?
        } else {
            Py::new(py, NDVmRecur::with_memoize(lambda.clone_ref(py), memoize))?
        };
        let result = lambda_bound.call1((seed, &recur));
        // Clear abort flag after ND recursion completes (success or error)
        clear_nd_abort();
        result.map(|r| r.unbind())
    }

    fn __repr__(&self) -> String {
        "NDVmDecl(~~)".to_string()
    }
}

/// Recur handle for standalone ND recursion.
///
/// Passed as second arg to ND lambdas. Calling `recur(value)` re-invokes
/// the lambda with `(value, self)`. Includes memoization cache and depth guard.
#[pyclass(name = "NDVmRecur", module = "catnip._rs", unsendable)]
pub struct NDVmRecur {
    nd_lambda: Py<PyAny>,
    cache: RefCell<HashMap<u64, Py<PyAny>>>,
    depth: Cell<usize>,
    memoize: bool,
    /// Compiled bytecode from the ND lambda (if it's a VMFunction)
    vm_code: Option<Arc<CodeObject>>,
    /// Closure scope from the ND lambda
    vm_closure: Option<NativeClosureScope>,
}

impl NDVmRecur {
    fn with_memoize(nd_lambda: Py<PyAny>, memoize: bool) -> Self {
        Self {
            nd_lambda,
            cache: RefCell::new(HashMap::new()),
            depth: Cell::new(0),
            memoize,
            vm_code: None,
            vm_closure: None,
        }
    }

    /// Create with VMFunction internals for VM fast path.
    pub(crate) fn with_vm_internals(
        nd_lambda: Py<PyAny>,
        code: Arc<CodeObject>,
        closure: Option<NativeClosureScope>,
        memoize: bool,
    ) -> Self {
        Self {
            nd_lambda,
            cache: RefCell::new(HashMap::new()),
            depth: Cell::new(0),
            memoize,
            vm_code: Some(code),
            vm_closure: closure,
        }
    }

    pub(crate) fn vm_code_arc(&self) -> Option<&Arc<CodeObject>> {
        self.vm_code.as_ref()
    }

    pub(crate) fn vm_closure_ref(&self) -> Option<&NativeClosureScope> {
        self.vm_closure.as_ref()
    }

    pub(crate) fn depth_cell(&self) -> &Cell<usize> {
        &self.depth
    }

    pub(crate) fn cache_ref(&self) -> &RefCell<HashMap<u64, Py<PyAny>>> {
        &self.cache
    }

    pub(crate) fn is_memoize(&self) -> bool {
        self.memoize
    }

    fn cache_key(py: Python<'_>, value: &Py<PyAny>) -> Option<u64> {
        value.bind(py).hash().ok().map(|h| h as u64)
    }
}

#[pymethods]
impl NDVmRecur {
    fn __call__(self_: &Bound<'_, Self>, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let this = self_.borrow();

        // Depth guard
        let depth = this.depth.get();
        if depth >= ND_MAX_RECURSION_DEPTH {
            set_nd_abort();
            return Err(pyo3::exceptions::PyRecursionError::new_err(
                "maximum ND recursion depth exceeded",
            ));
        }

        // Check memoization cache
        let key = if this.memoize {
            Self::cache_key(py, &value)
        } else {
            None
        };
        if let Some(k) = key {
            if let Some(cached) = this.cache.borrow().get(&k) {
                return Ok(cached.clone_ref(py));
            }
        }

        this.depth.set(depth + 1);
        let lambda = this.nd_lambda.clone_ref(py);
        drop(this); // release borrow before calling back into Python

        let result = lambda.bind(py).call1((value, self_));

        let this = self_.borrow();
        this.depth.set(this.depth.get() - 1);

        let result = result?;

        // Store in cache
        if let Some(k) = key {
            this.cache.borrow_mut().insert(k, result.clone().unbind());
        }

        Ok(result.unbind())
    }

    fn __repr__(&self) -> String {
        "NDVmRecur(~~)".to_string()
    }
}

// ---------------------------------------------------------------------------
// Thread-safe variant for rayon parallel ND broadcast
// ---------------------------------------------------------------------------

/// Thread-safe ND declaration wrapper for parallel broadcast.
///
/// Same semantics as `NDVmDecl` but uses `Arc<Mutex<_>>` for the
/// memoization cache and `AtomicUsize` for the depth counter, so rayon
/// worker threads can each hold a reference.
#[pyclass(name = "NDParallelDecl", module = "catnip._rs")]
pub struct NDParallelDecl {
    nd_lambda: Py<PyAny>,
    memoize: bool,
}

impl NDParallelDecl {
    pub fn new(nd_lambda: Py<PyAny>) -> Self {
        Self {
            nd_lambda,
            memoize: true,
        }
    }

    pub fn with_memoize(nd_lambda: Py<PyAny>, memoize: bool) -> Self {
        Self { nd_lambda, memoize }
    }
}

#[pymethods]
impl NDParallelDecl {
    fn __call__(self_: &Bound<'_, Self>, py: Python<'_>, seed: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let this = self_.borrow();
        let lambda = this.nd_lambda.clone_ref(py);
        let memoize = this.memoize;
        drop(this);
        let recur = Py::new(py, NDParallelRecur::with_memoize(lambda.clone_ref(py), memoize))?;
        lambda.bind(py).call1((seed, &recur)).map(|r| r.unbind())
    }

    fn __repr__(&self) -> String {
        "NDParallelDecl(~~)".to_string()
    }
}

/// Thread-safe recur handle for parallel ND recursion.
///
/// Uses `Arc<Mutex<HashMap>>` for memoization and `AtomicUsize` for depth,
/// making it `Send + Sync` for rayon's `par_iter`.
#[pyclass(name = "NDParallelRecur", module = "catnip._rs")]
pub struct NDParallelRecur {
    nd_lambda: Py<PyAny>,
    cache: Arc<Mutex<HashMap<u64, Py<PyAny>>>>,
    depth: Arc<AtomicUsize>,
    memoize: bool,
}

impl NDParallelRecur {
    pub fn new(nd_lambda: Py<PyAny>) -> Self {
        Self::with_memoize(nd_lambda, true)
    }

    pub fn with_memoize(nd_lambda: Py<PyAny>, memoize: bool) -> Self {
        Self {
            nd_lambda,
            cache: Arc::new(Mutex::new(HashMap::new())),
            depth: Arc::new(AtomicUsize::new(0)),
            memoize,
        }
    }

    /// Create a recur handle sharing an existing cache and depth counter.
    pub fn with_shared(
        nd_lambda: Py<PyAny>,
        cache: Arc<Mutex<HashMap<u64, Py<PyAny>>>>,
        depth: Arc<AtomicUsize>,
        memoize: bool,
    ) -> Self {
        Self {
            nd_lambda,
            cache,
            depth,
            memoize,
        }
    }

    fn cache_key(py: Python<'_>, value: &Py<PyAny>) -> Option<u64> {
        value.bind(py).hash().ok().map(|h| h as u64)
    }
}

#[pymethods]
impl NDParallelRecur {
    fn __call__(self_: &Bound<'_, Self>, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        let this = self_.borrow();

        // Depth guard (atomic)
        let depth = this.depth.fetch_add(1, Ordering::Relaxed);
        if depth >= ND_MAX_RECURSION_DEPTH {
            this.depth.fetch_sub(1, Ordering::Relaxed);
            set_nd_abort();
            return Err(pyo3::exceptions::PyRecursionError::new_err(
                "maximum ND recursion depth exceeded",
            ));
        }

        // Check memoization cache
        let key = if this.memoize {
            Self::cache_key(py, &value)
        } else {
            None
        };
        if let Some(k) = key {
            if let Ok(guard) = this.cache.lock() {
                if let Some(cached) = guard.get(&k) {
                    this.depth.fetch_sub(1, Ordering::Relaxed);
                    return Ok(cached.clone_ref(py));
                }
            }
        }

        let lambda = this.nd_lambda.clone_ref(py);
        drop(this); // release borrow before calling back into Python

        let result = lambda.bind(py).call1((value, self_));

        let this = self_.borrow();
        this.depth.fetch_sub(1, Ordering::Relaxed);

        let result = result?;

        // Store in cache
        if let Some(k) = key {
            if let Ok(mut guard) = this.cache.lock() {
                guard.insert(k, result.clone().unbind());
            }
        }

        Ok(result.unbind())
    }

    fn __repr__(&self) -> String {
        "NDParallelRecur(~~)".to_string()
    }
}

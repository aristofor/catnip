// FILE: catnip_rs/src/vm/iter.rs
//! Native iterators for common Python sequences.

use super::value::Value;
use pyo3::prelude::*;
use pyo3::types::{PyList, PyTuple};

#[derive(Clone, Copy)]
enum SeqKind {
    List,
    Tuple,
}

/// Iterator over list/tuple values without Python-level `next()` calls.
#[pyclass(module = "catnip._rs")]
pub struct SeqIter {
    seq: Py<PyAny>,
    index: usize,
    len: usize,
    kind: SeqKind,
}

impl SeqIter {
    pub fn from_list(list: &Bound<'_, PyList>) -> PyResult<Self> {
        Ok(Self {
            seq: list.to_owned().into_any().unbind(),
            index: 0,
            len: list.len(),
            kind: SeqKind::List,
        })
    }

    pub fn from_tuple(tuple: &Bound<'_, PyTuple>) -> PyResult<Self> {
        Ok(Self {
            seq: tuple.to_owned().into_any().unbind(),
            index: 0,
            len: tuple.len(),
            kind: SeqKind::Tuple,
        })
    }

    pub fn next_value(&mut self, py: Python<'_>) -> PyResult<Option<Value>> {
        if self.index >= self.len {
            return Ok(None);
        }
        // Direct FFI access: skip PyO3 cast + bounds check per iteration
        let item_ptr = unsafe {
            match self.kind {
                SeqKind::List => pyo3::ffi::PyList_GetItem(self.seq.as_ptr(), self.index as pyo3::ffi::Py_ssize_t),
                SeqKind::Tuple => pyo3::ffi::PyTuple_GetItem(self.seq.as_ptr(), self.index as pyo3::ffi::Py_ssize_t),
            }
        };
        if item_ptr.is_null() {
            return Err(PyErr::take(py)
                .unwrap_or_else(|| pyo3::exceptions::PyIndexError::new_err("sequence index out of range")));
        }
        self.index += 1;
        let item = unsafe { pyo3::Bound::from_borrowed_ptr(py, item_ptr) };
        Ok(Some(Value::from_pyobject(py, &item)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyList, PyTuple};

    #[test]
    fn seq_iter_list_basic() {
        Python::attach(|py| {
            let list = PyList::new(py, [1, 2, 3]).unwrap();
            let mut iter = SeqIter::from_list(&list).unwrap();

            assert_eq!(iter.next_value(py).unwrap().unwrap().as_int(), Some(1));
            assert_eq!(iter.next_value(py).unwrap().unwrap().as_int(), Some(2));
            assert_eq!(iter.next_value(py).unwrap().unwrap().as_int(), Some(3));
            assert!(iter.next_value(py).unwrap().is_none());
        });
    }

    #[test]
    fn seq_iter_tuple_basic() {
        Python::attach(|py| {
            let tuple = PyTuple::new(py, [10, 20]).unwrap();
            let mut iter = SeqIter::from_tuple(&tuple).unwrap();

            assert_eq!(iter.next_value(py).unwrap().unwrap().as_int(), Some(10));
            assert_eq!(iter.next_value(py).unwrap().unwrap().as_int(), Some(20));
            assert!(iter.next_value(py).unwrap().is_none());
        });
    }
}

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
        let value = match self.kind {
            SeqKind::List => {
                let list = self.seq.bind(py).cast::<PyList>()?;
                let len = list.len();
                if self.index >= len {
                    return Ok(None);
                }
                let item = list.get_item(self.index)?;
                Value::from_pyobject(py, &item)?
            }
            SeqKind::Tuple => {
                if self.index >= self.len {
                    return Ok(None);
                }
                let tuple = self.seq.bind(py).cast::<PyTuple>()?;
                let item = tuple.get_item(self.index)?;
                Value::from_pyobject(py, &item)?
            }
        };

        self.index += 1;
        Ok(Some(value))
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

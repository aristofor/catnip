// FILE: catnip_rs/src/core/registry/literals.rs
//! Literal operations: list_literal, tuple_literal, set_literal, dict_literal, fstring

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};

impl Registry {
    /// Create a list literal from items, evaluating each item.
    pub(crate) fn op_list_literal(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            list.append(value)?;
        }
        Ok(list.into())
    }

    /// Create a tuple literal from items, evaluating each item.
    pub(crate) fn op_tuple_literal(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let mut temp = Vec::new();
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            temp.push(value);
        }
        Ok(PyTuple::new(py, &temp)?.into())
    }

    /// Create a set literal from items, evaluating each item.
    pub(crate) fn op_set_literal(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let set = PySet::empty(py)?;
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            set.add(value)?;
        }
        Ok(set.into())
    }

    /// Create a dict literal from key-value pairs, evaluating keys and values
    pub(crate) fn op_dict_literal(
        &self,
        py: Python<'_>,
        pairs: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let dict = PyDict::new(py);

        for i in 0..pairs.len() {
            let pair = pairs.get_item(i)?;

            // Each pair should be a tuple (key, value)
            if let Ok(pair_tuple) = pair.cast::<PyTuple>() {
                if pair_tuple.len() == 2 {
                    let key_node = pair_tuple.get_item(0)?.unbind();
                    let value_node = pair_tuple.get_item(1)?.unbind();

                    let key = self.exec_stmt_impl(py, key_node)?;
                    let value = self.exec_stmt_impl(py, value_node)?;

                    dict.set_item(key, value)?;
                }
            }
        }

        Ok(dict.into())
    }

    /// Evaluate an f-string template with interpolated expressions
    ///
    /// Parts are tuples: ('text', str) or ('expr', expr_node)
    pub(crate) fn op_fstring(
        &self,
        py: Python<'_>,
        parts: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        let mut result = String::new();

        for i in 0..parts.len() {
            let part = parts.get_item(i)?;

            // Each part should be a tuple (type, value)
            if let Ok(part_tuple) = part.cast::<PyTuple>() {
                if part_tuple.len() == 2 {
                    let part_type: String = part_tuple.get_item(0)?.extract()?;
                    let part_value = part_tuple.get_item(1)?;

                    if part_type == "text" {
                        // Direct text, just extract as string
                        let text: String = part_value.extract()?;
                        result.push_str(&text);
                    } else if part_type == "expr" {
                        // Expression node, evaluate it
                        let expr_result = self.exec_stmt_impl(py, part_value.unbind())?;
                        // Convert to string using Python str()
                        let str_value = expr_result.bind(py).str()?;
                        let text: String = str_value.extract()?;
                        result.push_str(&text);
                    }
                }
            }
        }

        Ok(result.into_pyobject(py)?.unbind().into())
    }
}

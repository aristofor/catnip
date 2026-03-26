// FILE: catnip_rs/src/core/registry/literals.rs
//! Literal operations: list_literal, tuple_literal, set_literal, dict_literal, fstring

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PySet, PyTuple};

impl Registry {
    /// Create a list literal from items, evaluating each item.
    pub(crate) fn op_list_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            list.append(value)?;
        }
        Ok(list.into())
    }

    /// Create a tuple literal from items, evaluating each item.
    pub(crate) fn op_tuple_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let mut temp = Vec::new();
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            temp.push(value);
        }
        Ok(PyTuple::new(py, &temp)?.into())
    }

    /// Create a set literal from items, evaluating each item.
    pub(crate) fn op_set_literal(&self, py: Python<'_>, items: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let set = PySet::empty(py)?;
        for i in 0..items.len() {
            let item = items.get_item(i)?.unbind();
            let value = self.exec_stmt_impl(py, item)?;
            set.add(value)?;
        }
        Ok(set.into())
    }

    /// Create a dict literal from key-value pairs, evaluating keys and values
    pub(crate) fn op_dict_literal(&self, py: Python<'_>, pairs: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
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
    /// Parts: String (text) or Tuple(expr, Int(conv), spec)
    /// conv: 0=none, 1=str, 2=repr, 3=ascii
    pub(crate) fn op_fstring(&self, py: Python<'_>, parts: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let builtins = py.import("builtins")?;
        let format_fn = builtins.getattr("format")?;
        let mut result = String::new();

        for i in 0..parts.len() {
            let part = parts.get_item(i)?;

            if let Ok(text) = part.extract::<String>() {
                // Text part
                result.push_str(&text);
            } else if let Ok(tuple) = part.cast::<PyTuple>() {
                // Interpolation: (expr, conv, spec)
                let expr = tuple.get_item(0)?;
                let conv: i64 = tuple.get_item(1)?.extract()?;
                let spec_node = tuple.get_item(2)?;

                let value = self.exec_stmt_impl(py, expr.unbind())?;

                // Apply conversion
                let converted = match conv {
                    1 => builtins.getattr("str")?.call1((value.bind(py),))?.unbind(),
                    2 => builtins.getattr("repr")?.call1((value.bind(py),))?.unbind(),
                    3 => builtins.getattr("ascii")?.call1((value.bind(py),))?.unbind(),
                    _ => value,
                };

                // Apply format spec
                let spec: String = if spec_node.is_none() {
                    String::new()
                } else {
                    spec_node.extract()?
                };

                let formatted = format_fn.call1((converted.bind(py), spec))?.extract::<String>()?;
                result.push_str(&formatted);
            }
        }

        Ok(result.into_pyobject(py)?.unbind().into())
    }
}

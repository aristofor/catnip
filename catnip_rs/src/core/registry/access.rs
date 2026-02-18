// FILE: catnip_rs/src/core/registry/access.rs
//! Access operations: getattr, getitem, setattr, setitem, slice

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Get an attribute from an object: getattr(parent, ident)
    pub(crate) fn op_getattr(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "getattr requires 2 arguments: parent, ident",
            ));
        }

        let parent_node = args.get_item(0)?.unbind();
        let ident_node = args.get_item(1)?.unbind();

        let parent = self.exec_stmt_impl(py, parent_node)?;
        let ident = self.exec_stmt_impl(py, ident_node)?;

        // Extract ident as string
        let ident_str: String = ident.bind(py).extract()?;

        // Use Python's getattr
        Ok(parent.bind(py).getattr(ident_str)?.unbind())
    }

    /// Get an item from an object: obj[index]
    pub(crate) fn op_getitem(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "getitem requires 2 arguments: obj, index",
            ));
        }

        let obj_node = args.get_item(0)?.unbind();
        let index_node = args.get_item(1)?.unbind();

        let obj = self.exec_stmt_impl(py, obj_node)?;
        let index = self.exec_stmt_impl(py, index_node)?;

        // Use Python's __getitem__
        Ok(obj.bind(py).get_item(index)?.unbind())
    }

    /// Set an attribute on an object: setattr(obj, attr, value)
    pub(crate) fn op_setattr(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 3 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "setattr requires 3 arguments: obj, attr, value",
            ));
        }

        let obj_node = args.get_item(0)?.unbind();
        let attr_node = args.get_item(1)?.unbind();
        let value_node = args.get_item(2)?.unbind();

        let obj = self.exec_stmt_impl(py, obj_node)?;
        let attr = self.exec_stmt_impl(py, attr_node)?;
        let value = self.exec_stmt_impl(py, value_node)?;

        // Extract attr as string
        let attr_str: String = attr.bind(py).extract()?;

        // Use Python's setattr
        obj.bind(py).setattr(attr_str, value.clone_ref(py))?;

        // Return the value
        Ok(value)
    }

    /// Set an item in an object: obj[index] = value
    pub(crate) fn op_setitem(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 3 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "setitem requires 3 arguments: obj, index, value",
            ));
        }

        let obj_node = args.get_item(0)?.unbind();
        let index_node = args.get_item(1)?.unbind();
        let value_node = args.get_item(2)?.unbind();

        let obj = self.exec_stmt_impl(py, obj_node)?;
        let index = self.exec_stmt_impl(py, index_node)?;
        let value = self.exec_stmt_impl(py, value_node)?;

        // Use Python's __setitem__
        obj.bind(py).set_item(index, value.clone_ref(py))?;

        // Return the value
        Ok(value)
    }

    /// Create a slice object: slice(start, stop, step)
    pub(crate) fn op_slice(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() < 3 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "slice requires 3 arguments: start, stop, step",
            ));
        }

        let start_node = args.get_item(0)?.unbind();
        let stop_node = args.get_item(1)?.unbind();
        let step_node = args.get_item(2)?.unbind();

        let start = self.exec_stmt_impl(py, start_node)?;
        let stop = self.exec_stmt_impl(py, stop_node)?;
        let step = self.exec_stmt_impl(py, step_node)?;

        // Create Python slice object
        let slice_fn = py.import("builtins")?.getattr("slice")?;
        Ok(slice_fn.call1((start, stop, step))?.unbind())
    }
}

// FILE: catnip_rs/src/core/registry/bitwise.rs
//! Bitwise operations: bit_or, bit_xor, bit_and, lshift, rshift

use super::Registry;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

impl Registry {
    /// Bitwise OR: fold left with __or__
    pub(crate) fn op_bit_or(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        self.fold_left_magic(py, items, "__or__")
    }

    /// Bitwise XOR: fold left with __xor__
    pub(crate) fn op_bit_xor(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        self.fold_left_magic(py, items, "__xor__")
    }

    /// Bitwise AND: fold left with __and__
    pub(crate) fn op_bit_and(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        self.fold_left_magic(py, items, "__and__")
    }

    /// Left shift: fold left with __lshift__
    pub(crate) fn op_lshift(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        self.fold_left_magic(py, items, "__lshift__")
    }

    /// Right shift: fold left with __rshift__
    pub(crate) fn op_rshift(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        self.fold_left_magic(py, items, "__rshift__")
    }
}

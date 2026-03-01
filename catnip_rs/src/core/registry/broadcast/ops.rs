// FILE: catnip_rs/src/core/registry/broadcast/ops.rs
//
// Rust port of catnip/core/broadcast_ops.pyx
//
// Provides optimized broadcast utility functions:
// - is_boolean_mask: Check if operand is a boolean mask
// - filter_by_mask: Filter by boolean mask
// - filter_conditional: Filter with condition function
// - broadcast_map: Map function over elements

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyList, PyTuple};
use pyo3::PyTypeInfo;

/// Check if operand is a boolean mask (list/tuple of booleans).
///
/// Port of: catnip/core/broadcast_ops.pyx::is_boolean_mask()
///
/// :param operand: Object to test
/// :return: True if it's a boolean mask
pub fn is_boolean_mask(_py: Python<'_>, operand: &Bound<'_, PyAny>) -> PyResult<bool> {
    // Must be list or tuple
    if !operand.is_instance_of::<PyList>() && !operand.is_instance_of::<PyTuple>() {
        return Ok(false);
    }

    // Empty lists/tuples are valid (trivial) boolean masks
    let len = operand.len()?;
    if len == 0 {
        return Ok(true);
    }

    // All elements must be bool
    // Use iteration to check each element type
    for item in operand.try_iter()? {
        let item = item?;
        // Check if type(item) is bool (not isinstance, exact type check)
        if !item.is_instance_of::<PyBool>() {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Filter an iterable by boolean mask: target.[mask]
///
/// Port of: catnip/core/broadcast_ops.pyx::filter_by_mask()
///
/// :param target: Collection to filter
/// :param mask: Boolean mask (list/tuple of bool)
/// :return: Elements of target where mask[i] is True
/// :raises ValueError: If sizes are incompatible
/// :raises TypeError: If mask is not a boolean mask
pub fn filter_by_mask(
    py: Python<'_>,
    target: &Bound<'_, PyAny>,
    mask: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Validate mask
    if !is_boolean_mask(py, mask)? {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "Mask must be a list or tuple of booleans",
        ));
    }

    // Preserve original type
    let target_is_tuple = target.is_instance_of::<PyTuple>();
    let target_is_list = target.is_instance_of::<PyList>();

    // Convert to list if needed for uniform processing
    let target_iter: Bound<'_, PyAny> = if target_is_list || target_is_tuple {
        target.clone()
    } else {
        // Try to convert to list
        match target.try_iter() {
            Ok(iter) => {
                let list = PyList::empty(py);
                for item in iter {
                    list.append(item?)?;
                }
                list.into_any()
            }
            Err(_) => {
                return Err(pyo3::exceptions::PyTypeError::new_err(format!(
                    "Cannot filter {} with boolean mask",
                    target.get_type().name()?
                )));
            }
        }
    };

    // Check size compatibility
    let target_len = target_iter.len()?;
    let mask_len = mask.len()?;
    if target_len != mask_len {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "Mask size mismatch: target has {} elements, mask has {}",
            target_len, mask_len
        )));
    }

    // Filter using zip
    let result_list = PyList::empty(py);
    let target_iter = target_iter.try_iter()?;
    let mask_iter = mask.try_iter()?;

    for (item, mask_val) in target_iter.zip(mask_iter) {
        let item = item?;
        let mask_val = mask_val?;
        // Check if mask value is truthy
        if mask_val.is_truthy()? {
            result_list.append(item)?;
        }
    }

    // Preserve original type
    if target_is_tuple {
        Ok(PyTuple::new(py, &result_list)?.into())
    } else {
        Ok(result_list.into())
    }
}

/// Conditional filter: target.[if condition]
///
/// Port of: catnip/core/broadcast_ops.pyx::filter_conditional()
///
/// Applies condition_func to each element and keeps those for which it returns True.
///
/// :param target: Collection to filter
/// :param condition_func: Condition function (callable)
/// :return: Elements where condition_func(elem) is True
pub fn filter_conditional(
    py: Python<'_>,
    target: &Bound<'_, PyAny>,
    condition_func: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Validate callable
    if !condition_func.is_callable() {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "Filter condition must be callable",
        ));
    }

    // Check if target is a scalar type
    let target_type = target.get_type();
    let type_name_bound = target_type.name()?;
    let type_name = type_name_bound.to_str()?;
    let is_scalar = matches!(type_name, "int" | "float" | "str" | "bool" | "NoneType");

    if is_scalar {
        // Scalar: apply condition directly
        let result = condition_func.call1((target,))?;
        if result.is_truthy()? {
            // Return as single-element list
            let result_list = PyList::empty(py);
            result_list.append(target)?;
            return Ok(result_list.into());
        } else {
            return Ok(PyList::empty(py).into());
        }
    }

    // Preserve original type
    let target_is_tuple = target.is_instance_of::<PyTuple>();
    let target_is_list = target.is_instance_of::<PyList>();

    // Try iteration
    let result_list = PyList::empty(py);

    if target_is_list || target_is_tuple {
        // Filter elements using iteration
        for item in target.try_iter()? {
            let item = item?;
            let cond_result = condition_func.call1((&item,))?;
            if cond_result.is_truthy()? {
                result_list.append(&item)?;
            }
        }
    } else {
        // Try iteration for other types
        match target.try_iter() {
            Ok(iter) => {
                for item in iter {
                    let item = item?;
                    let cond_result = condition_func.call1((item.clone(),))?;
                    if cond_result.is_truthy()? {
                        result_list.append(item)?;
                    }
                }
            }
            Err(_) => {
                // Not iterable, treat as scalar
                let result = condition_func.call1((target,))?;
                if result.is_truthy()? {
                    result_list.append(target)?;
                }
            }
        }
    }

    // Preserve original type
    if target_is_tuple {
        Ok(PyTuple::new(py, &result_list)?.into())
    } else {
        Ok(result_list.into())
    }
}

/// Map a function over all elements: target.[func]
///
/// Port of: catnip/core/broadcast_ops.pyx::broadcast_map()
///
/// Optimized version of callable broadcast.
///
/// :param target: Collection to map
/// :param func: Function to apply
/// :return: Collection with func applied to each element
pub fn broadcast_map(
    py: Python<'_>,
    target: &Bound<'_, PyAny>,
    func: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    // Validate callable
    if !func.is_callable() {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "Broadcast operator must be callable",
        ));
    }

    // Check if target is a scalar type
    let target_type = target.get_type();
    let type_name_bound = target_type.name()?;
    let type_name = type_name_bound.to_str()?;
    let is_scalar = matches!(type_name, "int" | "float" | "str" | "bool" | "NoneType");

    if is_scalar {
        // Scalar: apply directly
        return func.call1((target,)).map(|r| r.into());
    }

    // Check exact type (not isinstance)
    let target_is_list = target.get_type().is(PyList::type_object(py));
    let target_is_tuple = target.get_type().is(PyTuple::type_object(py));

    // Lists: recurse into elements, return list
    if target_is_list {
        let result_list = PyList::empty(py);
        for item in target.try_iter()? {
            let item = item?;
            result_list.append(broadcast_map(py, &item, func)?)?;
        }
        return Ok(result_list.into());
    }

    // Tuples: recurse into elements, return tuple
    if target_is_tuple {
        let result_list = PyList::empty(py);
        for item in target.try_iter()? {
            let item = item?;
            result_list.append(broadcast_map(py, &item, func)?)?;
        }
        return Ok(PyTuple::new(py, &result_list)?.into());
    }

    // Other iterables: recurse into elements
    match target.try_iter() {
        Ok(iter) => {
            let result_list = PyList::empty(py);
            for item in iter {
                let item = item?;
                result_list.append(broadcast_map(py, &item, func)?)?;
            }
            Ok(result_list.into())
        }
        Err(_) => {
            // Not iterable, non-scalar (struct, etc.): treat as leaf
            func.call1((target,)).map(|r| r.into())
        }
    }
}

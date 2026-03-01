// FILE: catnip_rs/src/core/registry/broadcast/simd.rs
// SIMD fast paths for broadcasting on homogeneous numeric lists.
//
// Intercepts "list of ints/floats + arithmetic/comparison operator" patterns
// and executes them in tight Rust loops, with no per-element PyO3 crossing.
// LLVM auto-vectorizes these loops to AVX2/SSE2 at opt-level=3.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyFloat, PyInt, PyList};

/// Detected numeric type for a homogeneous list.
enum NumericData {
    Ints(Vec<i64>),
    Floats(Vec<f64>),
}

/// Extract data from a homogeneous PyList into a contiguous Rust Vec.
/// Returns None if the list is empty, non-homogeneous, or non-numeric.
fn extract_numeric(_py: Python<'_>, target: &Bound<'_, PyAny>) -> Option<NumericData> {
    // Seulement les listes (pas tuples pour l'instant)
    let list = target.cast::<PyList>().ok()?;
    let len = list.len();
    if len == 0 {
        return None;
    }

    // Inspecter le premier élément pour décider du type
    let first = list.get_item(0).ok()?;

    // Attention : bool est sous-classe de int en Python, on l'exclut
    if first.is_instance_of::<PyBool>() {
        return None;
    }

    if first.is_instance_of::<PyInt>() {
        let mut vals = Vec::with_capacity(len);
        for i in 0..len {
            let item = list.get_item(i).ok()?;
            if item.is_instance_of::<PyBool>() {
                return None;
            }
            vals.push(item.extract::<i64>().ok()?);
        }
        Some(NumericData::Ints(vals))
    } else if first.is_instance_of::<PyFloat>() {
        let mut vals = Vec::with_capacity(len);
        for i in 0..len {
            let item = list.get_item(i).ok()?;
            vals.push(item.extract::<f64>().ok()?);
        }
        Some(NumericData::Floats(vals))
    } else {
        None
    }
}

// --- Opérations vectorisées sur i64 ---

fn arith_int(data: &[i64], op: &str, operand: i64) -> Option<Vec<i64>> {
    match op {
        "+" => Some(data.iter().map(|&x| x.wrapping_add(operand)).collect()),
        "-" => Some(data.iter().map(|&x| x.wrapping_sub(operand)).collect()),
        "*" => Some(data.iter().map(|&x| x.wrapping_mul(operand)).collect()),
        "%" => {
            if operand == 0 {
                return None; // division par zéro -> fallback Python pour l'erreur
            }
            Some(data.iter().map(|&x| x % operand).collect())
        }
        "//" => {
            if operand == 0 {
                return None;
            }
            // Floor division Python : résultat arrondi vers -inf
            Some(data.iter().map(|&x| x.div_euclid(operand)).collect())
        }
        "**" => {
            if operand < 0 {
                return None; // puissance négative -> float, fallback
            }
            let exp = operand as u32;
            Some(data.iter().map(|&x| x.wrapping_pow(exp)).collect())
        }
        _ => None,
    }
}

fn arith_float(data: &[f64], op: &str, operand: f64) -> Option<Vec<f64>> {
    match op {
        "+" => Some(data.iter().map(|&x| x + operand).collect()),
        "-" => Some(data.iter().map(|&x| x - operand).collect()),
        "*" => Some(data.iter().map(|&x| x * operand).collect()),
        "/" => Some(data.iter().map(|&x| x / operand).collect()),
        "//" => Some(data.iter().map(|&x| (x / operand).floor()).collect()),
        "%" => Some(data.iter().map(|&x| x % operand).collect()),
        "**" => Some(data.iter().map(|&x| x.powf(operand)).collect()),
        _ => None,
    }
}

// "/" sur ints produit des floats (sémantique Python)
fn div_int_to_float(data: &[i64], operand: i64) -> Option<Vec<f64>> {
    if operand == 0 {
        return None;
    }
    Some(
        data.iter()
            .map(|&x| (x as f64) / (operand as f64))
            .collect(),
    )
}

// --- Comparaisons vectorisées ---

fn cmp_int(data: &[i64], op: &str, operand: i64) -> Option<Vec<bool>> {
    match op {
        ">" => Some(data.iter().map(|&x| x > operand).collect()),
        "<" => Some(data.iter().map(|&x| x < operand).collect()),
        ">=" => Some(data.iter().map(|&x| x >= operand).collect()),
        "<=" => Some(data.iter().map(|&x| x <= operand).collect()),
        "==" => Some(data.iter().map(|&x| x == operand).collect()),
        "!=" => Some(data.iter().map(|&x| x != operand).collect()),
        _ => None,
    }
}

fn cmp_float(data: &[f64], op: &str, operand: f64) -> Option<Vec<bool>> {
    match op {
        ">" => Some(data.iter().map(|&x| x > operand).collect()),
        "<" => Some(data.iter().map(|&x| x < operand).collect()),
        ">=" => Some(data.iter().map(|&x| x >= operand).collect()),
        "<=" => Some(data.iter().map(|&x| x <= operand).collect()),
        "==" => Some(data.iter().map(|&x| x == operand).collect()),
        "!=" => Some(data.iter().map(|&x| x != operand).collect()),
        _ => None,
    }
}

// --- Filtrage vectorisé ---

fn filter_int(data: &[i64], op: &str, operand: i64) -> Option<Vec<i64>> {
    let pred: Box<dyn Fn(&i64) -> bool> = match op {
        ">" => Box::new(move |&x| x > operand),
        "<" => Box::new(move |&x| x < operand),
        ">=" => Box::new(move |&x| x >= operand),
        "<=" => Box::new(move |&x| x <= operand),
        "==" => Box::new(move |&x| x == operand),
        "!=" => Box::new(move |&x| x != operand),
        _ => return None,
    };
    Some(data.iter().copied().filter(pred).collect())
}

fn filter_float(data: &[f64], op: &str, operand: f64) -> Option<Vec<f64>> {
    let pred: Box<dyn Fn(&f64) -> bool> = match op {
        ">" => Box::new(move |&x| x > operand),
        "<" => Box::new(move |&x| x < operand),
        ">=" => Box::new(move |&x| x >= operand),
        "<=" => Box::new(move |&x| x <= operand),
        "==" => Box::new(move |&x| x == operand),
        "!=" => Box::new(move |&x| x != operand),
        _ => return None,
    };
    Some(data.iter().copied().filter(pred).collect())
}

// --- Construction résultat Python ---

fn build_int_list(py: Python<'_>, values: &[i64]) -> PyResult<Py<PyAny>> {
    Ok(PyList::new(py, values)?.into_any().unbind())
}

fn build_float_list(py: Python<'_>, values: &[f64]) -> PyResult<Py<PyAny>> {
    Ok(PyList::new(py, values)?.into_any().unbind())
}

fn build_bool_list(py: Python<'_>, values: &[bool]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for &v in values {
        list.append(v)?;
    }
    Ok(list.into_any().unbind())
}

// --- Points d'entrée publics ---

/// Attempt the SIMD fast path for a map broadcast: target.[op operand]
/// Returns None if not applicable (non-numeric type, unknown operator, etc.)
pub fn try_simd_map(
    py: Python<'_>,
    target: &Bound<'_, PyAny>,
    op: &str,
    operand: &Bound<'_, PyAny>,
) -> Option<PyResult<Py<PyAny>>> {
    let data = extract_numeric(py, target)?;

    match data {
        NumericData::Ints(ref vals) => {
            // "/" sur ints -> floats
            if op == "/" {
                let operand_val = operand.extract::<i64>().ok()?;
                let result = div_int_to_float(vals, operand_val)?;
                return Some(build_float_list(py, &result));
            }

            // Ops arithmétiques int -> int
            if let Some(operand_val) = operand.extract::<i64>().ok() {
                // Arithmétique
                if let Some(result) = arith_int(vals, op, operand_val) {
                    return Some(build_int_list(py, &result));
                }
                // Comparaisons
                if let Some(result) = cmp_int(vals, op, operand_val) {
                    return Some(build_bool_list(py, &result));
                }
            }
            None
        }
        NumericData::Floats(ref vals) => {
            if let Some(operand_val) = operand.extract::<f64>().ok() {
                // Arithmétique
                if let Some(result) = arith_float(vals, op, operand_val) {
                    return Some(build_float_list(py, &result));
                }
                // Comparaisons
                if let Some(result) = cmp_float(vals, op, operand_val) {
                    return Some(build_bool_list(py, &result));
                }
            }
            None
        }
    }
}

/// Attempt the SIMD fast path for a filter broadcast: target.[if op operand]
/// Returns None if not applicable.
pub fn try_simd_filter(
    py: Python<'_>,
    target: &Bound<'_, PyAny>,
    op: &str,
    operand: &Bound<'_, PyAny>,
) -> Option<PyResult<Py<PyAny>>> {
    let data = extract_numeric(py, target)?;

    match data {
        NumericData::Ints(ref vals) => {
            let operand_val = operand.extract::<i64>().ok()?;
            let result = filter_int(vals, op, operand_val)?;
            Some(build_int_list(py, &result))
        }
        NumericData::Floats(ref vals) => {
            let operand_val = operand.extract::<f64>().ok()?;
            let result = filter_float(vals, op, operand_val)?;
            Some(build_float_list(py, &result))
        }
    }
}

// FILE: catnip_rs/src/core/registry/patterns.rs
//! Pattern matching operations: match, match_pattern, match_tuple_pattern
//!
//! Implements Catnip's pattern matching system with support for:
//! - Literal patterns: match specific values
//! - Variable patterns: capture matched values
//! - Wildcard patterns: match anything without capture
//! - OR patterns: try multiple patterns
//! - Tuple patterns: destructure iterables (with star support)

use super::Registry;
use crate::constants::*;
use crate::core::pattern::*;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

type PatternBindings = Vec<(String, Py<PyAny>)>;

impl Registry {
    /// Execute a match expression with pattern matching.
    ///
    /// Args:
    ///     value_expr: Expression to match against (unevaluated)
    ///     cases: Tuple of (pattern, guard, body) tuples
    ///
    /// Returns:
    ///     Result of the matched case body, or None if no match
    pub(crate) fn op_match(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if args.len() < 2 {
            return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                "match requires 2 arguments: value_expr, cases",
            ));
        }

        let value_expr = args.get_item(0)?.unbind();
        let cases = args.get_item(1)?;

        // Evaluate the value expression
        let value = self.exec_stmt_impl(py, value_expr)?;

        // Get context for scope management
        let ctx = self.ctx.bind(py);
        let locals = ctx.getattr("locals")?;

        // Try each case in order
        for case_result in cases.try_iter()? {
            let case = case_result?;
            let case_tuple = case.cast::<PyTuple>()?;

            if case_tuple.len() < 3 {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "Each case must be a (pattern, guard, body) tuple",
                ));
            }

            let pattern = case_tuple.get_item(0)?;
            let guard = case_tuple.get_item(1)?;
            let body = case_tuple.get_item(2)?.unbind();

            // Try to match the pattern
            let bindings = self.match_pattern(py, &pattern, &value)?;

            if let Some(bindings_dict) = bindings {
                // Pattern matched, now check guard if present
                if !guard.is_none() {
                    // Guard scope must be isolated: captured bindings should not leak
                    // to outer variables when the guard fails.
                    let empty = PyDict::new(py);
                    ctx.call_method1("push_scope_with_capture", (empty,))?;

                    // Bind variables in temporary scope
                    for (var_name, var_value) in &bindings_dict {
                        locals.call_method1("_set", (var_name, var_value.clone_ref(py)))?;
                    }

                    // Evaluate guard, then always pop temporary scope.
                    let guard_eval = (|| -> PyResult<bool> {
                        let guard_result = self.exec_stmt_impl(py, guard.unbind())?;
                        guard_result.bind(py).is_truthy()
                    })();
                    ctx.call_method0("pop_scope")?;
                    let guard_passed = guard_eval?;

                    if !guard_passed {
                        // Guard failed, try next case
                        continue;
                    }
                }

                // Pattern matched and guard passed (or no guard), execute body
                // Bind variables directly in current scope (no new scope for body)
                for (var_name, var_value) in &bindings_dict {
                    locals.call_method1("_set", (var_name, var_value.clone_ref(py)))?;
                }

                let result = self.exec_stmt_impl(py, body)?;
                ctx.setattr("result", result.clone_ref(py))?;
                return Ok(result);
            }
        }

        // No pattern matched - raise CatnipRuntimeError if available
        let exc_class = py.import(PY_MOD_EXC).and_then(|m| m.getattr("CatnipRuntimeError"));
        match exc_class {
            Ok(cls) => Err(PyErr::from_value(cls.call1(("No matching pattern",))?)),
            Err(_) => Err(pyo3::exceptions::PyRuntimeError::new_err("No matching pattern")),
        }
    }

    /// Try to match a pattern against a value.
    ///
    /// Args:
    ///     pattern: Pattern object (PatternWildcard, PatternLiteral, PatternVar, PatternOr, PatternTuple)
    ///     value: Value to match
    ///
    /// Returns:
    ///     Some(Vec) with variable bindings if match succeeds, None if it fails
    pub(crate) fn match_pattern(
        &self,
        py: Python<'_>,
        pattern: &Bound<'_, PyAny>,
        value: &Py<PyAny>,
    ) -> PyResult<Option<PatternBindings>> {
        // Tag dispatch via downcast (pointer comparison, not string)
        let tag = get_pattern_tag(pattern).ok_or_else(|| {
            let type_name = pattern.get_type().name().map(|n| n.to_string()).unwrap_or_default();
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Unknown pattern type: {}", type_name))
        })?;

        match tag {
            TAG_WILDCARD => Ok(Some(Vec::new())),
            TAG_LITERAL => {
                let pat = pattern.cast::<PatternLiteral>().unwrap();
                let pattern_value_node = pat.borrow().value.clone_ref(py);
                let pattern_value = self.exec_stmt_impl(py, pattern_value_node)?;
                let is_equal = pattern_value.bind(py).eq(value.bind(py))?;
                if is_equal { Ok(Some(Vec::new())) } else { Ok(None) }
            }
            TAG_VAR => {
                let pat = pattern.cast::<PatternVar>().unwrap();
                let name = pat.borrow().name.clone();
                Ok(Some(vec![(name, value.clone_ref(py))]))
            }
            TAG_OR => {
                let pat = pattern.cast::<PatternOr>().unwrap();
                let patterns = pat.borrow().patterns.clone_ref(py);
                for sub_pattern_result in patterns.bind(py).try_iter()? {
                    let sub_pattern = sub_pattern_result?;
                    if let Some(bindings) = self.match_pattern(py, &sub_pattern, value)? {
                        return Ok(Some(bindings));
                    }
                }
                Ok(None)
            }
            TAG_TUPLE => {
                let pat = pattern.cast::<PatternTuple>().unwrap();
                let patterns = pat.borrow().patterns.clone_ref(py);
                self.match_tuple_pattern(py, patterns.bind(py), value)
            }
            TAG_STRUCT => {
                let pat = pattern.cast::<PatternStruct>().unwrap();
                let struct_name = pat.borrow().name.clone();
                let fields = pat.borrow().fields.clone_ref(py);

                // Check type name matches (CatnipStructProxy stores type_name directly)
                let value_type_name: String = if let Ok(proxy) = value.bind(py).cast::<crate::vm::CatnipStructProxy>() {
                    proxy.borrow().type_name.clone()
                } else {
                    value.bind(py).get_type().name()?.extract()?
                };
                if value_type_name != struct_name {
                    return Ok(None);
                }

                // Extract field values as bindings (missing field = no match)
                let mut bindings = Vec::new();
                for field_result in fields.bind(py).try_iter()? {
                    let field_name: String = field_result?.extract()?;
                    let field_value = match value.bind(py).getattr(field_name.as_str()) {
                        Ok(v) => v,
                        Err(_) => return Ok(None),
                    };
                    bindings.push((field_name, field_value.unbind()));
                }
                Ok(Some(bindings))
            }
            TAG_ENUM => {
                let pat = pattern.cast::<PatternEnum>().unwrap();
                let enum_name = pat.borrow().enum_name.clone();
                let variant_name = pat.borrow().variant_name.clone();
                // Resolve EnumName.variant via context lookup + getattr
                let ctx = self.ctx.bind(py);
                let enum_obj = ctx.call_method1("get_local", (enum_name.as_str(),))?;
                let variant_val = enum_obj.getattr(variant_name.as_str())?;
                let is_equal = variant_val.eq(value.bind(py))?;
                if is_equal { Ok(Some(Vec::new())) } else { Ok(None) }
            }
            _ => unreachable!(),
        }
    }

    /// Match a tuple pattern against a value: (a, b) or (a, *rest, z) or (a, (b, c))
    ///
    /// Args:
    ///     patterns: List of sub-patterns (can include PatternVar, PatternLiteral, PatternTuple, star tuples)
    ///     value: Value to match (must be iterable)
    ///
    /// Returns:
    ///     Some(Vec) with variable bindings if match succeeds, None if it fails
    fn match_tuple_pattern(
        &self,
        py: Python<'_>,
        patterns: &Bound<'_, PyAny>,
        value: &Py<PyAny>,
    ) -> PyResult<Option<PatternBindings>> {
        // Convert value to list
        let value_bound = value.bind(py);
        let values_list = match value_bound.try_iter() {
            Ok(iter) => {
                let values: Result<Vec<Py<PyAny>>, PyErr> = iter.map(|item| Ok(item?.unbind())).collect();
                values?
            }
            Err(_) => {
                // Value is not iterable, pattern doesn't match
                return Ok(None);
            }
        };

        // Convert patterns to list
        let patterns_iter = patterns.try_iter()?;
        let patterns_list: Vec<Bound<'_, PyAny>> = patterns_iter.collect::<PyResult<_>>()?;

        let n_patterns = patterns_list.len();
        let n_values = values_list.len();

        // Find star pattern if present
        let mut star_idx: Option<usize> = None;
        let mut star_name: Option<String> = None;

        for (i, pattern_item) in patterns_list.iter().enumerate() {
            // Check if pattern_item is a tuple with ('*', name)
            if let Ok(tuple) = pattern_item.cast::<PyTuple>() {
                if tuple.len() == 2 {
                    let first = tuple.get_item(0)?;
                    if let Ok(star_str) = first.extract::<String>() {
                        if star_str == "*" {
                            if star_idx.is_some() {
                                // Multiple star patterns not allowed
                                return Ok(None);
                            }
                            star_idx = Some(i);
                            star_name = Some(tuple.get_item(1)?.extract::<String>()?);
                        }
                    }
                }
            }
        }

        let mut bindings = Vec::new();

        if star_idx.is_none() {
            // No star: exact length match required
            if n_patterns != n_values {
                return Ok(None);
            }

            // Match each pattern against corresponding value
            for i in 0..n_patterns {
                let sub_bindings = self.match_pattern(py, &patterns_list[i], &values_list[i])?;
                if let Some(sub_bindings) = sub_bindings {
                    bindings.extend(sub_bindings);
                } else {
                    // Sub-pattern didn't match
                    return Ok(None);
                }
            }
            Ok(Some(bindings))
        } else if let Some(star_idx) = star_idx {
            // With star: need at least (n_patterns - 1) values
            let n_required = n_patterns - 1;

            if n_values < n_required {
                return Ok(None);
            }

            let n_before = star_idx;
            let n_after = n_patterns - star_idx - 1;

            // Match patterns before star
            for i in 0..n_before {
                let sub_bindings = self.match_pattern(py, &patterns_list[i], &values_list[i])?;
                if let Some(sub_bindings) = sub_bindings {
                    bindings.extend(sub_bindings);
                } else {
                    return Ok(None);
                }
            }

            // Match patterns after star (from the end)
            for i in 0..n_after {
                let value_idx = n_values - n_after + i;
                let pattern_idx = star_idx + 1 + i;
                let sub_bindings = self.match_pattern(py, &patterns_list[pattern_idx], &values_list[value_idx])?;
                if let Some(sub_bindings) = sub_bindings {
                    bindings.extend(sub_bindings);
                } else {
                    return Ok(None);
                }
            }

            // Bind star variable (everything in the middle)
            if let Some(star_name) = star_name {
                let star_values: Vec<Py<PyAny>> = values_list[n_before..(n_values - n_after)]
                    .iter()
                    .map(|v| v.clone_ref(py))
                    .collect();
                let star_list = PyList::new(py, &star_values)?;
                bindings.push((star_name, star_list.unbind().into()));
            }
            Ok(Some(bindings))
        } else {
            Ok(Some(bindings))
        }
    }
}

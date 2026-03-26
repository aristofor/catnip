// FILE: catnip_vm/src/vm/broadcast.rs
//! Broadcast and ND operations for PureVM -- pure Rust, no PyO3.
//!
//! Implements element-wise operations on lists/tuples (broadcast),
//! boolean mask filtering, and ND recursion/map operators.

use std::sync::Arc;

use crate::error::{VMError, VMResult};
use crate::host::{BinaryOp, VmHost};
use crate::ops::arith;
use crate::value::Value;

use super::core::PureVM;

/// Sentinel string for ND recursion callback handle.
pub(crate) const ND_RECUR_SENTINEL: &str = "__nd_recur__";

/// Sentinel tag for ND declaration wrapper (~~lambda).
pub(crate) const ND_DECL_TAG: &str = "__nd_decl__";

/// Sentinel tag for ND lift/map wrapper (~>func).
pub(crate) const ND_LIFT_TAG: &str = "__nd_lift__";

/// Maximum ND recursion depth.
const ND_MAX_DEPTH: usize = 10_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map operator name to BinaryOp.
fn str_to_binary_op(name: &str) -> Option<BinaryOp> {
    match name {
        "+" => Some(BinaryOp::Add),
        "-" => Some(BinaryOp::Sub),
        "*" => Some(BinaryOp::Mul),
        "/" => Some(BinaryOp::TrueDiv),
        "//" => Some(BinaryOp::FloorDiv),
        "%" => Some(BinaryOp::Mod),
        "**" => Some(BinaryOp::Pow),
        "<" => Some(BinaryOp::Lt),
        "<=" => Some(BinaryOp::Le),
        ">" => Some(BinaryOp::Gt),
        ">=" => Some(BinaryOp::Ge),
        _ => None,
    }
}

/// Extract items from a list or tuple as a Vec<Value>.
/// Clones refcounts for list items (as_slice_cloned does).
fn extract_items(val: Value) -> VMResult<Vec<Value>> {
    if val.is_native_list() {
        let list = unsafe { val.as_native_list_ref().unwrap() };
        Ok(list.as_slice_cloned())
    } else if val.is_native_tuple() {
        let tuple = unsafe { val.as_native_tuple_ref().unwrap() };
        let items: Vec<Value> = tuple.as_slice().to_vec();
        for v in &items {
            v.clone_refcount();
        }
        Ok(items)
    } else {
        Err(VMError::TypeError(format!(
            "broadcast target must be list or tuple, got {}",
            val.type_name()
        )))
    }
}

/// Wrap result items in list or tuple depending on the original target type.
#[inline]
fn wrap_result(items: Vec<Value>, is_tuple: bool) -> Value {
    if is_tuple {
        Value::from_tuple(items)
    } else {
        Value::from_list(items)
    }
}

/// Check if a value is a collection (list or tuple) for deep broadcast.
#[inline]
fn is_collection(val: Value) -> bool {
    val.is_native_list() || val.is_native_tuple()
}

/// Check if a value is a boolean mask (list/tuple of all bools).
fn is_boolean_mask(val: Value) -> bool {
    if val.is_native_list() {
        let list = unsafe { val.as_native_list_ref().unwrap() };
        if list.is_empty() {
            return false;
        }
        let items = list.as_slice_cloned();
        let result = items.iter().all(|v| v.as_bool().is_some());
        for v in &items {
            v.decref();
        }
        result
    } else if val.is_native_tuple() {
        let tuple = unsafe { val.as_native_tuple_ref().unwrap() };
        let items = tuple.as_slice();
        !items.is_empty() && items.iter().all(|v| v.as_bool().is_some())
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// PureVM broadcast methods
// ---------------------------------------------------------------------------

impl PureVM {
    // =====================================================================
    // Synchronous VMFunc invocation
    // =====================================================================

    /// Call a VMFunc synchronously by running a sub-dispatch.
    /// Saves and restores the frame stack so the outer dispatch is unaffected.
    pub(crate) fn call_vmfunc_sync(&mut self, func_idx: u32, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        let slot = self
            .func_table
            .get(func_idx)
            .ok_or_else(|| VMError::RuntimeError("invalid function index in broadcast".into()))?;
        let callee_code = Arc::clone(&slot.code);
        let closure = slot.closure.clone();

        let mut new_frame = self.frame_pool.alloc_with_code(callee_code);
        new_frame.bind_args(args);
        new_frame.closure_scope = closure;

        let saved_stack = std::mem::take(&mut self.frame_stack);
        let result = self.dispatch(new_frame, host);
        self.frame_stack = saved_stack;
        result
    }

    /// Call a value (VMFunc or host-callable) synchronously.
    fn call_value_sync(&mut self, func: Value, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        if func.is_vmfunc() && !func.is_invalid() {
            self.call_vmfunc_sync(func.as_vmfunc_idx(), args, host)
        } else {
            host.call_function(func, args)
        }
    }

    // =====================================================================
    // Broadcast entry point
    // =====================================================================

    /// Main broadcast dispatch. Handles map, filter, deep recursion.
    pub(crate) fn apply_broadcast(
        &mut self,
        target: Value,
        operator: Value,
        operand: Option<Value>,
        is_filter: bool,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        // Scalar target: apply directly
        if !is_collection(target) {
            return self.broadcast_scalar(target, operator, operand, is_filter, host);
        }

        let is_tuple = target.is_native_tuple();

        if is_filter {
            self.broadcast_filter(target, operator, operand, is_tuple, host)
        } else {
            self.broadcast_map(target, operator, operand, is_tuple, host)
        }
    }

    /// Broadcast on a scalar target.
    fn broadcast_scalar(
        &mut self,
        target: Value,
        operator: Value,
        operand: Option<Value>,
        is_filter: bool,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        if is_filter {
            let cond = self.apply_single(target, operator, operand, host)?;
            if cond.is_truthy() {
                target.clone_refcount();
                Ok(Value::from_list(vec![target]))
            } else {
                Ok(Value::from_list(vec![]))
            }
        } else {
            self.apply_single(target, operator, operand, host)
        }
    }

    /// Broadcast map: apply operator to each element, recurse into nested lists.
    fn broadcast_map(
        &mut self,
        target: Value,
        operator: Value,
        operand: Option<Value>,
        is_tuple: bool,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        let items = extract_items(target)?;
        let mut result = Vec::with_capacity(items.len());

        for item in &items {
            if is_collection(*item) {
                // Deep broadcast: recurse into nested list/tuple
                let inner_tuple = item.is_native_tuple();
                let inner = self.broadcast_map(*item, operator, operand, inner_tuple, host)?;
                result.push(inner);
            } else {
                let val = self.apply_single(*item, operator, operand, host)?;
                result.push(val);
            }
        }

        // Decref extracted items (as_slice_cloned incremented them)
        for item in &items {
            item.decref();
        }

        Ok(wrap_result(result, is_tuple))
    }

    /// Broadcast filter: keep elements where condition is truthy.
    fn broadcast_filter(
        &mut self,
        target: Value,
        operator: Value,
        operand: Option<Value>,
        is_tuple: bool,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        // Boolean mask detection: operator is a list/tuple of bools
        if is_boolean_mask(operator) {
            let items = extract_items(target)?;
            let result = self.filter_by_mask(&items, operator)?;
            for item in &items {
                item.decref();
            }
            return Ok(wrap_result(result, is_tuple));
        }

        let items = extract_items(target)?;
        let mut result = Vec::new();

        for item in &items {
            let cond = self.apply_single(*item, operator, operand, host)?;
            if cond.is_truthy() {
                item.clone_refcount();
                result.push(*item);
            }
        }

        // Decref extracted items
        for item in &items {
            item.decref();
        }

        Ok(wrap_result(result, is_tuple))
    }

    /// Filter by boolean mask. Mask and items must have the same length.
    fn filter_by_mask(&self, items: &[Value], mask: Value) -> VMResult<Vec<Value>> {
        let mask_items = extract_items(mask)?;

        if items.len() != mask_items.len() {
            for m in &mask_items {
                m.decref();
            }
            return Err(VMError::ValueError(format!(
                "boolean mask length ({}) does not match data length ({})",
                mask_items.len(),
                items.len()
            )));
        }

        let mut result = Vec::new();
        for (item, mask_val) in items.iter().zip(mask_items.iter()) {
            let keep = mask_val
                .as_bool()
                .ok_or_else(|| VMError::TypeError("boolean mask must contain only booleans".into()))?;
            if keep {
                item.clone_refcount();
                result.push(*item);
            }
        }

        for m in &mask_items {
            m.decref();
        }

        Ok(result)
    }

    // =====================================================================
    // Apply single element
    // =====================================================================

    /// Apply operator (with optional operand) to a single element.
    fn apply_single(
        &mut self,
        element: Value,
        operator: Value,
        operand: Option<Value>,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        // String operator: binary/comparison op or builtin name
        if operator.is_native_str() {
            let op_name = unsafe { operator.as_native_str_ref().unwrap() };

            if let Some(operand) = operand {
                // Binary op: element OP operand
                if let Some(binop) = str_to_binary_op(op_name) {
                    return host.binary_op(binop, element, operand);
                }
                // Equality/inequality
                if op_name == "==" {
                    let eq = arith::eq_native(element, operand).unwrap_or(false);
                    return Ok(Value::from_bool(eq));
                }
                if op_name == "!=" {
                    let eq = arith::eq_native(element, operand).unwrap_or(true);
                    return Ok(Value::from_bool(!eq));
                }
                return Err(VMError::TypeError(format!("unknown broadcast operator '{op_name}'")));
            }

            // Unary: negate or builtin function call
            if op_name == "-" {
                return self.negate(element);
            }
            // Try as builtin function (abs, str, int, etc.)
            return host.call_function(operator, &[element]);
        }

        // VMFunc operator: call with element (+ operand if present)
        if operator.is_vmfunc() && !operator.is_invalid() {
            let idx = operator.as_vmfunc_idx();
            return if let Some(operand) = operand {
                self.call_vmfunc_sync(idx, &[element, operand], host)
            } else {
                self.call_vmfunc_sync(idx, &[element], host)
            };
        }

        // Fallback: host callable
        if let Some(operand) = operand {
            host.call_function(operator, &[element, operand])
        } else {
            host.call_function(operator, &[element])
        }
    }

    /// Negate a numeric value.
    fn negate(&self, val: Value) -> VMResult<Value> {
        if let Some(i) = val.as_int() {
            return Ok(Value::from_int(-i));
        }
        if let Some(f) = val.as_float() {
            return Ok(Value::from_float(-f));
        }
        if val.is_bigint() {
            let n = unsafe { val.as_bigint_ref().unwrap() };
            let neg = rug::Integer::from(-n);
            return Ok(Value::from_bigint_or_demote(neg));
        }
        Err(VMError::TypeError(format!("cannot negate {}", val.type_name())))
    }

    // =====================================================================
    // ND Map (~>)
    // =====================================================================

    /// Apply ND map: recursively apply func to leaf elements.
    /// For lists/tuples, recurse. For scalars, call func(scalar).
    pub(crate) fn nd_map_apply(&mut self, data: Value, func: Value, host: &dyn VmHost) -> VMResult<Value> {
        if is_collection(data) {
            let is_tuple = data.is_native_tuple();
            let items = extract_items(data)?;
            let mut result = Vec::with_capacity(items.len());

            for item in &items {
                let val = self.nd_map_apply(*item, func, host)?;
                result.push(val);
            }

            for item in &items {
                item.decref();
            }

            Ok(wrap_result(result, is_tuple))
        } else {
            // Leaf: call func(data)
            self.call_value_sync(func, &[data], host)
        }
    }

    // =====================================================================
    // ND Recursion (~~)
    // =====================================================================

    /// Broadcast ND recursion: apply ND recursion to each element of target.
    pub(crate) fn broadcast_nd_recursion(
        &mut self,
        target: Value,
        lambda: Value,
        host: &dyn VmHost,
    ) -> VMResult<Value> {
        if !is_collection(target) {
            // Scalar: single ND recursion call
            return self.nd_recursion_call(target, lambda, host);
        }

        let is_tuple = target.is_native_tuple();
        let items = extract_items(target)?;
        let mut result = Vec::with_capacity(items.len());

        for item in &items {
            let val = self.nd_recursion_call(*item, lambda, host)?;
            result.push(val);
        }

        for item in &items {
            item.decref();
        }

        Ok(wrap_result(result, is_tuple))
    }

    /// Execute ND recursion: call lambda(seed, recur) where recur is a sentinel
    /// that the Call opcode intercepts for recursive dispatch.
    pub(crate) fn nd_recursion_call(&mut self, seed: Value, lambda: Value, host: &dyn VmHost) -> VMResult<Value> {
        if !lambda.is_vmfunc() || lambda.is_invalid() {
            return Err(VMError::TypeError("ND recursion lambda must be a function".into()));
        }
        let lambda_idx = lambda.as_vmfunc_idx();

        // Depth guard
        if self.nd_lambda_stack.len() >= ND_MAX_DEPTH {
            return Err(VMError::RuntimeError("ND recursion depth limit exceeded".into()));
        }

        // Push lambda index so Call can find it when recur sentinel is called
        self.nd_lambda_stack.push(lambda_idx);
        let recur = Value::from_str(ND_RECUR_SENTINEL);

        let result = self.call_vmfunc_sync(lambda_idx, &[seed, recur], host);

        self.nd_lambda_stack.pop();
        result
    }

    /// Handle a call to the ND recur sentinel from within the dispatch loop.
    /// Called when the Call opcode detects __nd_recur__.
    pub(crate) fn handle_nd_recur_call(&mut self, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        let lambda_idx = self
            .nd_lambda_stack
            .last()
            .copied()
            .ok_or_else(|| VMError::RuntimeError("ND recur called outside ND context".into()))?;

        if args.is_empty() {
            return Err(VMError::TypeError("ND recur requires an argument".into()));
        }
        let seed = args[0];
        let lambda = Value::from_vmfunc(lambda_idx);

        self.nd_recursion_call(seed, lambda, host)
    }

    /// Handle a call to an ND declaration wrapper (~~lambda).
    pub(crate) fn handle_nd_decl_call(&mut self, lambda: Value, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        if args.is_empty() {
            return Err(VMError::TypeError(
                "ND declaration call requires a seed argument".into(),
            ));
        }
        self.nd_recursion_call(args[0], lambda, host)
    }

    /// Handle a call to an ND lift wrapper (~>func).
    pub(crate) fn handle_nd_lift_call(&mut self, func: Value, args: &[Value], host: &dyn VmHost) -> VMResult<Value> {
        if args.is_empty() {
            return Err(VMError::TypeError("ND lift call requires a data argument".into()));
        }
        self.nd_map_apply(args[0], func, host)
    }

    /// Check if a value is the ND recur sentinel.
    pub(crate) fn is_nd_recur_sentinel(val: Value) -> bool {
        if val.is_native_str() {
            let s = unsafe { val.as_native_str_ref().unwrap() };
            s == ND_RECUR_SENTINEL
        } else {
            false
        }
    }

    /// Check if a value is an ND wrapper tuple (decl or lift).
    /// Returns Some((tag, inner_value)) if it is.
    pub(crate) fn check_nd_wrapper(val: Value) -> Option<(&'static str, Value)> {
        if !val.is_native_tuple() {
            return None;
        }
        let tuple = unsafe { val.as_native_tuple_ref()? };
        let items = tuple.as_slice();
        if items.len() != 2 || !items[0].is_native_str() {
            return None;
        }
        let tag = unsafe { items[0].as_native_str_ref().unwrap() };
        match tag {
            ND_DECL_TAG => Some((ND_DECL_TAG, items[1])),
            ND_LIFT_TAG => Some((ND_LIFT_TAG, items[1])),
            _ => None,
        }
    }
}

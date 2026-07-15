//! Tests for the pure VM host (builtins, name resolution, returns).

use super::*;

#[test]
fn test_pure_host_globals() {
    let host = PureHost::with_builtins();
    assert!(host.has_global("True"));
    assert!(host.has_global("False"));
    assert!(host.has_global("None"));
    assert!(!host.has_global("x"));

    host.store_global("x", Value::from_int(42));
    assert!(host.has_global("x"));
    assert_eq!(host.lookup_global("x").unwrap(), Some(Value::from_int(42)));

    host.delete_global("x");
    assert!(!host.has_global("x"));
}

#[test]
fn test_binary_add_int() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Add, Value::from_int(2), Value::from_int(3))
        .unwrap();
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_binary_add_float() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Add, Value::from_float(1.5), Value::from_float(2.5))
        .unwrap();
    assert!((result.as_float().unwrap() - 4.0).abs() < 1e-10);
}

#[test]
fn test_binary_add_mixed() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Add, Value::from_int(1), Value::from_float(2.5))
        .unwrap();
    assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);
}

#[test]
fn test_binary_add_string() {
    let host = PureHost::new();
    let a = Value::from_str("hello");
    let b = Value::from_str(" world");
    let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("hello world"));
    a.decref();
    b.decref();
    result.decref();
}

#[test]
fn test_binary_sub() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Sub, Value::from_int(10), Value::from_int(3))
        .unwrap();
    assert_eq!(result.as_int(), Some(7));
}

#[test]
fn test_binary_mul() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Mul, Value::from_int(4), Value::from_int(5))
        .unwrap();
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_binary_mul_string_repeat() {
    let host = PureHost::new();
    let s = Value::from_str("ab");
    let result = host.binary_op(BinaryOp::Mul, s, Value::from_int(3)).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("ababab"));
    s.decref();
    result.decref();
}

#[test]
fn test_binary_div() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::TrueDiv, Value::from_int(7), Value::from_int(2))
        .unwrap();
    assert!((result.as_float().unwrap() - 3.5).abs() < 1e-10);
}

#[test]
fn test_binary_div_zero() {
    let host = PureHost::new();
    let result = host.binary_op(BinaryOp::TrueDiv, Value::from_int(1), Value::from_int(0));
    assert!(matches!(result, Err(VMError::ZeroDivisionError(_))));
}

#[test]
fn test_binary_floordiv() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::FloorDiv, Value::from_int(7), Value::from_int(2))
        .unwrap();
    assert_eq!(result.as_int(), Some(3));

    // Python semantics: -7 // 2 == -4 (floor division)
    let result = host
        .binary_op(BinaryOp::FloorDiv, Value::from_int(-7), Value::from_int(2))
        .unwrap();
    assert_eq!(result.as_int(), Some(-4));
}

#[test]
fn test_binary_mod() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Mod, Value::from_int(7), Value::from_int(3))
        .unwrap();
    assert_eq!(result.as_int(), Some(1));

    // Python semantics: -7 % 3 == 2
    let result = host
        .binary_op(BinaryOp::Mod, Value::from_int(-7), Value::from_int(3))
        .unwrap();
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_binary_pow() {
    let host = PureHost::new();
    let result = host
        .binary_op(BinaryOp::Pow, Value::from_int(2), Value::from_int(10))
        .unwrap();
    assert_eq!(result.as_int(), Some(1024));
}

#[test]
fn test_comparison() {
    let host = PureHost::new();
    let t = |op: BinaryOp, a: i64, b: i64| -> bool {
        host.binary_op(op, Value::from_int(a), Value::from_int(b))
            .unwrap()
            .as_bool()
            .unwrap()
    };
    assert!(t(BinaryOp::Lt, 1, 2));
    assert!(!t(BinaryOp::Lt, 2, 1));
    assert!(t(BinaryOp::Le, 1, 1));
    assert!(t(BinaryOp::Gt, 3, 2));
    assert!(t(BinaryOp::Ge, 2, 2));
}

#[test]
fn test_comparison_string() {
    let host = PureHost::new();
    let a = Value::from_str("abc");
    let b = Value::from_str("abd");
    let result = host.binary_op(BinaryOp::Lt, a, b).unwrap();
    assert_eq!(result.as_bool(), Some(true));
    a.decref();
    b.decref();
}

#[test]
fn test_bigint_add() {
    let host = PureHost::new();
    let a = Value::from_bigint(Integer::from(1_u64 << 50));
    let b = Value::from_int(1);
    let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
    assert!(result.is_bigint());
    let expected = Integer::from(1_u64 << 50) + Integer::from(1);
    assert_eq!(unsafe { result.as_bigint_ref() }, Some(&expected));
    a.decref();
    result.decref();
}

#[test]
fn test_contains_string() {
    let host = PureHost::new();
    let haystack = Value::from_str("hello world");
    let needle = Value::from_str("world");
    assert!(host.contains_op(needle, haystack).unwrap());
    haystack.decref();
    needle.decref();
}

#[test]
fn test_str_iter() {
    let host = PureHost::new();
    let s = Value::from_str("abc");
    let mut iter = host.get_iter(s).unwrap();
    let a = iter.next_value().unwrap().unwrap();
    assert_eq!(unsafe { a.as_native_str_ref() }, Some("a"));
    let b = iter.next_value().unwrap().unwrap();
    assert_eq!(unsafe { b.as_native_str_ref() }, Some("b"));
    let c = iter.next_value().unwrap().unwrap();
    assert_eq!(unsafe { c.as_native_str_ref() }, Some("c"));
    assert!(iter.next_value().unwrap().is_none());
    s.decref();
    a.decref();
    b.decref();
    c.decref();
}

#[test]
fn test_str_getattr() {
    let host = PureHost::new();
    let s = Value::from_str("hello");
    let upper = host.obj_getattr(s, "upper").unwrap();
    assert_eq!(unsafe { upper.as_native_str_ref() }, Some("HELLO"));
    s.decref();
    upper.decref();
}

#[test]
fn test_str_getitem() {
    let host = PureHost::new();
    let s = Value::from_str("hello");
    let ch = host.obj_getitem(s, Value::from_int(1)).unwrap();
    assert_eq!(unsafe { ch.as_native_str_ref() }, Some("e"));
    s.decref();
    ch.decref();
}

#[test]
fn test_builtin_abs() {
    let result = call_builtin("abs", &[Value::from_int(-42)]).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_builtin_len() {
    let s = Value::from_str("hello");
    let result = call_builtin("len", &[s]).unwrap();
    assert_eq!(result.as_int(), Some(5));
    s.decref();
}

#[test]
fn test_builtin_str() {
    let result = call_builtin("str", &[Value::from_int(42)]).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("42"));
    result.decref();
}

#[test]
fn test_builtin_int_from_str() {
    let s = Value::from_str("42");
    let result = call_builtin("int", &[s]).unwrap();
    assert_eq!(result.as_int(), Some(42));
    s.decref();
}

#[test]
fn test_builtin_float_from_str() {
    let s = Value::from_str("3.14");
    let result = call_builtin("float", &[s]).unwrap();
    assert!((result.as_float().unwrap() - 3.14).abs() < 1e-10);
    s.decref();
}

#[test]
fn test_builtin_bool() {
    assert_eq!(
        call_builtin("bool", &[Value::from_int(0)]).unwrap().as_bool(),
        Some(false)
    );
    assert_eq!(
        call_builtin("bool", &[Value::from_int(1)]).unwrap().as_bool(),
        Some(true)
    );
}

#[test]
fn test_builtin_type() {
    let result = call_builtin("type", &[Value::from_int(1)]).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("int"));
    result.decref();

    let s = Value::from_str("x");
    let result = call_builtin("type", &[s]).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("str"));
    s.decref();
    result.decref();
}

#[test]
fn test_builtin_min_max() {
    let result = call_builtin("min", &[Value::from_int(3), Value::from_int(1), Value::from_int(2)]).unwrap();
    assert_eq!(result.as_int(), Some(1));

    let result = call_builtin("max", &[Value::from_int(3), Value::from_int(1), Value::from_int(2)]).unwrap();
    assert_eq!(result.as_int(), Some(3));
}

// --- Collection host tests ---

#[test]
fn test_list_iter() {
    let host = PureHost::new();
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
    let mut iter = host.get_iter(list).unwrap();
    assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(1));
    assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(2));
    assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(3));
    assert!(iter.next_value().unwrap().is_none());
    list.decref();
}

#[test]
fn test_tuple_iter() {
    let host = PureHost::new();
    let tuple = Value::from_tuple(vec![Value::from_int(10), Value::from_int(20)]);
    let mut iter = host.get_iter(tuple).unwrap();
    assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(10));
    assert_eq!(iter.next_value().unwrap().unwrap(), Value::from_int(20));
    assert!(iter.next_value().unwrap().is_none());
    tuple.decref();
}

#[test]
fn test_dict_iter_keys() {
    let host = PureHost::new();
    let dict = Value::from_empty_dict();
    let d = unsafe { dict.as_native_dict_ref().unwrap() };
    d.set_item(crate::collections::ValueKey::Int(1), Value::from_int(10));
    d.set_item(crate::collections::ValueKey::Int(2), Value::from_int(20));
    let mut iter = host.get_iter(dict).unwrap();
    let k1 = iter.next_value().unwrap().unwrap();
    assert_eq!(k1.as_int(), Some(1));
    let k2 = iter.next_value().unwrap().unwrap();
    assert_eq!(k2.as_int(), Some(2));
    assert!(iter.next_value().unwrap().is_none());
    dict.decref();
}

#[test]
fn test_list_getitem_setitem() {
    let host = PureHost::new();
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    let v = host.obj_getitem(list, Value::from_int(0)).unwrap();
    assert_eq!(v, Value::from_int(1));
    host.obj_setitem(list, Value::from_int(0), Value::from_int(10)).unwrap();
    let v = host.obj_getitem(list, Value::from_int(0)).unwrap();
    assert_eq!(v, Value::from_int(10));
    list.decref();
}

#[test]
fn test_dict_getitem_setitem() {
    let host = PureHost::new();
    let dict = Value::from_empty_dict();
    host.obj_setitem(dict, Value::from_int(1), Value::from_int(10)).unwrap();
    let v = host.obj_getitem(dict, Value::from_int(1)).unwrap();
    assert_eq!(v, Value::from_int(10));
    dict.decref();
}

#[test]
fn test_contains_list() {
    let host = PureHost::new();
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
    assert!(host.contains_op(Value::from_int(2), list).unwrap());
    assert!(!host.contains_op(Value::from_int(5), list).unwrap());
    list.decref();
}

#[test]
fn test_contains_dict() {
    let host = PureHost::new();
    let dict = Value::from_empty_dict();
    let d = unsafe { dict.as_native_dict_ref().unwrap() };
    d.set_item(crate::collections::ValueKey::Int(1), Value::from_int(10));
    assert!(host.contains_op(Value::from_int(1), dict).unwrap());
    assert!(!host.contains_op(Value::from_int(2), dict).unwrap());
    dict.decref();
}

#[test]
fn test_list_concat() {
    let host = PureHost::new();
    let a = Value::from_list(vec![Value::from_int(1)]);
    let b = Value::from_list(vec![Value::from_int(2)]);
    let result = host.binary_op(BinaryOp::Add, a, b).unwrap();
    assert!(result.is_native_list());
    let list = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 2);
    a.decref();
    b.decref();
    result.decref();
}

#[test]
fn test_list_repeat() {
    let host = PureHost::new();
    let a = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    let result = host.binary_op(BinaryOp::Mul, a, Value::from_int(3)).unwrap();
    let list = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 6);
    a.decref();
    result.decref();
}

#[test]
fn test_builtin_len_collections() {
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    let result = call_builtin("len", &[list]).unwrap();
    assert_eq!(result.as_int(), Some(2));
    list.decref();

    let tuple = Value::from_tuple(vec![Value::from_int(1)]);
    let result = call_builtin("len", &[tuple]).unwrap();
    assert_eq!(result.as_int(), Some(1));
    tuple.decref();

    let dict = Value::from_empty_dict();
    let result = call_builtin("len", &[dict]).unwrap();
    assert_eq!(result.as_int(), Some(0));
    dict.decref();
}

#[test]
fn test_builtin_range() {
    let result = call_builtin("range", &[Value::from_int(5)]).unwrap();
    let list = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 5);
    assert_eq!(list.get(0).unwrap(), Value::from_int(0));
    assert_eq!(list.get(4).unwrap(), Value::from_int(4));
    result.decref();
}

#[test]
fn test_builtin_sorted() {
    let list = Value::from_list(vec![Value::from_int(3), Value::from_int(1), Value::from_int(2)]);
    let result = call_builtin("sorted", &[list]).unwrap();
    let sorted = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(sorted.get(0).unwrap(), Value::from_int(1));
    assert_eq!(sorted.get(2).unwrap(), Value::from_int(3));
    list.decref();
    result.decref();
}

#[test]
fn test_builtin_sum() {
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
    let result = call_builtin("sum", &[list]).unwrap();
    assert_eq!(result.as_int(), Some(6));
    list.decref();
}

#[test]
fn test_builtin_any_all() {
    let list = Value::from_list(vec![Value::from_int(0), Value::from_int(1)]);
    assert_eq!(call_builtin("any", &[list]).unwrap().as_bool(), Some(true));
    assert_eq!(call_builtin("all", &[list]).unwrap().as_bool(), Some(false));
    list.decref();

    let list2 = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    assert_eq!(call_builtin("all", &[list2]).unwrap().as_bool(), Some(true));
    list2.decref();
}

#[test]
fn test_builtin_enumerate() {
    let list = Value::from_list(vec![Value::from_str("a"), Value::from_str("b")]);
    let result = call_builtin("enumerate", &[list]).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 2);
    let first = r.get(0).unwrap();
    assert!(first.is_native_tuple());
    let t = unsafe { first.as_native_tuple_ref().unwrap() };
    assert_eq!(t.get(0).unwrap(), Value::from_int(0));
    first.decref();
    list.decref();
    result.decref();
}

#[test]
fn test_builtin_zip() {
    let a = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    let b = Value::from_list(vec![Value::from_str("a"), Value::from_str("b"), Value::from_str("c")]);
    let result = call_builtin("zip", &[a, b]).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 2); // min(2, 3)
    a.decref();
    b.decref();
    result.decref();
}

#[test]
fn test_builtin_type_collections() {
    let list = Value::from_list(vec![]);
    let result = call_builtin("type", &[list]).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("list"));
    list.decref();
    result.decref();

    let tuple = Value::from_tuple(vec![]);
    let result = call_builtin("type", &[tuple]).unwrap();
    assert_eq!(unsafe { result.as_native_str_ref() }, Some("tuple"));
    tuple.decref();
    result.decref();
}

#[test]
fn test_call_method_str_split() {
    let host = PureHost::new();
    let s = Value::from_str("a,b,c");
    let sep = Value::from_str(",");
    let result = host.call_method(s, "split", &[sep]).unwrap();
    assert!(result.is_native_list());
    let list = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(list.len(), 3);
    s.decref();
    sep.decref();
    result.decref();
}

#[test]
fn test_call_method_list_append() {
    let host = PureHost::new();
    let list = Value::from_list(vec![]);
    host.call_method(list, "append", &[Value::from_int(42)]).unwrap();
    let l = unsafe { list.as_native_list_ref().unwrap() };
    assert_eq!(l.len(), 1);
    list.decref();
}

// --- apply_slice unit tests ---

#[test]
fn test_apply_slice_list_basic() {
    let list = Value::from_list(vec![
        Value::from_int(10),
        Value::from_int(20),
        Value::from_int(30),
        Value::from_int(40),
    ]);
    let result = apply_slice(list, Value::from_int(1), Value::from_int(3), Value::NIL).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 2);
    assert_eq!(r.get(0).unwrap(), Value::from_int(20));
    assert_eq!(r.get(1).unwrap(), Value::from_int(30));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_list_negative() {
    let list = Value::from_list(vec![
        Value::from_int(1),
        Value::from_int(2),
        Value::from_int(3),
        Value::from_int(4),
    ]);
    // list[:-1] -> [1, 2, 3]
    let result = apply_slice(list, Value::NIL, Value::from_int(-1), Value::NIL).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 3);
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_list_open_end() {
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2), Value::from_int(3)]);
    // list[1:]
    let result = apply_slice(list, Value::from_int(1), Value::NIL, Value::NIL).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 2);
    assert_eq!(r.get(0).unwrap(), Value::from_int(2));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_string() {
    let s = Value::from_string("hello".to_string());
    let result = apply_slice(s, Value::from_int(1), Value::from_int(4), Value::NIL).unwrap();
    let r = unsafe { result.as_native_str_ref().unwrap() };
    assert_eq!(r, "ell");
    result.decref();
    s.decref();
}

#[test]
fn test_apply_slice_tuple() {
    let t = Value::from_tuple(vec![Value::from_int(10), Value::from_int(20), Value::from_int(30)]);
    // tuple[:2]
    let result = apply_slice(t, Value::NIL, Value::from_int(2), Value::NIL).unwrap();
    let r = unsafe { result.as_native_tuple_ref().unwrap() };
    assert_eq!(r.len(), 2);
    result.decref();
    t.decref();
}

#[test]
fn test_apply_slice_step_1_ok() {
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    // step=1 is allowed
    let result = apply_slice(list, Value::NIL, Value::NIL, Value::from_int(1)).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 2);
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_step_positive() {
    // [0,1,2,3,4][::2] -> [0, 2, 4]
    let list = Value::from_list(vec![
        Value::from_int(0),
        Value::from_int(1),
        Value::from_int(2),
        Value::from_int(3),
        Value::from_int(4),
    ]);
    let result = apply_slice(list, Value::NIL, Value::NIL, Value::from_int(2)).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 3);
    assert_eq!(r.get(0).unwrap(), Value::from_int(0));
    assert_eq!(r.get(1).unwrap(), Value::from_int(2));
    assert_eq!(r.get(2).unwrap(), Value::from_int(4));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_step_negative_reverse() {
    // [0,1,2,3,4][::-1] -> [4, 3, 2, 1, 0]
    let list = Value::from_list(vec![
        Value::from_int(0),
        Value::from_int(1),
        Value::from_int(2),
        Value::from_int(3),
        Value::from_int(4),
    ]);
    let result = apply_slice(list, Value::NIL, Value::NIL, Value::from_int(-1)).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 5);
    assert_eq!(r.get(0).unwrap(), Value::from_int(4));
    assert_eq!(r.get(4).unwrap(), Value::from_int(0));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_step_negative_skip() {
    // [0,1,2,3,4][::-2] -> [4, 2, 0]
    let list = Value::from_list(vec![
        Value::from_int(0),
        Value::from_int(1),
        Value::from_int(2),
        Value::from_int(3),
        Value::from_int(4),
    ]);
    let result = apply_slice(list, Value::NIL, Value::NIL, Value::from_int(-2)).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 3);
    assert_eq!(r.get(0).unwrap(), Value::from_int(4));
    assert_eq!(r.get(1).unwrap(), Value::from_int(2));
    assert_eq!(r.get(2).unwrap(), Value::from_int(0));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_step_with_bounds() {
    // [0,1,2,3,4,5,6,7,8,9][1:8:2] -> [1, 3, 5, 7]
    let list = Value::from_list((0..10).map(Value::from_int).collect());
    let result = apply_slice(list, Value::from_int(1), Value::from_int(8), Value::from_int(2)).unwrap();
    let r = unsafe { result.as_native_list_ref().unwrap() };
    assert_eq!(r.len(), 4);
    assert_eq!(r.get(0).unwrap(), Value::from_int(1));
    assert_eq!(r.get(1).unwrap(), Value::from_int(3));
    assert_eq!(r.get(2).unwrap(), Value::from_int(5));
    assert_eq!(r.get(3).unwrap(), Value::from_int(7));
    result.decref();
    list.decref();
}

#[test]
fn test_apply_slice_step_zero_rejected() {
    let list = Value::from_list(vec![Value::from_int(1)]);
    assert!(apply_slice(list, Value::NIL, Value::NIL, Value::from_int(0)).is_err());
    list.decref();
}

#[test]
fn test_apply_slice_string_reverse() {
    // "hello"[::-1] -> "olleh"
    let s = Value::from_string("hello".to_string());
    let result = apply_slice(s, Value::NIL, Value::NIL, Value::from_int(-1)).unwrap();
    let r = unsafe { result.as_native_str_ref().unwrap() };
    assert_eq!(r, "olleh");
    result.decref();
    s.decref();
}

#[test]
fn test_apply_slice_non_int_bound_rejected() {
    let list = Value::from_list(vec![Value::from_int(1), Value::from_int(2)]);
    let bad_start = Value::from_string("oops".to_string());
    assert!(apply_slice(list, bad_start, Value::NIL, Value::NIL).is_err());
    bad_start.decref();
    list.decref();
}

#[test]
fn test_apply_slice_non_sliceable_rejected() {
    let dict = Value::from_empty_dict();
    assert!(apply_slice(dict, Value::NIL, Value::NIL, Value::NIL).is_err());
    dict.decref();
}

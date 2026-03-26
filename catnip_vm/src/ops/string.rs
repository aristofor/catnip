// FILE: catnip_vm/src/ops/string.rs
//! Native string operations for NativeStr values.
//!
//! All functions operate on `&str` references borrowed from the NaN-boxed Value.
//! Results are returned as new `Value::from_string(...)`.

use crate::error::{VMError, VMResult};
use crate::value::Value;

// ---------------------------------------------------------------------------
// Binary operations
// ---------------------------------------------------------------------------

/// String concatenation: a ++ b (or a + b when both are strings).
#[inline]
pub fn str_concat(a: &str, b: &str) -> Value {
    let mut result = String::with_capacity(a.len() + b.len());
    result.push_str(a);
    result.push_str(b);
    Value::from_string(result)
}

/// String repeat: s * n.
#[inline]
pub fn str_repeat(s: &str, n: i64) -> Value {
    if n <= 0 {
        Value::from_string(String::new())
    } else {
        Value::from_string(s.repeat(n as usize))
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

#[inline]
pub fn str_eq(a: &str, b: &str) -> bool {
    a == b
}

#[inline]
pub fn str_lt(a: &str, b: &str) -> bool {
    a < b
}

#[inline]
pub fn str_le(a: &str, b: &str) -> bool {
    a <= b
}

#[inline]
pub fn str_gt(a: &str, b: &str) -> bool {
    a > b
}

#[inline]
pub fn str_ge(a: &str, b: &str) -> bool {
    a >= b
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// len(s) -- number of characters (not bytes).
#[inline]
pub fn str_len(s: &str) -> usize {
    s.chars().count()
}

// ---------------------------------------------------------------------------
// Indexing and slicing
// ---------------------------------------------------------------------------

/// s[i] -- character at index (negative indexing supported).
pub fn str_getitem(s: &str, index: i64) -> VMResult<Value> {
    let len = s.chars().count() as i64;
    let idx = if index < 0 { index + len } else { index };
    if idx < 0 || idx >= len {
        return Err(VMError::IndexError(format!(
            "string index out of range: {} (len {})",
            index, len
        )));
    }
    let ch = s.chars().nth(idx as usize).unwrap();
    Value::from_string(ch.to_string());
    Ok(Value::from_string(ch.to_string()))
}

/// s[start:end] -- slice with Python semantics.
pub fn str_slice(s: &str, start: Option<i64>, end: Option<i64>) -> Value {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;

    let resolve = |idx: i64| -> usize {
        let i = if idx < 0 { idx + len } else { idx };
        i.clamp(0, len) as usize
    };

    let start = resolve(start.unwrap_or(0));
    let end = resolve(end.unwrap_or(len));

    if start >= end {
        Value::from_string(String::new())
    } else {
        let result: String = chars[start..end].iter().collect();
        Value::from_string(result)
    }
}

// ---------------------------------------------------------------------------
// String methods
// ---------------------------------------------------------------------------

#[inline]
pub fn str_upper(s: &str) -> Value {
    Value::from_string(s.to_uppercase())
}

#[inline]
pub fn str_lower(s: &str) -> Value {
    Value::from_string(s.to_lowercase())
}

#[inline]
pub fn str_strip(s: &str) -> Value {
    Value::from_string(s.trim().to_string())
}

#[inline]
pub fn str_lstrip(s: &str) -> Value {
    Value::from_string(s.trim_start().to_string())
}

#[inline]
pub fn str_rstrip(s: &str) -> Value {
    Value::from_string(s.trim_end().to_string())
}

/// split(sep) -- split string by separator, returns a list of NativeStr.
/// For now returns a Vec<Value> (will become NativeList in Phase 2).
pub fn str_split(s: &str, sep: &str) -> Vec<Value> {
    s.split(sep).map(Value::from_str).collect()
}

/// join(sep, parts) -- join strings with separator.
pub fn str_join(sep: &str, parts: &[Value]) -> VMResult<Value> {
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push_str(sep);
        }
        let s = unsafe {
            part.as_native_str_ref()
                .ok_or_else(|| VMError::TypeError("join expects all string elements".to_string()))?
        };
        result.push_str(s);
    }
    Ok(Value::from_string(result))
}

/// replace(old, new) -- replace all occurrences.
#[inline]
pub fn str_replace(s: &str, old: &str, new: &str) -> Value {
    Value::from_string(s.replace(old, new))
}

/// contains(substr) -- substring test.
#[inline]
pub fn str_contains(s: &str, substr: &str) -> bool {
    s.contains(substr)
}

/// startswith(prefix).
#[inline]
pub fn str_startswith(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// endswith(suffix).
#[inline]
pub fn str_endswith(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// find(substr) -- returns index or -1.
#[inline]
pub fn str_find(s: &str, substr: &str) -> i64 {
    // Character index, not byte index
    match s.find(substr) {
        Some(byte_idx) => s[..byte_idx].chars().count() as i64,
        None => -1,
    }
}

/// count(substr) -- count non-overlapping occurrences.
#[inline]
pub fn str_count(s: &str, substr: &str) -> i64 {
    s.matches(substr).count() as i64
}

/// title() -- titlecase.
pub fn str_title(s: &str) -> Value {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            if capitalize_next {
                result.extend(ch.to_uppercase());
                capitalize_next = false;
            } else {
                result.extend(ch.to_lowercase());
            }
        } else {
            result.push(ch);
            capitalize_next = true;
        }
    }
    Value::from_string(result)
}

/// capitalize() -- first char upper, rest lower.
pub fn str_capitalize(s: &str) -> Value {
    let mut chars = s.chars();
    let result = match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut r = String::with_capacity(s.len());
            r.extend(first.to_uppercase());
            for ch in chars {
                r.extend(ch.to_lowercase());
            }
            r
        }
    };
    Value::from_string(result)
}

/// isdigit().
#[inline]
pub fn str_isdigit(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// isalpha().
#[inline]
pub fn str_isalpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphabetic())
}

/// isalnum().
#[inline]
pub fn str_isalnum(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric())
}

// ---------------------------------------------------------------------------
// F-string building
// ---------------------------------------------------------------------------

/// Concatenate multiple string segments into one (f-string building).
pub fn str_build(parts: &[&str]) -> Value {
    let total_len: usize = parts.iter().map(|s| s.len()).sum();
    let mut result = String::with_capacity(total_len);
    for part in parts {
        result.push_str(part);
    }
    Value::from_string(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: Value) -> String {
        let r = unsafe { v.as_native_str_ref().unwrap().to_string() };
        v.decref();
        r
    }

    #[test]
    fn test_concat() {
        assert_eq!(s(str_concat("hello", " world")), "hello world");
        assert_eq!(s(str_concat("", "x")), "x");
        assert_eq!(s(str_concat("x", "")), "x");
    }

    #[test]
    fn test_repeat() {
        assert_eq!(s(str_repeat("ab", 3)), "ababab");
        assert_eq!(s(str_repeat("x", 0)), "");
        assert_eq!(s(str_repeat("x", -1)), "");
    }

    #[test]
    fn test_comparison() {
        assert!(str_eq("abc", "abc"));
        assert!(!str_eq("abc", "def"));
        assert!(str_lt("abc", "abd"));
        assert!(str_le("abc", "abc"));
        assert!(str_gt("b", "a"));
        assert!(str_ge("a", "a"));
    }

    #[test]
    fn test_len() {
        assert_eq!(str_len("hello"), 5);
        assert_eq!(str_len(""), 0);
        assert_eq!(str_len("cafe\u{0301}"), 5); // e + combining accent = 2 chars
    }

    #[test]
    fn test_getitem() {
        assert_eq!(s(str_getitem("hello", 0).unwrap()), "h");
        assert_eq!(s(str_getitem("hello", -1).unwrap()), "o");
        assert!(str_getitem("hello", 5).is_err());
        assert!(str_getitem("hello", -6).is_err());
    }

    #[test]
    fn test_slice() {
        assert_eq!(s(str_slice("hello", Some(1), Some(4))), "ell");
        assert_eq!(s(str_slice("hello", None, Some(3))), "hel");
        assert_eq!(s(str_slice("hello", Some(2), None)), "llo");
        assert_eq!(s(str_slice("hello", Some(-2), None)), "lo");
        assert_eq!(s(str_slice("hello", Some(3), Some(1))), ""); // empty when start >= end
    }

    #[test]
    fn test_upper_lower() {
        assert_eq!(s(str_upper("hello")), "HELLO");
        assert_eq!(s(str_lower("HELLO")), "hello");
    }

    #[test]
    fn test_strip() {
        assert_eq!(s(str_strip("  hello  ")), "hello");
        assert_eq!(s(str_lstrip("  hello  ")), "hello  ");
        assert_eq!(s(str_rstrip("  hello  ")), "  hello");
    }

    #[test]
    fn test_split() {
        let parts = str_split("a,b,c", ",");
        assert_eq!(parts.len(), 3);
        assert_eq!(unsafe { parts[0].as_native_str_ref().unwrap() }, "a");
        assert_eq!(unsafe { parts[1].as_native_str_ref().unwrap() }, "b");
        assert_eq!(unsafe { parts[2].as_native_str_ref().unwrap() }, "c");
        for p in &parts {
            p.decref();
        }
    }

    #[test]
    fn test_join() {
        let parts = vec![Value::from_str("a"), Value::from_str("b"), Value::from_str("c")];
        let result = str_join(", ", &parts).unwrap();
        assert_eq!(s(result), "a, b, c");
        for p in &parts {
            p.decref();
        }
    }

    #[test]
    fn test_replace() {
        assert_eq!(s(str_replace("hello world", "world", "rust")), "hello rust");
    }

    #[test]
    fn test_contains() {
        assert!(str_contains("hello world", "world"));
        assert!(!str_contains("hello world", "xyz"));
    }

    #[test]
    fn test_startswith_endswith() {
        assert!(str_startswith("hello", "hel"));
        assert!(!str_startswith("hello", "lo"));
        assert!(str_endswith("hello", "llo"));
        assert!(!str_endswith("hello", "hel"));
    }

    #[test]
    fn test_find() {
        assert_eq!(str_find("hello world", "world"), 6);
        assert_eq!(str_find("hello world", "xyz"), -1);
    }

    #[test]
    fn test_count() {
        assert_eq!(str_count("abcabcabc", "abc"), 3);
        assert_eq!(str_count("hello", "xyz"), 0);
    }

    #[test]
    fn test_title() {
        assert_eq!(s(str_title("hello world")), "Hello World");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(s(str_capitalize("hello")), "Hello");
        assert_eq!(s(str_capitalize("HELLO")), "Hello");
    }

    #[test]
    fn test_isdigit() {
        assert!(str_isdigit("123"));
        assert!(!str_isdigit("12a"));
        assert!(!str_isdigit(""));
    }

    #[test]
    fn test_build_fstring() {
        assert_eq!(s(str_build(&["hello", " ", "world"])), "hello world");
        assert_eq!(s(str_build(&[])), "");
    }
}

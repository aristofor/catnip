// FILE: catnip_core/src/jit/memo_cache.rs
/// Memoization cache for JIT-compiled recursive functions.
///
/// Phase 4.3: Cache results of recursive calls to avoid redundant computation.
/// Target: fibonacci-like functions with overlapping subproblems (O(2^n) → O(n)).
///
/// Implementation: thread_local HashMap for fast lookup without locking.
/// Key: (func_id_hash, arg) - func_id ensures cache isolation per function
/// Value: computed result (NaN-boxed i64)
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Thread-local cache for memoization.
    /// Uses RefCell for interior mutability (single-threaded access).
    static MEMO_CACHE: RefCell<HashMap<(u64, i64), i64>> = RefCell::new(HashMap::new());
}

/// Lookup a memoized result.
///
/// Returns the cached result if present, or -1 if not found.
/// Note: -1 is also used for guard failures, but in memoization context
/// it simply means "cache miss, compute the value".
///
/// # Arguments
/// * `func_id` - Hash of function identifier (for cache isolation)
/// * `arg` - Function argument (unboxed integer)
///
/// # Returns
/// * Cached result (NaN-boxed i64) if found
/// * -1 if not in cache
#[no_mangle]
pub extern "C" fn memo_lookup(func_id: u64, arg: i64) -> i64 {
    MEMO_CACHE.with(|cache| cache.borrow().get(&(func_id, arg)).copied().unwrap_or(-1))
}

/// Store a computed result in the memoization cache.
///
/// # Arguments
/// * `func_id` - Hash of function identifier (for cache isolation)
/// * `arg` - Function argument (unboxed integer)
/// * `result` - Computed result (NaN-boxed i64)
#[no_mangle]
pub extern "C" fn memo_store(func_id: u64, arg: i64, result: i64) {
    MEMO_CACHE.with(|cache| {
        cache.borrow_mut().insert((func_id, arg), result);
    });
}

/// Clear the memoization cache (for testing).
#[cfg(test)]
pub fn clear_memo_cache() {
    MEMO_CACHE.with(|cache| {
        cache.borrow_mut().clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memo_cache_basic() {
        clear_memo_cache();

        let func_id = 12345;
        let arg = 5;

        // Cache miss
        assert_eq!(memo_lookup(func_id, arg), -1);

        // Store value
        let result = 42;
        memo_store(func_id, arg, result);

        // Cache hit
        assert_eq!(memo_lookup(func_id, arg), result);
    }

    #[test]
    fn test_memo_cache_isolation() {
        clear_memo_cache();

        let func1 = 111;
        let func2 = 222;
        let arg = 5;

        memo_store(func1, arg, 100);
        memo_store(func2, arg, 200);

        // Different functions have different cached values
        assert_eq!(memo_lookup(func1, arg), 100);
        assert_eq!(memo_lookup(func2, arg), 200);
    }
}

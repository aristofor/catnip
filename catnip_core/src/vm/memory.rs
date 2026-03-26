// FILE: catnip_core/src/vm/memory.rs
//! RSS memory guard for the VM dispatch loop.
//!
//! On Linux, reads `/proc/self/statm` to get the resident set size.
//! On other platforms, returns `None` (guard is a no-op).

/// Get the current process RSS in bytes.
///
/// Reads `/proc/self/statm` field 1 (RSS in pages) and multiplies by page size.
/// Returns `None` on non-Linux platforms or if the read fails.
#[cfg(target_os = "linux")]
pub fn get_rss_bytes() -> Option<u64> {
    let data = std::fs::read_to_string("/proc/self/statm").ok()?;
    let rss_pages: u64 = data.split_whitespace().nth(1)?.parse().ok()?;
    Some(rss_pages * 4096)
}

#[cfg(not(target_os = "linux"))]
pub fn get_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn test_get_rss_bytes_returns_nonzero() {
        let rss = get_rss_bytes();
        assert!(rss.is_some(), "should read RSS on Linux");
        assert!(rss.unwrap() > 0, "RSS should be > 0");
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn test_get_rss_bytes_returns_none() {
        assert!(get_rss_bytes().is_none());
    }
}

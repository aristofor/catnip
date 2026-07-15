//! Capture the process stdout (fd 1) while running an evaluation.
//!
//! The native `io` stdlib plugin that backs the PureVM writes `print`/`write`
//! output straight to the process file descriptors. Because the MCP server
//! speaks JSON-RPC over `stdio`, that output would corrupt the protocol stream
//! and is otherwise lost to the caller. We redirect fd 1 into a scratch file for
//! the duration of the closure, and point fd 0 (stdin) at `/dev/null` so
//! `input()` sees EOF instead of consuming protocol bytes. stderr (fd 2) is left
//! untouched: it is not the protocol channel.

/// Run `f` with the process stdout captured, returning its result and whatever
/// was written to stdout. On non-unix targets (or if the fd dance fails) the
/// closure runs unchanged and the captured string is empty.
#[cfg(unix)]
pub fn with_captured_stdio<R>(f: impl FnOnce() -> R) -> (R, String) {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::unix::io::AsRawFd;

    // A scratch file (unlinked immediately) avoids the pipe-buffer deadlock a
    // self-read pipe would hit if the program writes more than the pipe capacity
    // with no concurrent reader.
    let mut scratch = match scratch_file() {
        Some(f) => f,
        None => return (f(), String::new()),
    };
    let devnull = std::fs::File::open("/dev/null").ok();

    // Flush std's buffered stdout before swapping the fd out from under it.
    let _ = std::io::stdout().flush();

    // SAFETY: dup on STDOUT/STDIN_FILENO, which are always valid process fds.
    let saved_out = unsafe { libc::dup(libc::STDOUT_FILENO) };
    let saved_in = unsafe { libc::dup(libc::STDIN_FILENO) };
    if saved_out < 0 || saved_in < 0 {
        // SAFETY: close only the fds that were successfully dup'd (>= 0).
        unsafe {
            if saved_out >= 0 {
                libc::close(saved_out);
            }
            if saved_in >= 0 {
                libc::close(saved_in);
            }
        }
        return (f(), String::new());
    }

    // Restores the original fds on every path (including unwind), so the swap is
    // scoped strictly to `f`.
    struct RestoreFds {
        saved_out: i32,
        saved_in: i32,
    }
    impl Drop for RestoreFds {
        fn drop(&mut self) {
            // SAFETY: saved_* are live fds we dup'd above; dup2 closes the target
            // first, then we release the backups.
            unsafe {
                libc::dup2(self.saved_out, libc::STDOUT_FILENO);
                libc::dup2(self.saved_in, libc::STDIN_FILENO);
                libc::close(self.saved_out);
                libc::close(self.saved_in);
            }
        }
    }
    let _restore = RestoreFds { saved_out, saved_in };

    // SAFETY: point fd 1 at the scratch file and fd 0 at /dev/null for `f`. Both
    // source fds are owned by live File handles kept alive across the call.
    unsafe {
        libc::dup2(scratch.as_raw_fd(), libc::STDOUT_FILENO);
        if let Some(dn) = &devnull {
            libc::dup2(dn.as_raw_fd(), libc::STDIN_FILENO);
        }
    }

    let result = f();

    // Flush anything std buffered onto the (redirected) fd 1, then `_restore`'s
    // Drop puts the real fds back.
    let _ = std::io::stdout().flush();
    drop(_restore);

    let mut captured = String::new();
    let _ = scratch.seek(SeekFrom::Start(0));
    let _ = scratch.read_to_string(&mut captured);
    (result, captured)
}

#[cfg(not(unix))]
pub fn with_captured_stdio<R>(f: impl FnOnce() -> R) -> (R, String) {
    (f(), String::new())
}

/// Open a unique scratch file and unlink it immediately: the fd keeps the file
/// alive, but it never appears in listings and is reclaimed when the fd closes.
#[cfg(unix)]
fn scratch_file() -> Option<std::fs::File> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("catnip-mcp-{}-{}.out", std::process::id(), n));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .ok()?;
    let _ = std::fs::remove_file(&path);
    Some(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::io::FromRawFd;

    // Write straight to fd 1, like the native io plugin (and unlike print!, which
    // libtest intercepts at the std level, bypassing the fd).
    fn write_fd1(s: &str) {
        let mut f = unsafe { std::fs::File::from_raw_fd(1) };
        let _ = f.write_all(s.as_bytes());
        let _ = f.flush();
        std::mem::forget(f); // keep fd 1 open
    }

    #[test]
    fn captures_then_restores_fd1() {
        let (r, out) = with_captured_stdio(|| {
            write_fd1("hello-capture");
            42
        });
        assert_eq!(r, 42);
        assert_eq!(out, "hello-capture");

        // A second capture works -> fd 1 was restored to a writable state.
        let (_, out2) = with_captured_stdio(|| write_fd1("second"));
        assert_eq!(out2, "second");
    }
}

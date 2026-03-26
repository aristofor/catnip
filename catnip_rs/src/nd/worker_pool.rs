// FILE: catnip_rs/src/nd/worker_pool.rs
//! Pool of persistent Rust worker processes for ND process mode.
//!
//! Each worker is a `catnip worker` subprocess communicating
//! via length-prefixed bincode over stdin/stdout pipes.

use catnip_core::freeze::FrozenValue;
use catnip_core::freeze::worker::{WorkerCommand, WorkerResult, read_message, write_message};
use std::io::{BufReader, BufWriter};
use std::process::{Child, Command, Stdio};

/// A single worker process with buffered I/O handles.
struct Worker {
    child: Child,
    writer: BufWriter<std::process::ChildStdin>,
    reader: BufReader<std::process::ChildStdout>,
}

impl Worker {
    /// Spawn a new worker process and verify it's alive with Ping/Pong.
    fn spawn(bin_path: &str) -> Result<Self, String> {
        let mut child = Command::new(bin_path)
            .arg("worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("failed to spawn worker: {e}"))?;

        let stdin = child.stdin.take().ok_or("worker: no stdin")?;
        let stdout = child.stdout.take().ok_or("worker: no stdout")?;
        let mut w = Worker {
            child,
            writer: BufWriter::new(stdin),
            reader: BufReader::new(stdout),
        };

        // Verify readiness
        w.send(&WorkerCommand::Ping)?;
        match w.recv()? {
            WorkerResult::Pong => Ok(w),
            other => Err(format!("worker: expected Pong, got {:?}", other)),
        }
    }

    fn send(&mut self, cmd: &WorkerCommand) -> Result<(), String> {
        write_message(&mut self.writer, cmd).map_err(|e| format!("worker send: {e}"))
    }

    fn recv(&mut self) -> Result<WorkerResult, String> {
        read_message(&mut self.reader)
            .map_err(|e| format!("worker recv: {e}"))?
            .ok_or_else(|| "worker: unexpected EOF".to_string())
    }

    fn shutdown(mut self) {
        let _ = write_message(&mut self.writer, &WorkerCommand::Shutdown);
        let _ = self.child.wait();
    }
}

/// Pool of persistent worker processes.
pub struct WorkerPool {
    workers: Vec<Option<Worker>>,
    bin_path: String,
    next: usize,
}

impl WorkerPool {
    /// Create a new pool with `n_workers` processes.
    pub fn new(n_workers: usize) -> Result<Self, String> {
        let bin_path = resolve_worker_bin()?;
        let mut workers = Vec::with_capacity(n_workers);
        for _ in 0..n_workers {
            workers.push(Some(Worker::spawn(&bin_path)?));
        }
        Ok(WorkerPool {
            workers,
            bin_path,
            next: 0,
        })
    }

    /// Submit a batch of tasks and collect results in order.
    /// Each task shares the same encoded_ir, captures, and param_names, but has a different seed.
    pub fn submit_batch(
        &mut self,
        encoded_ir: &[u8],
        captures: &[(String, FrozenValue)],
        param_names: &[String],
        seeds: &[FrozenValue],
    ) -> Result<Vec<FrozenValue>, String> {
        let n_workers = self.workers.len();
        if n_workers == 0 {
            return Err("worker pool: no workers".to_string());
        }

        // Assign tasks to workers round-robin
        // Track which worker owns which task for ordered collection
        let mut assignments: Vec<usize> = Vec::with_capacity(seeds.len());

        for (i, seed) in seeds.iter().enumerate() {
            let worker_idx = (self.next + i) % n_workers;
            assignments.push(worker_idx);

            let cmd = WorkerCommand::Execute {
                encoded_ir: encoded_ir.to_vec(),
                captures: captures.to_vec(),
                param_names: param_names.to_vec(),
                seed: seed.clone(),
            };

            if let Err(e) = self.send_to(worker_idx, &cmd) {
                // Worker crashed, try to respawn and retry
                self.respawn(worker_idx)?;
                self.send_to(worker_idx, &cmd)
                    .map_err(|e2| format!("worker {worker_idx} retry failed: {e}, {e2}"))?;
            }
        }

        // Advance round-robin cursor
        self.next = (self.next + seeds.len()) % n_workers;

        // Collect results in submission order
        let mut results = Vec::with_capacity(seeds.len());
        for &worker_idx in &assignments {
            match self.recv_from(worker_idx) {
                Ok(WorkerResult::Ok(val)) => results.push(val),
                Ok(WorkerResult::Err(msg)) => return Err(format!("worker error: {msg}")),
                Ok(WorkerResult::Pong) => return Err("worker: unexpected Pong".to_string()),
                Err(e) => {
                    // Worker crashed mid-batch, try to respawn for future use
                    let _ = self.respawn(worker_idx);
                    return Err(format!("worker {worker_idx} crashed: {e}"));
                }
            }
        }

        Ok(results)
    }

    fn send_to(&mut self, idx: usize, cmd: &WorkerCommand) -> Result<(), String> {
        self.workers[idx]
            .as_mut()
            .ok_or_else(|| format!("worker {idx}: not running"))?
            .send(cmd)
    }

    fn recv_from(&mut self, idx: usize) -> Result<WorkerResult, String> {
        self.workers[idx]
            .as_mut()
            .ok_or_else(|| format!("worker {idx}: not running"))?
            .recv()
    }

    fn respawn(&mut self, idx: usize) -> Result<(), String> {
        // Drop old worker (kills process)
        if let Some(w) = self.workers[idx].take() {
            w.shutdown();
        }
        self.workers[idx] = Some(Worker::spawn(&self.bin_path)?);
        Ok(())
    }

    /// Number of workers in the pool.
    pub fn size(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for slot in &mut self.workers {
            if let Some(w) = slot.take() {
                w.shutdown();
            }
        }
    }
}

/// Resolve the `catnip` binary path.
/// Order: CATNIP_WORKER_BIN env var → adjacent to current exe → PATH lookup.
fn resolve_worker_bin() -> Result<String, String> {
    // 1. Explicit env var
    if let Ok(path) = std::env::var("CATNIP_WORKER_BIN") {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // 2. Adjacent to current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("catnip");
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // 3. PATH lookup (cross-platform: let the OS resolve via a no-op invocation)
    if let Ok(output) = Command::new("catnip").arg("--version").output() {
        if output.status.success() {
            return Ok("catnip".to_string());
        }
    }

    Err("cannot find catnip binary (set CATNIP_WORKER_BIN or add to PATH)".to_string())
}

/// Default pool size based on available parallelism.
pub fn default_pool_size() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

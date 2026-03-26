// FILE: catnip_vm/src/vm/debug.rs
//! Debug hook trait and types for PureVM breakpoint/stepping support.

use std::collections::HashSet;

use crate::compiler::code_object::CodeObject;

use super::frame::PureFrame;

/// Command returned by the debug hook to control VM execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugCommand {
    Continue,
    StepInto,
    StepOver,
    StepOut,
}

/// Snapshot of VM state at a pause point.
///
/// The VM provides raw byte offsets and locals. Source mapping
/// (byte -> line/col, snippets) is the caller's responsibility.
pub struct PauseInfo {
    /// Source byte offset of the current instruction.
    pub start_byte: u32,
    /// Local variables as (name, repr) pairs, sorted by name.
    pub locals: Vec<(String, String)>,
    /// Current call depth (number of frames on the stack).
    pub depth: usize,
}

/// Trait for receiving debug pause notifications from PureVM.
///
/// Implementations control VM execution by returning a `DebugCommand`.
pub trait DebugHook {
    /// Called when the VM pauses (breakpoint hit or step completed).
    /// Must return the next command to execute.
    fn on_pause(&mut self, info: &PauseInfo) -> DebugCommand;
}

/// Debug state extracted from PureVM so it can be borrowed independently.
pub(crate) struct DebugState {
    pub hook: Option<Box<dyn DebugHook>>,
    /// Breakpoint line numbers (1-indexed). Wrapped in Arc<Mutex> so external
    /// code (MCP handlers) can add/remove breakpoints while the VM is paused.
    pub breakpoint_lines: std::sync::Arc<std::sync::Mutex<HashSet<usize>>>,
    /// Source bytes for byte-offset -> line conversion.
    pub source_bytes: Option<Vec<u8>>,
    pub stepping: bool,
    pub step_mode: DebugCommand,
    pub step_depth: usize,
    pub last_pause_byte: u32,
}

impl DebugState {
    pub fn new() -> Self {
        Self {
            hook: None,
            breakpoint_lines: std::sync::Arc::new(std::sync::Mutex::new(HashSet::new())),
            source_bytes: None,
            stepping: false,
            step_mode: DebugCommand::Continue,
            step_depth: 0,
            last_pause_byte: u32::MAX,
        }
    }

    /// Returns true if any debug checking is needed in the dispatch loop.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.hook.is_some()
    }

    /// Convert a source byte offset to a 1-indexed line number.
    fn byte_to_line(&self, byte_offset: u32) -> usize {
        match &self.source_bytes {
            Some(bytes) => {
                let offset = byte_offset as usize;
                let count = bytes[..offset.min(bytes.len())].iter().filter(|&&b| b == b'\n').count();
                count + 1
            }
            None => 0,
        }
    }

    /// Check if this instruction is on a breakpoint line.
    pub fn is_breakpoint(&self, code: &CodeObject, instr_idx: usize) -> bool {
        let bp = self.breakpoint_lines.lock().unwrap();
        if bp.is_empty() || self.source_bytes.is_none() {
            return false;
        }
        let src_byte = code.line_table.get(instr_idx).copied().unwrap_or(u32::MAX);
        if src_byte == u32::MAX {
            return false;
        }
        let line = self.byte_to_line(src_byte);
        bp.contains(&line)
    }

    /// Check if this instruction should trigger a step pause.
    /// Called every instruction when stepping is active.
    pub fn check_step(&mut self, frame: &PureFrame, code: &CodeObject, instr_idx: usize, frame_depth: usize) {
        let start_byte = code.line_table.get(instr_idx).copied().unwrap_or(0);
        if start_byte == self.last_pause_byte {
            return;
        }

        let should_pause = match self.step_mode {
            DebugCommand::StepInto => true,
            DebugCommand::StepOver => frame_depth <= self.step_depth,
            DebugCommand::StepOut => frame_depth < self.step_depth,
            DebugCommand::Continue => false,
        };

        if should_pause {
            self.pause(frame, code, instr_idx, frame_depth, false);
        }
    }

    /// Pause execution (breakpoint hit or step completed).
    pub fn pause(
        &mut self,
        frame: &PureFrame,
        code: &CodeObject,
        instr_idx: usize,
        frame_depth: usize,
        _is_breakpoint: bool,
    ) {
        let start_byte = code.line_table.get(instr_idx).copied().unwrap_or(0);

        let hook = match &mut self.hook {
            Some(h) => h,
            None => return,
        };

        let mut locals = Vec::new();
        for (i, name) in code.varnames.iter().enumerate() {
            let val = frame.get_local(i);
            if !val.is_invalid() {
                locals.push((name.clone(), val.repr_string()));
            }
        }
        locals.sort_by(|a, b| a.0.cmp(&b.0));

        let info = PauseInfo {
            start_byte,
            locals,
            depth: frame_depth,
        };

        let cmd = hook.on_pause(&info);
        self.last_pause_byte = start_byte;

        match cmd {
            DebugCommand::Continue => {
                self.stepping = false;
            }
            DebugCommand::StepInto | DebugCommand::StepOver | DebugCommand::StepOut => {
                self.stepping = true;
                self.step_mode = cmd;
                self.step_depth = frame_depth;
            }
        }
    }
}

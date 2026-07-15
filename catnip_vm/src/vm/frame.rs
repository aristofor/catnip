// FILE: catnip_vm/src/vm/frame.rs
//! Pure Rust VM frame -- no PyO3 dependency.
//!
//! Stripped-down version of catnip_rs::vm::frame::Frame.
//! No py_scope, no super_proxy, no Python kwargs in bind_args.

use crate::Value;
use crate::compiler::code_object::CodeObject;
use crate::vm::closure::PureClosureScope;
use catnip_core::exception::{Handler, PendingUnwind};
use std::sync::Arc;

const STACK_INIT_CAPACITY: usize = 32;

/// VM execution frame (pure Rust, no PyO3).
pub struct PureFrame {
    /// Operand stack
    pub stack: Vec<Value>,
    /// Local variable slots
    pub locals: Vec<Value>,
    /// Instruction pointer
    pub ip: usize,
    /// Code object being executed
    pub code: Option<Arc<CodeObject>>,
    /// Block stack for scope isolation: (slot_start, saved_values)
    pub block_stack: Vec<(usize, Vec<Value>)>,
    /// Pending match bindings from MatchPatternVM (slot, value) pairs
    pub match_bindings: Option<Vec<(usize, Value)>>,
    /// If true, return value is discarded (used for init post-constructor)
    pub discard_return: bool,
    /// Closure scope for free variable resolution (set by MakeFunction/Call).
    pub closure_scope: Option<PureClosureScope>,
    /// The callee value this frame is executing (a runtime `TAG_CLOSURE`, or a
    /// template `TAG_VMFUNC`/`NIL`). The frame owns one strong ref for its whole
    /// lifetime so a runtime closure's slot stays alive while its body runs --
    /// letrec self-recursion resolves through the slot's *weak* self-ref, which
    /// must upgrade during the call even when the callee had no surviving binding
    /// (e.g. `mk()(5)`). Released at frame teardown (`free`/`reset`); a template
    /// or NIL decref is a no-op.
    pub callee: Value,
    /// Exception handler stack (try/except/finally)
    pub handler_stack: Vec<Handler>,
    /// Active exception stack for CheckExcMatch/LoadException.
    /// Vec (not Option) to support save/restore across nested except handlers.
    pub active_exception_stack: Vec<catnip_core::exception::ExceptionInfo>,
    /// Pending unwind state (saved signal during finally execution)
    pub pending_unwind: Option<PendingUnwind>,
}

impl PureFrame {
    /// Create an empty frame.
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(STACK_INIT_CAPACITY),
            locals: Vec::new(),
            ip: 0,
            code: None,
            block_stack: Vec::new(),
            match_bindings: None,
            discard_return: false,
            closure_scope: None,
            callee: Value::NIL,
            handler_stack: Vec::new(),
            active_exception_stack: Vec::new(),
            pending_unwind: None,
        }
    }

    /// Create a frame for executing a CodeObject.
    pub fn with_code(code: Arc<CodeObject>) -> Self {
        let nlocals = code.nlocals;
        let fill = if cfg!(debug_assertions) {
            Value::INVALID
        } else {
            Value::NIL
        };
        let mut locals = Vec::with_capacity(nlocals);
        locals.resize(nlocals, fill);
        Self {
            stack: Vec::with_capacity(STACK_INIT_CAPACITY),
            locals,
            ip: 0,
            code: Some(code),
            block_stack: Vec::new(),
            match_bindings: None,
            discard_return: false,
            closure_scope: None,
            callee: Value::NIL,
            handler_stack: Vec::new(),
            active_exception_stack: Vec::new(),
            pending_unwind: None,
        }
    }

    /// Reset frame for pool reuse.
    pub fn reset(&mut self) {
        self.stack.clear();
        self.locals.clear();
        self.ip = 0;
        self.code = None;
        for (_slot_start, saved) in self.block_stack.drain(..) {
            for val in saved {
                val.decref();
            }
        }
        self.match_bindings = None;
        self.discard_return = false;
        self.closure_scope = None;
        // `free` already released and NIL'd the callee on the pooled path; this
        // decref is a no-op there and correct for any standalone reset.
        self.callee.decref();
        self.callee = Value::NIL;
        self.handler_stack.clear();
        self.active_exception_stack.clear();
        self.pending_unwind = None;
    }

    // --- Stack operations ---

    #[inline]
    pub fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    #[inline]
    pub fn pop(&mut self) -> Value {
        if cfg!(debug_assertions) {
            self.stack.pop().expect("VM stack underflow")
        } else {
            self.stack.pop().unwrap_or(Value::NIL)
        }
    }

    #[inline]
    pub fn peek(&self) -> Value {
        if cfg!(debug_assertions) {
            *self.stack.last().expect("VM stack underflow (peek)")
        } else {
            *self.stack.last().unwrap_or(&Value::NIL)
        }
    }

    // --- Local variable operations ---

    #[inline]
    pub fn set_local(&mut self, slot: usize, value: Value) {
        if slot < self.locals.len() {
            self.locals[slot] = value;
        } else {
            debug_assert!(
                false,
                "set_local: slot {slot} out of bounds (len={})",
                self.locals.len()
            );
        }
    }

    #[inline]
    pub fn get_local(&self, slot: usize) -> Value {
        if slot < self.locals.len() {
            self.locals[slot]
        } else if cfg!(debug_assertions) {
            panic!("get_local: slot {} out of bounds (nlocals={})", slot, self.locals.len())
        } else {
            Value::NIL
        }
    }

    /// Bind positional arguments to local slots (pure Rust, no kwargs).
    ///
    /// Consumes the caller's `args`: bound slots take ownership of the refs, the
    /// excess is collected into the variadic slot (which also takes ownership),
    /// and any remaining arg is released here. Mirrors the PyO3 VM's `bind_args`
    /// (`catnip_rs`) so both runtimes agree on arity, defaults, and variadic
    /// collection.
    pub fn bind_args(&mut self, args: &[Value]) {
        let code = match &self.code {
            Some(c) => c,
            None => return,
        };

        let nparams = code.nargs;
        let vararg_idx = code.vararg_idx;

        if vararg_idx >= 0 {
            let vararg_idx = vararg_idx as usize;
            // Fixed params before the variadic slot take their args by move.
            let nfixed = args.len().min(vararg_idx);
            self.locals[..nfixed].copy_from_slice(&args[..nfixed]);
            // The excess becomes the variadic list; `from_list` takes ownership
            // of those refs, so they are transferred, not leaked.
            let rest = if args.len() > vararg_idx {
                Value::from_list(args[vararg_idx..].to_vec())
            } else {
                Value::from_list(Vec::new())
            };
            self.locals[vararg_idx] = rest;

            Self::fill_defaults(&mut self.locals, &code.defaults, nfixed, vararg_idx);
        } else {
            let n = args.len().min(nparams);
            self.locals[..n].copy_from_slice(&args[..n]);
            // Excess positional args (a malformed, non-variadic call) are
            // consumed but unbound: release them instead of leaking.
            for v in &args[n..] {
                v.decref();
            }

            Self::fill_defaults(&mut self.locals, &code.defaults, n, nparams);
        }
    }

    /// Fill unbound parameter slots in `[nbound, end)` with their defaults.
    ///
    /// Defaults live in the CodeObject's constant pool, so a default that is a
    /// heap value must be incref'd: the slot will be decref'd at frame teardown
    /// like any other local, and without the incref that would over-release the
    /// shared constant.
    fn fill_defaults(locals: &mut [Value], defaults: &[Value], nbound: usize, end: usize) {
        let ndefaults = defaults.len();
        if ndefaults == 0 {
            return;
        }
        let default_start = end.saturating_sub(ndefaults);
        // index arithmetic across two arrays (locals slot vs defaults[i - default_start])
        #[allow(clippy::needless_range_loop)]
        for i in nbound.max(default_start)..end {
            if !locals[i].is_nil() && !locals[i].is_invalid() {
                continue;
            }
            let default_idx = i - default_start;
            if default_idx < ndefaults {
                let val = defaults[default_idx];
                val.clone_refcount();
                locals[i] = val;
            }
        }
    }

    // --- Block stack operations ---

    /// Save locals from slot_start onwards and push to block stack.
    /// Each saved value gets an independent refcount so the snapshot can be
    /// released independently of the current locals on pop / truncate / clear.
    pub fn push_block(&mut self, slot_start: usize) {
        let mut saved: Vec<Value> = self.locals[slot_start..].to_vec();
        for val in &mut saved {
            val.clone_refcount();
        }
        self.block_stack.push((slot_start, saved));
    }

    /// Restore locals from top of block stack.
    ///
    /// Each snapshot entry holds an independent refcount (taken at push_block),
    /// so every overwritten current local and every NILed block-local slot is
    /// decref'd before the snapshot value is transferred.
    pub fn pop_block(&mut self) {
        if let Some((slot_start, saved)) = self.block_stack.pop() {
            let saved_len = saved.len();
            for (i, val) in saved.into_iter().enumerate() {
                if slot_start + i < self.locals.len() {
                    let old = self.locals[slot_start + i];
                    old.decref();
                    self.locals[slot_start + i] = val;
                }
            }
            for i in (slot_start + saved_len)..self.locals.len() {
                let old = self.locals[i];
                old.decref();
                self.locals[i] = Value::NIL;
            }
        }
    }
}

impl Default for PureFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PureFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.code.as_ref().map(|c| c.name.as_str()).unwrap_or("<no code>");
        write!(
            f,
            "<PureFrame {} ip={} stack_depth={}>",
            name,
            self.ip,
            self.stack.len()
        )
    }
}

/// Frame pool for reducing allocation overhead (pure Rust).
pub struct PureFramePool {
    frames: Vec<PureFrame>,
    max_size: usize,
}

impl PureFramePool {
    pub fn new(max_size: usize) -> Self {
        Self {
            frames: Vec::with_capacity(max_size),
            max_size,
        }
    }

    /// Get a frame initialized with code (from pool or fresh).
    pub fn alloc_with_code(&mut self, code: Arc<CodeObject>) -> PureFrame {
        if let Some(mut frame) = self.frames.pop() {
            let nlocals = code.nlocals;
            frame.locals.clear();
            let fill = if cfg!(debug_assertions) {
                Value::INVALID
            } else {
                Value::NIL
            };
            frame.locals.resize(nlocals, fill);
            frame.code = Some(code);
            frame.ip = 0;
            frame.handler_stack.clear();
            frame.active_exception_stack.clear();
            frame.pending_unwind = None;
            frame
        } else {
            PureFrame::with_code(code)
        }
    }

    /// Return a frame to the pool (decref heap values first).
    pub fn free(&mut self, mut frame: PureFrame) {
        // Decref heap values (BigInt, NativeStr, collections)
        for &val in &frame.stack {
            val.decref();
        }
        for &val in &frame.locals {
            val.decref();
        }
        // The frame owned one strong ref to its callee; release it and NIL it so
        // the pooled reset() does not double-decref.
        frame.callee.decref();
        frame.callee = Value::NIL;
        // block_stack snapshots hold independent refcounts (taken at push_block).
        // Release them unconditionally: reset() (which also drains) runs only on
        // the pooled path, so a frame freed with a non-empty block_stack while the
        // pool is full would otherwise leak its snapshot refs.
        for (_slot_start, saved) in frame.block_stack.drain(..) {
            for val in saved {
                val.decref();
            }
        }
        // Pending match bindings own independent refs (BindMatch clones into
        // slots); release them here for the same pool-full reason.
        if let Some(bindings) = frame.match_bindings.take() {
            for (_slot, val) in bindings {
                val.decref();
            }
        }
        if self.frames.len() < self.max_size {
            frame.reset();
            self.frames.push(frame);
        }
    }
}

impl Default for PureFramePool {
    fn default() -> Self {
        Self::new(16)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let mut f = PureFrame::new();
        f.push(Value::from_int(42));
        f.push(Value::from_int(7));
        assert_eq!(f.stack.len(), 2);
        assert_eq!(f.pop().as_int(), Some(7));
        assert_eq!(f.pop().as_int(), Some(42));
        assert_eq!(f.stack.len(), 0);
    }

    #[test]
    fn test_peek() {
        let mut f = PureFrame::new();
        f.push(Value::from_int(99));
        assert_eq!(f.peek().as_int(), Some(99));
        assert_eq!(f.stack.len(), 1); // peek doesn't consume
    }

    #[test]
    fn test_locals() {
        let code = CodeObject {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nlocals: 3,
            varnames: vec!["a".into(), "b".into(), "c".into()],
            slotmap: [("a".into(), 0), ("b".into(), 1), ("c".into(), 2)].into(),
            nargs: 2,
            defaults: vec![],
            name: "test".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            union_checks: vec![],
            composite_checks: vec![],
            generic_checks: vec![],
            encoded_ir: None,
        };
        let code = Arc::new(code);
        let mut f = PureFrame::with_code(code);

        f.set_local(0, Value::from_int(10));
        f.set_local(1, Value::from_int(20));
        assert_eq!(f.get_local(0).as_int(), Some(10));
        assert_eq!(f.get_local(1).as_int(), Some(20));
    }

    #[test]
    fn test_bind_args() {
        let code = Arc::new(CodeObject {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nlocals: 3,
            varnames: vec!["x".into(), "y".into(), "z".into()],
            slotmap: [("x".into(), 0), ("y".into(), 1), ("z".into(), 2)].into(),
            nargs: 3,
            defaults: vec![Value::from_int(99)], // z defaults to 99
            name: "f".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            union_checks: vec![],
            composite_checks: vec![],
            generic_checks: vec![],
            encoded_ir: None,
        });
        let mut f = PureFrame::with_code(code);
        f.bind_args(&[Value::from_int(1), Value::from_int(2)]);
        assert_eq!(f.get_local(0).as_int(), Some(1));
        assert_eq!(f.get_local(1).as_int(), Some(2));
        assert_eq!(f.get_local(2).as_int(), Some(99)); // default
    }

    #[test]
    fn test_block_push_pop_restores() {
        let code = Arc::new(CodeObject {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nlocals: 4,
            varnames: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            slotmap: Default::default(),
            nargs: 0,
            defaults: vec![],
            name: "test".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            union_checks: vec![],
            composite_checks: vec![],
            generic_checks: vec![],
            encoded_ir: None,
        });
        let mut f = PureFrame::with_code(code);

        // Set initial values
        f.set_local(0, Value::from_int(10));
        f.set_local(1, Value::from_int(20));
        f.set_local(2, Value::from_int(30));
        f.set_local(3, Value::from_int(40));

        // Push block saving from slot 2 onwards
        f.push_block(2);

        // Modify slots 2-3 inside block
        f.set_local(2, Value::from_int(300));
        f.set_local(3, Value::from_int(400));
        assert_eq!(f.get_local(2).as_int(), Some(300));
        assert_eq!(f.get_local(3).as_int(), Some(400));

        // Pop block: slots 2-3 restored, slots 0-1 unchanged
        f.pop_block();
        assert_eq!(f.get_local(0).as_int(), Some(10));
        assert_eq!(f.get_local(1).as_int(), Some(20));
        assert_eq!(f.get_local(2).as_int(), Some(30));
        assert_eq!(f.get_local(3).as_int(), Some(40));
    }

    #[test]
    fn test_frame_pool() {
        let mut pool = PureFramePool::new(4);
        let code = Arc::new(CodeObject {
            instructions: vec![],
            constants: vec![],
            names: vec![],
            nlocals: 2,
            varnames: vec![],
            slotmap: Default::default(),
            nargs: 0,
            defaults: vec![],
            name: "pooled".into(),
            freevars: vec![],
            vararg_idx: -1,
            is_pure: false,
            complexity: 0,
            line_table: vec![],
            patterns: vec![],
            union_checks: vec![],
            composite_checks: vec![],
            generic_checks: vec![],
            encoded_ir: None,
        });

        let mut f = pool.alloc_with_code(Arc::clone(&code));
        f.push(Value::from_int(1));
        f.set_local(0, Value::from_int(42));
        pool.free(f);

        // Re-allocate: should reuse pooled frame with clean state
        let f2 = pool.alloc_with_code(code);
        assert_eq!(f2.stack.len(), 0);
        assert_eq!(f2.ip, 0);
        assert_eq!(f2.locals.len(), 2);
    }

    #[test]
    fn test_reset_clears_all() {
        let mut f = PureFrame::new();
        f.push(Value::from_int(1));
        f.ip = 42;
        f.discard_return = true;
        f.match_bindings = Some(vec![(0, Value::from_int(1))]);
        f.reset();
        assert!(f.stack.is_empty());
        assert!(f.locals.is_empty());
        assert_eq!(f.ip, 0);
        assert!(f.code.is_none());
        assert!(f.block_stack.is_empty());
        assert!(f.match_bindings.is_none());
        assert!(!f.discard_return);
    }
}

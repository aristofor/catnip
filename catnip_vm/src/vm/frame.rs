// FILE: catnip_vm/src/vm/frame.rs
//! Pure Rust VM frame -- no PyO3 dependency.
//!
//! Stripped-down version of catnip_rs::vm::frame::Frame.
//! No py_scope, no super_proxy, no Python kwargs in bind_args.

use crate::Value;
use crate::compiler::code_object::CodeObject;
use crate::vm::closure::PureClosureScope;
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
        }
    }

    /// Reset frame for pool reuse.
    pub fn reset(&mut self) {
        self.stack.clear();
        self.locals.clear();
        self.ip = 0;
        self.code = None;
        self.block_stack.clear();
        self.match_bindings = None;
        self.discard_return = false;
        self.closure_scope = None;
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
    pub fn bind_args(&mut self, args: &[Value]) {
        let code = match &self.code {
            Some(c) => c,
            None => return,
        };

        let nparams = code.nargs;
        let n = args.len().min(nparams);
        self.locals[..n].copy_from_slice(&args[..n]);

        // Fill defaults for unbound params
        let ndefaults = code.defaults.len();
        if ndefaults > 0 {
            let default_start = nparams.saturating_sub(ndefaults);
            for i in n.max(default_start)..nparams {
                if !self.locals[i].is_nil() && !self.locals[i].is_invalid() {
                    continue;
                }
                let default_idx = i - default_start;
                if default_idx < ndefaults {
                    self.locals[i] = code.defaults[default_idx];
                }
            }
        }
    }

    // --- Block stack operations ---

    /// Save locals from slot_start onwards and push to block stack.
    pub fn push_block(&mut self, slot_start: usize) {
        let saved: Vec<Value> = self.locals[slot_start..].to_vec();
        self.block_stack.push((slot_start, saved));
    }

    /// Restore locals from top of block stack.
    pub fn pop_block(&mut self) {
        if let Some((slot_start, saved)) = self.block_stack.pop() {
            let saved_len = saved.len();
            for (i, val) in saved.into_iter().enumerate() {
                if slot_start + i < self.locals.len() {
                    self.locals[slot_start + i] = val;
                }
            }
            for i in (slot_start + saved_len)..self.locals.len() {
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

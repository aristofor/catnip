// FILE: catnip_repl/src/context.rs
//! RustContext - Execution context with no Python dependency
//!
//! Replaces the Python Context for the standalone REPL.

use std::collections::HashMap;

/// Value type in the context
#[derive(Debug, Clone)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    // Dict, Function: ajoutables si le completer/hints doit les introspecter
}

impl Value {
    /// Convert to string for display
    pub fn to_display_string(&self) -> String {
        match self {
            Value::None => "None".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                // Format floats nicely
                if f.fract() == 0.0 && f.abs() < 1e10 {
                    format!("{:.1}", f)
                } else {
                    f.to_string()
                }
            }
            Value::String(s) => format!("\"{}\"", s),
            Value::List(items) => {
                let items_str: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", items_str.join(", "))
            }
        }
    }

    /// Convert to string for f-string interpolation (no quotes for strings)
    pub fn to_string_value(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            other => other.to_display_string(),
        }
    }

    /// Check if value is None
    pub fn is_none(&self) -> bool {
        matches!(self, Value::None)
    }

    /// Get type name for error messages
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::None => "None",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::String(_) => "str",
            Value::List(_) => "list",
        }
    }
}

/// Builtin function signature
pub type BuiltinFn = fn(&[Value]) -> Result<Value, String>;

/// RustContext - Pure Rust execution context
pub struct RustContext {
    /// Global variables
    globals: HashMap<String, Value>,

    /// Builtin functions
    builtins: HashMap<String, BuiltinFn>,

    /// Statistics
    pub stats: ContextStats,
}

/// Execution statistics
#[derive(Debug, Default)]
pub struct ContextStats {
    pub parse_time_us: u64,
    pub exec_time_us: u64,
    pub total_evaluations: usize,
    pub jit_compilations: usize,
}

impl RustContext {
    /// Create new context with builtins
    pub fn new() -> Self {
        let mut ctx = Self {
            globals: HashMap::new(),
            builtins: HashMap::new(),
            stats: ContextStats::default(),
        };

        // Register builtins
        ctx.register_builtins();

        ctx
    }

    /// Register all builtin functions
    fn register_builtins(&mut self) {
        self.builtins.insert("print".to_string(), builtin_print);
        self.builtins.insert("len".to_string(), builtin_len);
        self.builtins.insert("type".to_string(), builtin_type);
        self.builtins.insert("str".to_string(), builtin_str);
        self.builtins.insert("int".to_string(), builtin_int);
        self.builtins.insert("float".to_string(), builtin_float);
        self.builtins.insert("abs".to_string(), builtin_abs);
        self.builtins.insert("range".to_string(), builtin_range);
        // vars() et dir() sont gérés comme cas spéciaux dans executor.rs
    }

    /// Get all global variable names (for vars/dir builtins)
    pub fn get_global_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.globals.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get variable from globals
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.globals.get(name)
    }

    /// Set variable in globals
    pub fn set(&mut self, name: String, value: Value) {
        self.globals.insert(name, value);
    }

    /// Get builtin function
    pub fn get_builtin(&self, name: &str) -> Option<BuiltinFn> {
        self.builtins.get(name).copied()
    }
}

impl Default for RustContext {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Builtin Functions - Phase 2 minimal set
// ============================================================================

fn builtin_print(args: &[Value]) -> Result<Value, String> {
    let output = args
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_display_string(),
        })
        .collect::<Vec<_>>()
        .join(" ");

    println!("{}", output);
    Ok(Value::None)
}

fn builtin_len(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("len() takes exactly 1 argument ({} given)", args.len()));
    }

    match &args[0] {
        Value::List(items) => Ok(Value::Int(items.len() as i64)),
        Value::String(s) => Ok(Value::Int(s.len() as i64)),
        _ => Err(format!("object of type '{}' has no len()", args[0].type_name())),
    }
}

fn builtin_type(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("type() takes exactly 1 argument ({} given)", args.len()));
    }

    let type_name = match &args[0] {
        Value::None => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::String(_) => "str",
        Value::List(_) => "list",
    };

    Ok(Value::String(type_name.to_string()))
}

fn builtin_str(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("str() takes exactly 1 argument ({} given)", args.len()));
    }

    Ok(Value::String(args[0].to_display_string()))
}

fn builtin_int(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() || args.len() > 1 {
        return Err(format!("int() takes exactly 1 argument ({} given)", args.len()));
    }

    match &args[0] {
        Value::Int(i) => Ok(Value::Int(*i)),
        Value::Float(f) => Ok(Value::Int(*f as i64)),
        Value::String(s) => s
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| format!("invalid literal for int(): '{}'", s)),
        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
        Value::None => Err("int() argument must be a string or a number, not 'NoneType'".to_string()),
        Value::List(_) => Err("int() argument must be a string or a number, not 'list'".to_string()),
    }
}

fn builtin_float(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() || args.len() > 1 {
        return Err(format!("float() takes exactly 1 argument ({} given)", args.len()));
    }

    match &args[0] {
        Value::Float(f) => Ok(Value::Float(*f)),
        Value::Int(i) => Ok(Value::Float(*i as f64)),
        Value::String(s) => s
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|_| format!("could not convert string to float: '{}'", s)),
        Value::Bool(b) => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
        Value::None => Err("float() argument must be a string or a number, not 'NoneType'".to_string()),
        Value::List(_) => Err("float() argument must be a string or a number, not 'list'".to_string()),
    }
}

fn builtin_abs(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("abs() takes exactly 1 argument ({} given)", args.len()));
    }

    match &args[0] {
        Value::Int(i) => Ok(Value::Int(i.abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        _ => Err("bad operand type for abs()".to_string()),
    }
}

fn builtin_range(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() || args.len() > 3 {
        return Err(format!("range() takes 1 to 3 arguments ({} given)", args.len()));
    }

    let (start, stop, step) = match args.len() {
        1 => {
            // range(stop)
            let stop = match &args[0] {
                Value::Int(n) => *n,
                _ => return Err("range() argument must be an integer".to_string()),
            };
            (0, stop, 1)
        }
        2 => {
            // range(start, stop)
            let start = match &args[0] {
                Value::Int(n) => *n,
                _ => return Err("range() arguments must be integers".to_string()),
            };
            let stop = match &args[1] {
                Value::Int(n) => *n,
                _ => return Err("range() arguments must be integers".to_string()),
            };
            (start, stop, 1)
        }
        3 => {
            // range(start, stop, step)
            let start = match &args[0] {
                Value::Int(n) => *n,
                _ => return Err("range() arguments must be integers".to_string()),
            };
            let stop = match &args[1] {
                Value::Int(n) => *n,
                _ => return Err("range() arguments must be integers".to_string()),
            };
            let step = match &args[2] {
                Value::Int(n) => *n,
                _ => return Err("range() arguments must be integers".to_string()),
            };
            if step == 0 {
                return Err("range() step argument must not be zero".to_string());
            }
            (start, stop, step)
        }
        _ => unreachable!(),
    };

    let mut items = Vec::new();
    if step > 0 {
        let mut i = start;
        while i < stop {
            items.push(Value::Int(i));
            i += step;
        }
    } else {
        let mut i = start;
        while i > stop {
            items.push(Value::Int(i));
            i += step;
        }
    }

    Ok(Value::List(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = RustContext::new();
        assert!(ctx.get_builtin("print").is_some());
        assert!(ctx.get_builtin("len").is_some());
    }

    #[test]
    fn test_builtin_abs() {
        let result = builtin_abs(&[Value::Int(-42)]).unwrap();
        assert!(matches!(result, Value::Int(42)));
    }

    #[test]
    fn test_builtin_str() {
        let result = builtin_str(&[Value::Int(42)]).unwrap();
        assert!(matches!(result, Value::String(s) if s == "42"));
    }
}

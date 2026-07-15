// FILE: catnip_core/src/vm/opcode.rs
//! VM OpCode enumeration - SOURCE OF TRUTH
//!
//! This file defines the VM bytecode opcodes. Python bindings are generated from here.
//! Run `python catnip_rs/gen_opcodes.py` to regenerate Python files.

#![allow(dead_code)]

/// VMOpCode enumeration.
///
/// Layout: shared zone (1..=SHARED_MAX) has identical values to IROpCode,
/// followed by VM-only zone (SHARED_MAX+1..=MAX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum VMOpCode {
    // === Shared zone (1..=31) - same values as IROpCode ===

    // -- Arithmetic (1-8) --
    Add = 1,
    Sub = 2,
    Mul = 3,
    FloorDiv = 4,
    Mod = 5,
    Pow = 6,
    Neg = 7,
    Pos = 8,

    // -- Comparison (9-14) --
    Eq = 9,
    Ne = 10,
    Lt = 11,
    Le = 12,
    Gt = 13,
    Ge = 14,

    // -- Unary logic (15) --
    Not = 15,

    // -- Bitwise (16-21) --
    BAnd = 16,
    BOr = 17,
    BXor = 18,
    BNot = 19,
    LShift = 20,
    RShift = 21,

    // -- Access (22-25) --
    GetAttr = 22,
    SetAttr = 23,
    GetItem = 24,
    SetItem = 25,

    // -- Broadcasting & ND (26-29) --
    Broadcast = 26,
    NdRecursion = 27,
    NdMap = 28,
    NdEmptyTopos = 29,

    // -- Meta (30-31) --
    Nop = 30,
    Breakpoint = 31,

    // === VM-only zone (32..=MAX) ===

    // -- Operators (extends shared) --
    Div = 32,
    In = 33,
    NotIn = 34,
    Is = 35,
    IsNot = 36,

    // -- Conversion --
    ToBool = 37,
    TypeOf = 38,

    // -- Load/Store --
    LoadConst = 39,
    LoadLocal = 40,
    StoreLocal = 41,
    LoadScope = 42,
    StoreScope = 43,
    LoadGlobal = 44,

    // -- Stack --
    PopTop = 45,
    DupTop = 46,
    RotTwo = 47,

    // -- Jumps --
    Jump = 48,
    JumpIfFalse = 49,
    JumpIfTrue = 50,
    JumpIfFalseOrPop = 51,
    JumpIfTrueOrPop = 52,
    JumpIfNone = 53,
    JumpIfNotNoneOrPop = 54,

    // -- Iteration --
    GetIter = 55,
    ForIter = 56,
    ForRangeInt = 57,
    ForRangeStep = 58,

    // -- Functions --
    Call = 59,
    CallKw = 60,
    CallMethod = 61,
    TailCall = 62,
    Return = 63,
    MakeFunction = 64,

    // -- Collections --
    BuildList = 65,
    BuildTuple = 66,
    BuildSet = 67,
    BuildDict = 68,
    BuildSlice = 69,

    // -- String formatting --
    FormatValue = 70,
    BuildString = 71,

    // -- Blocks --
    PushBlock = 72,
    PopBlock = 73,
    Break = 74,
    Continue = 75,

    // -- Match --
    MatchPattern = 76,
    MatchPatternVM = 77,
    MatchAssignPatternVM = 78,
    BindMatch = 79,
    MatchFail = 80,

    // -- Unpack --
    UnpackSequence = 81,
    UnpackEx = 82,

    // -- Definitions --
    MakeStruct = 83,
    MakeTrait = 84,
    MakeEnum = 85,

    // -- Control --
    Halt = 86,
    Exit = 87,

    // -- Intrinsics --
    Globals = 88,
    Locals = 89,

    // -- Exception handling --
    SetupExcept = 90,
    SetupFinally = 91,
    PopHandler = 92,
    Raise = 93,
    CheckExcMatch = 94,
    LoadException = 95,
    ResumeUnwind = 96,
    ClearException = 97,

    /// Tagged union (ADT) definition. Reads a constant tuple `(name,
    /// type_params, variants)` from `arg` and registers the union namespace
    /// in globals. Variants are materialized as struct types (with payload)
    /// or enum singletons (nullary).
    MakeUnion = 98,

    /// Letrec group patch: pops `value` then `target`; if `target` is a
    /// VM function, inserts `value` under name `names[arg]` into its
    /// closure. No-op otherwise (e.g. sibling defined in a branch not
    /// taken yet). Pushes nothing.
    PatchClosure = 99,

    /// Typed-parameter boundary check (TH2-B step 0b). `arg` is a primitive
    /// type code (`type_code::*`). Pops the top value, enforces the declared
    /// type via the numeric tower (`int`/`bool` widen to `float`, `bool`
    /// widens to `int`): an exact or widenable value is coerced to the declared
    /// type and pushed back; anything else raises a `TypeError`. Emitted at a
    /// function prologue so an annotated param *is* its declared type before any
    /// specialized opcode reads it.
    CheckType = 100,

    /// Typed arithmetic (TH4 canal A): same result as the polymorphic op but
    /// emitted only when both operands are a proven-`int` / proven-`float` runtime
    /// fact, so the type dispatch (and struct-overload lookup) is skipped.
    /// The `*Int` variants keep the polymorphic op's integer overflow semantics
    /// (promote to bigint); the `*Float` variants operate directly on floats.
    /// Each falls back to the generic numeric op if an operand is unexpectedly
    /// off-type (defensive; the compiler only emits these on proven types). The
    /// main payoff is feeding the JIT pre-typed traces without a runtime type
    /// guard. True division (`/`) always yields a float, so only `DivFloat`
    /// exists.
    AddInt = 101,
    AddFloat = 102,
    SubInt = 103,
    SubFloat = 104,
    MulInt = 105,
    MulFloat = 106,
    DivFloat = 107,

    /// Nominal-type boundary check (enforcement nominal). `arg` indexes the
    /// `names` table for the declared type name (struct/enum/union). Pops the
    /// top value and checks membership with subtyping: a struct whose type name,
    /// MRO, or implemented traits include the name; a tagged-union variant
    /// (qualified `Name.Variant`); or an enum symbol whose enum name matches.
    /// Pushes the value unchanged on success (no coercion), raises `TypeError`
    /// otherwise. An unknown name at runtime is a no-op (the annotation is inert,
    /// like a composite). Emitted at a function prologue for a param annotated
    /// with a nominal type.
    CheckNominal = 108,

    /// Type-union boundary check (`int | str`, `Point | None`). `arg` indexes
    /// the `union_checks` table for a pre-classified list of member checks
    /// (primitive codes and/or nominal names). Pops the top value and accepts it
    /// if it belongs to *any* member: a primitive member matches by the numeric
    /// tower (a `bool` is a member of `int`, an `int`/`bool` of `float`) with no
    /// coercion (a union can't coerce toward one member), a nominal member by the
    /// same subtyping rule as `CheckNominal`. Pushes the value unchanged on
    /// success, raises `TypeError` otherwise. Emitted at a function prologue for a
    /// param annotated with a type union whose members are all enforceable.
    CheckUnion = 109,

    /// Composite boundary check (`list[T]`, `dict[K, V]`). `arg` indexes the
    /// `composite_checks` table for a pre-classified composite spec: the container
    /// head (a `LIST`/`DICT` [`type_code`]) plus the classified type parameters.
    /// Pops the top value, checks the container tag and -- when the spec carries
    /// parameters -- that each element (and, for a dict, each key and value)
    /// satisfies the corresponding parameter check, recursively. No coercion.
    /// Pushes the value unchanged on success, raises `TypeError` otherwise. An
    /// unenforceable parameter (unmodeled type, qualified name) is inert, like
    /// `CheckNominal`. Emitted at a function prologue for a param annotated with a
    /// `list`/`dict` constructor.
    CheckComposite = 110,

    /// Generic nominal boundary check (`Option[int]`, `Result[T, E]`). `arg`
    /// indexes the `generic_checks` table for a pre-classified [`ParamCheck::Generic`]
    /// (union name + type arguments). Pops the top value, checks it is a member of
    /// the named union and -- for each payload field that is a type parameter --
    /// that the field value satisfies the corresponding type argument (parametric
    /// substitution against the variant's [`FieldTemplate`]s), recursively. No
    /// coercion. Pushes the value unchanged on success, raises `TypeError`
    /// otherwise. An unknown union name is inert, like `CheckNominal`. Emitted at a
    /// function prologue for a param annotated with a generic union.
    CheckGeneric = 111,

    /// Function-type boundary check (`(int) -> int`, FT3). `arg` IS the
    /// declared arity (no side table: the spec fits the u32). Pops the top
    /// value, requires it to be callable and -- when introspectable -- that
    /// the declared arity is an acceptable call shape (see
    /// [`ParamCheck::Callable`]). Pushes the value unchanged on success,
    /// raises `TypeError` otherwise. Emitted at a function prologue for a
    /// param annotated with a function type.
    CheckCallable = 112,
}

/// Primitive type codes carried by the [`VMOpCode::CheckType`] argument.
///
/// One code per primitive the boundary can enforce. The coercion direction
/// mirrors `Ty::accepts` (PEP 484 numeric tower): a code names the *declared*
/// type a value must end up as.
pub mod type_code {
    pub const INT: u8 = 0;
    pub const FLOAT: u8 = 1;
    pub const STR: u8 = 2;
    pub const BOOL: u8 = 3;
    pub const NONE: u8 = 4;
    /// Composite constructors, enforced at the constructor level only (TH4 v1):
    /// a `list[T]`/`set[T]`/`dict[K, V]`/`tuple[...]` annotation checks the
    /// container type, not the element types. No coercion (like `str`): only a
    /// list satisfies `list`, only a set satisfies `set`, only a tuple a `tuple`.
    pub const LIST: u8 = 5;
    pub const DICT: u8 = 6;
    pub const SET: u8 = 7;
    /// Tuple, a *positional heterogeneous* composite: unlike the homogeneous
    /// `list`/`set` (one element type), `tuple[T0, T1, ...]` carries one type per
    /// position and the arity is part of the contract (a 3-tuple never satisfies
    /// `tuple[int, str]`). A bare `tuple` (no params) checks the container only.
    pub const TUPLE: u8 = 8;

    /// Map a type name (as written in an annotation, with composite params
    /// already stripped) to the code the boundary enforces. Returns `None` for
    /// anything not enforced -- nominals and unmodeled composites -- so the
    /// compiler emits a check only for the primitives (the numeric tower
    /// `int`/`float`/`bool`, `str`, `None`) and the `list`/`dict` constructors.
    pub fn from_name(name: &str) -> Option<u8> {
        match name {
            "int" => Some(INT),
            "float" => Some(FLOAT),
            "str" => Some(STR),
            "bool" => Some(BOOL),
            "None" => Some(NONE),
            "list" => Some(LIST),
            "dict" => Some(DICT),
            "set" => Some(SET),
            "tuple" => Some(TUPLE),
            _ => None,
        }
    }

    /// Display name of a type code, for boundary-check error messages.
    pub fn name(code: u8) -> &'static str {
        match code {
            INT => "int",
            FLOAT => "float",
            STR => "str",
            BOOL => "bool",
            NONE => "None",
            LIST => "list",
            DICT => "dict",
            SET => "set",
            TUPLE => "tuple",
            _ => "?",
        }
    }
}

/// What boundary check a param's annotation compiles to at a function prologue.
/// A primitive annotation emits [`VMOpCode::CheckType`] (numeric-tower coercion);
/// a nominal annotation -- a bare identifier naming a struct/enum/union -- emits
/// [`VMOpCode::CheckNominal`]; everything else (unannotated, a composite like
/// `list[int]`, or a qualified name) emits nothing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ParamCheck {
    /// No boundary check: unannotated, or an annotation the boundary can't enforce.
    None,
    /// Primitive boundary check carrying a [`type_code`] value.
    Primitive(u8),
    /// Nominal boundary check against this declared type name.
    Nominal(String),
    /// Type-union boundary check: the value must satisfy any one member. A
    /// member can be any variant except a nested `Union` or `None` -- every
    /// member-iteration site (the three `check_union` implementations) must
    /// handle each of them, or a valid value gets silently dropped.
    Union(Box<[ParamCheck]>),
    /// Composite boundary check (`list[T]`, `set[T]`, `dict[K, V]`, `tuple[...]`):
    /// the value must be the container named by `head` (a `LIST`/`SET`/`DICT`/
    /// `TUPLE` [`type_code`]), and each of its elements must satisfy the
    /// corresponding `params` check. `params` is empty for a bare composite
    /// (container-only contract); otherwise it carries one check per type
    /// parameter, recursively. For `TUPLE` `params` is positional -- one check per
    /// position, and `params.len()` is the enforced arity; for the others it
    /// carries the element (and, for a dict, key/value) checks.
    Composite { head: u8, params: Box<[ParamCheck]> },
    /// Generic nominal boundary check (`Option[int]`, `Result[T, E]`): the value
    /// must be a member of the union `name`, and each payload field that is a
    /// type parameter must satisfy the corresponding `args` check (parametric
    /// substitution). `args` carries one classified check per type argument,
    /// recursively; the per-variant field-to-parameter mapping lives on the
    /// union's variant types (see [`FieldTemplate`]). An unknown `name` is inert
    /// at runtime, exactly like a bare [`ParamCheck::Nominal`].
    Generic { name: String, args: Box<[ParamCheck]> },
    /// Function-type boundary check (`(int, str) -> bool`, FT3): the value must
    /// be callable, and when its arity is introspectable (a VM function's
    /// CodeObject, a struct constructor's field list) the declared arity must
    /// be an acceptable call shape (`required <= arity <= max`, or `arity >=
    /// required` with a variadic). The parameter and return types are NOT
    /// checked here -- a function's contract is only observable at its calls
    /// (the static half checks provable lambdas; the return is checked at use
    /// sites). A callable whose arity cannot be introspected (Python object,
    /// builtin-by-name) passes on callability alone.
    Callable { arity: u32 },
}

/// How a union variant's payload field is enforced under a generic annotation.
/// Computed once when a union type is built (from the declared `[T, ...]` and the
/// field's type text) and stored on the variant's type; combined at the boundary
/// with the use-site type arguments carried by [`ParamCheck::Generic`].
#[derive(Debug, Clone, PartialEq)]
pub enum FieldTemplate {
    /// The field's declared type is *exactly* the k-th type parameter
    /// (`Some(value: T)` -> `Param(0)`): the required check is the k-th use-site
    /// type argument.
    Param(usize),
    /// A concrete check with no substitution: a fixed type (`code: int`), an
    /// unannotated field, or a type parameter nested in a composite (`list[T]`,
    /// not substituted in v1 -- the container is checked, the element inert).
    Fixed(ParamCheck),
}

/// Classify one union-variant payload field into a [`FieldTemplate`] against the
/// union's declared type parameters. `field_type` is the field's raw annotation
/// text (`Some` when annotated). A text equal to a declared parameter name is a
/// `Param(k)`; anything else is a `Fixed` classified by
/// [`ParamCheck::from_annotation`] (an unannotated field is `Fixed(None)`, inert).
/// Shared by all three executors so the runtime templates are identical.
pub fn compute_field_template(type_params: &[String], field_type: Option<&str>) -> FieldTemplate {
    match field_type {
        Some(text) => {
            let text = text.trim();
            match type_params.iter().position(|p| p == text) {
                Some(k) => FieldTemplate::Param(k),
                None => FieldTemplate::Fixed(ParamCheck::from_annotation(text)),
            }
        }
        None => FieldTemplate::Fixed(ParamCheck::None),
    }
}

impl FieldTemplate {
    /// The boundary check enforced when a value fills this field *at
    /// construction*. A `Fixed` field carries its concrete check; a `Param`
    /// field is not fixed at construction (its type argument binds later, at
    /// the use-site generic boundary), so it is inert here. Shared by all three
    /// executors so a variant payload is enforced identically everywhere -- the
    /// classification (`compute_field_template`) and its construction-time
    /// consequence live in one place, not re-derived per executor.
    pub fn construction_check(&self) -> ParamCheck {
        match self {
            FieldTemplate::Fixed(pc) => pc.clone(),
            FieldTemplate::Param(_) => ParamCheck::None,
        }
    }
}

impl ParamCheck {
    /// Classify the type name written on a param annotation.
    pub fn from_annotation(name: &str) -> Self {
        let members = split_union_members(name);
        if members.len() == 1 {
            return ParamCheck::from_atom(members[0]);
        }
        // Type union: classify each member. Any member the boundary can't enforce
        // (unmodeled composite, qualified name) makes the whole union inert,
        // exactly like `resolve_annotation` widening to `Top`. Members are
        // deduped; a single surviving member degenerates to that member.
        let mut checks: Vec<ParamCheck> = Vec::new();
        for part in &members {
            match ParamCheck::from_atom(part) {
                ParamCheck::None => return ParamCheck::None,
                m if !checks.contains(&m) => checks.push(m),
                _ => {}
            }
        }
        match checks.len() {
            0 => ParamCheck::None,
            1 => checks.into_iter().next().unwrap(),
            _ => ParamCheck::Union(checks.into_boxed_slice()),
        }
    }

    /// Classify a single (non-union) annotation atom. A `list`/`set`/`dict`
    /// composite becomes a `Composite` carrying its classified type parameters
    /// (element, or key/value); a primitive emits a `CheckType`; a bare nominal
    /// identifier a `CheckNominal`; everything else (unmodeled composite,
    /// qualified name) is inert.
    fn from_atom(name: &str) -> Self {
        let name = name.trim();
        // Function type (`(int, str) -> bool`): only the arity is enforceable
        // at the boundary (FT3) -- the component types are observable only at
        // calls. Tested first: the leading `(` cannot start any other atom.
        if let Some((params, _ret)) = fn_type_split(name) {
            return ParamCheck::Callable {
                arity: params.len() as u32,
            };
        }
        if let Some(head) = composite_head(name) {
            // head is "list"/"set"/"dict"/"tuple", all in `type_code::from_name`.
            // The type parameters (when present) are classified recursively and
            // carried for element-level enforcement (positionally for a tuple); a
            // bare composite carries none.
            let head_code = type_code::from_name(head).unwrap();
            let params: Vec<ParamCheck> = composite_params(name)
                .into_iter()
                .map(ParamCheck::from_annotation)
                .collect();
            return ParamCheck::Composite {
                head: head_code,
                params: params.into_boxed_slice(),
            };
        }
        // Generic nominal (`Option[int]`, `Result[T, E]`): a bracketed head that
        // is not a v1 container nor a primitive. Classified structurally (no
        // registry here) -- the head is a nominal identifier, the arguments are
        // classified recursively. An unknown head is inert at runtime, exactly
        // like a bare `Nominal`; a same shape the static lattice recognizes only
        // for a *declared* union (`resolve_atom`), which is the intended
        // asymmetry (structural here, registry-checked there).
        if name.ends_with(']') {
            if let Some(open) = name.find('[') {
                let head = name[..open].trim();
                if type_code::from_name(head).is_none() && is_nominal_type_ident(head) {
                    let args: Vec<ParamCheck> = composite_params(name)
                        .into_iter()
                        .map(ParamCheck::from_annotation)
                        .collect();
                    return ParamCheck::Generic {
                        name: head.to_string(),
                        args: args.into_boxed_slice(),
                    };
                }
            }
        }
        if let Some(code) = type_code::from_name(name) {
            ParamCheck::Primitive(code)
        } else if is_nominal_type_ident(name) {
            ParamCheck::Nominal(name.to_string())
        } else {
            ParamCheck::None
        }
    }
}

/// The constructor head of a v1-enforced composite annotation: `list[T]` and
/// bare `list` yield `Some("list")`, `set[T]` yields `Some("set")`, `dict[K, V]`
/// yields `Some("dict")`, `tuple[...]` yields `Some("tuple")`. Any other atom --
/// a primitive, a nominal, or an unmodeled generic (`Option[T]`) -- yields `None`.
/// Shared with the lattice (`resolve_annotation`) so static E300 and the runtime
/// boundary classify composites identically.
pub fn composite_head(text: &str) -> Option<&'static str> {
    match text.split('[').next().map(str::trim) {
        Some("list") => Some("list"),
        Some("dict") => Some("dict"),
        Some("set") => Some("set"),
        Some("tuple") => Some("tuple"),
        _ => None,
    }
}

/// The type parameters of a composite annotation, as written: `list[int]` yields
/// `["int"]`, `dict[str, int]` yields `["str", "int"]`, `dict[int | str, V]`
/// yields `["int | str", "V"]` (split on top-level commas only, bracket-depth
/// aware so a nested composite's commas don't separate). A bare `list`/`dict` (or
/// any atom without brackets) yields `[]`. Shared with the lattice so static and
/// runtime classify a composite's parameters identically.
pub fn composite_params(text: &str) -> Vec<&str> {
    let Some(open) = text.find('[') else {
        return Vec::new();
    };
    let Some(close) = text.rfind(']') else {
        return Vec::new();
    };
    if close <= open + 1 {
        return Vec::new();
    }
    split_top_level_commas(&text[open + 1..close])
}

/// Split on top-level commas, bracket- and paren-aware (a nested composite's
/// or function type's commas stay inside their delimiters). Trimmed segments;
/// an empty trailing segment (trailing comma) is dropped. Shared by
/// `composite_params` and `fn_type_split` so both sides split identically.
fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let mut params = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                params.push(inner[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        params.push(last);
    }
    params
}

/// Split a type annotation into its top-level union members, ignoring any `|`
/// nested inside composite brackets or function-type parens
/// (`dict[int | str, int]` is one member, not two; `(int | str) -> bool` is
/// one; `list[int] | str` is two). The arrow is right-absorbing, mirroring the
/// grammar's `prec.right`: once a top-level `->` is seen, no later `|` splits
/// (`(int) -> int | str` is ONE member whose return is the union; a function
/// type as a union member goes last: `None | (int) -> int`). Members are
/// trimmed; a non-union annotation yields a single element. Shared with the
/// lattice so both classifiers see the same member structure.
pub fn split_union_members(text: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    let mut prev = '\0';
    let mut absorbed = false;
    for (i, c) in text.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            '>' if depth == 0 && prev == '-' => absorbed = true,
            '|' if depth == 0 && !absorbed => {
                members.push(text[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
        prev = c;
    }
    members.push(text[start..].trim());
    members
}

/// Arity acceptance for a function-type boundary (`ParamCheck::Callable`),
/// shared by the three executors so the rule cannot drift: a callable with
/// `fixed` positional slots (the vararg slot excluded), `real_defaults` of
/// them defaulted, accepts a declared arity when `required <= arity <= fixed`
/// -- or `arity >= required` with a variadic. Returns `(required, accepts)`;
/// `required` feeds the error message.
pub fn callable_arity_accepts(fixed: usize, has_vararg: bool, real_defaults: usize, arity: usize) -> (usize, bool) {
    let required = fixed.saturating_sub(real_defaults);
    let accepts = if has_vararg {
        arity >= required
    } else {
        arity >= required && arity <= fixed
    };
    (required, accepts)
}

/// Split a function-type annotation (`(int, str) -> bool`) into its parameter
/// texts and return text. Returns `None` when the text is not a function type:
/// it must start with `(`, close that paren at top level, and be followed by
/// `->`. The return is everything after the arrow (right absorption: it may
/// itself be a union or another function type). Parameters split on top-level
/// commas, paren/bracket-aware. Used by the static resolver today; the runtime
/// classifier leaves FT annotations inert (no `ParamCheck` arm yet) and must
/// call this same splitter when the FT boundary lands, so the two sides keep
/// one shape.
pub fn fn_type_split(text: &str) -> Option<(Vec<&str>, &str)> {
    let text = text.trim();
    if !text.starts_with('(') {
        return None;
    }
    // Find the matching close paren of the leading open.
    let mut depth: i32 = 0;
    let mut close = None;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 && c == ')' {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let rest = text[close + 1..].trim_start();
    let ret = rest.strip_prefix("->")?.trim();
    if ret.is_empty() {
        return None;
    }
    Some((split_top_level_commas(&text[1..close]), ret))
}

/// True if `name` is a bare identifier naming a nominal type (struct/enum/union).
/// Composites (`list[int]`), type unions (`int | str`), and qualified names
/// (`Mod.Type`) are excluded -- the boundary doesn't enforce them, so they stay
/// inert rather than emitting a check.
fn is_nominal_type_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Class of a runtime value for the union membership test, abstracting over the
/// executor's value representation (NaN-box `Value` in either VM, `PyAny` in AST
/// mode). Each executor fills these from its own type predicates; the
/// numeric-tower logic lives once in [`primitive_membership`]. `int_like` covers
/// integers including bigint; whether it also includes bool is executor-dependent
/// and irrelevant, since `bool_like` covers bool in every tower arm. `list_like`,
/// `set_like`, `dict_like` and `tuple_like` carry the v1 composite constructors
/// so a `list`/`set`/`dict`/`tuple` union member is enforceable too.
pub struct PrimitiveClass {
    pub int_like: bool,
    pub float_like: bool,
    pub str_like: bool,
    pub bool_like: bool,
    pub nil_like: bool,
    pub list_like: bool,
    pub set_like: bool,
    pub dict_like: bool,
    pub tuple_like: bool,
}

/// Does a value of class `c` belong to the type named by `code`, under the PEP
/// 484 numeric tower (`bool` <: `int` <: `float`) and with **no coercion**?
/// Composites (`list`/`dict`) match by their container tag, params ignored.
/// Shared by the `CheckUnion` membership test across the two VMs and AST mode,
/// so the tower is defined in exactly one place (mirrors `Ty::accepts` and
/// `boundary_coerce`'s acceptance conditions, minus the coercion).
pub fn primitive_membership(code: u8, c: &PrimitiveClass) -> bool {
    match code {
        type_code::INT => c.int_like || c.bool_like,
        type_code::FLOAT => c.float_like || c.int_like || c.bool_like,
        type_code::STR => c.str_like,
        type_code::BOOL => c.bool_like,
        type_code::NONE => c.nil_like,
        type_code::LIST => c.list_like,
        type_code::DICT => c.dict_like,
        type_code::SET => c.set_like,
        type_code::TUPLE => c.tuple_like,
        _ => false,
    }
}

/// Render a single `ParamCheck` for a boundary error message: `int`, `Point`,
/// `int | str` (union), `list[int]` / `dict[str, int]` (composite). Shared by the
/// `CheckUnion`/`CheckComposite` `TypeError` paths across both VMs and AST mode.
pub fn format_param_check(pc: &ParamCheck) -> String {
    match pc {
        ParamCheck::None => "?".to_string(),
        ParamCheck::Primitive(code) => type_code::name(*code).to_string(),
        ParamCheck::Nominal(name) => name.clone(),
        ParamCheck::Union(members) => format_union_members(members),
        ParamCheck::Composite { head, params } => {
            if params.is_empty() {
                type_code::name(*head).to_string()
            } else {
                let inner: Vec<String> = params.iter().map(format_param_check).collect();
                format!("{}[{}]", type_code::name(*head), inner.join(", "))
            }
        }
        ParamCheck::Generic { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<String> = args.iter().map(format_param_check).collect();
                format!("{name}[{}]", inner.join(", "))
            }
        }
        ParamCheck::Callable { arity } => format!("a callable taking {arity} argument(s)"),
    }
}

/// Render a list of union member checks as `int | str` for a boundary error
/// message. Shared by `CheckUnion`'s `TypeError` path in both VMs and AST mode.
pub fn format_union_members(members: &[ParamCheck]) -> String {
    members.iter().map(format_param_check).collect::<Vec<_>>().join(" | ")
}

impl VMOpCode {
    /// Highest opcode value. Used for range checks and cache invalidation.
    pub const MAX: u8 = VMOpCode::CheckCallable as u8;

    /// Highest shared opcode value (same values as IROpCode).
    pub const SHARED_MAX: u8 = VMOpCode::Breakpoint as u8;

    /// Convert from u8, returning None for invalid values.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        if (1..=Self::MAX).contains(&v) {
            // SAFETY: We verified the range matches our enum
            Some(unsafe { std::mem::transmute::<u8, Self>(v) })
        } else {
            None
        }
    }

    /// Check if this opcode has an argument.
    #[inline]
    pub fn has_arg(self) -> bool {
        matches!(
            self,
            // Shared
            VMOpCode::Broadcast
                | VMOpCode::GetAttr
                | VMOpCode::SetAttr
                | VMOpCode::NdRecursion
                | VMOpCode::NdMap
                // Load/Store
                | VMOpCode::LoadConst
                | VMOpCode::LoadLocal
                | VMOpCode::StoreLocal
                | VMOpCode::LoadScope
                | VMOpCode::StoreScope
                | VMOpCode::LoadGlobal
                // Jumps
                | VMOpCode::Jump
                | VMOpCode::JumpIfFalse
                | VMOpCode::JumpIfTrue
                | VMOpCode::JumpIfFalseOrPop
                | VMOpCode::JumpIfTrueOrPop
                | VMOpCode::JumpIfNone
                | VMOpCode::JumpIfNotNoneOrPop
                // Iteration
                | VMOpCode::ForIter
                | VMOpCode::ForRangeInt
                | VMOpCode::ForRangeStep
                // Functions
                | VMOpCode::Call
                | VMOpCode::CallKw
                | VMOpCode::CallMethod
                | VMOpCode::TailCall
                | VMOpCode::MakeFunction
                // Collections
                | VMOpCode::BuildList
                | VMOpCode::BuildTuple
                | VMOpCode::BuildSet
                | VMOpCode::BuildDict
                | VMOpCode::BuildSlice
                // Match
                | VMOpCode::MatchPatternVM
                | VMOpCode::MatchAssignPatternVM
                | VMOpCode::MatchFail
                // Unpack
                | VMOpCode::UnpackSequence
                | VMOpCode::UnpackEx
                // Structures
                | VMOpCode::MakeStruct
                | VMOpCode::MakeTrait
                | VMOpCode::MakeUnion
                | VMOpCode::PatchClosure
                // Control
                | VMOpCode::Exit
                // String formatting
                | VMOpCode::FormatValue
                | VMOpCode::BuildString
                // Exception handling
                | VMOpCode::SetupExcept
                | VMOpCode::SetupFinally
                | VMOpCode::Raise
                | VMOpCode::CheckExcMatch
                // Type boundary (arg = type_code)
                | VMOpCode::CheckType
                // Nominal boundary (arg = names index)
                | VMOpCode::CheckNominal
                // Type-union boundary (arg = union_checks index)
                | VMOpCode::CheckUnion
                // Composite boundary (arg = composite_checks index)
                | VMOpCode::CheckComposite
                // Generic-nominal boundary (arg = generic_checks index)
                | VMOpCode::CheckGeneric
                | VMOpCode::CheckCallable
        )
    }

    /// Get stack effect: (pops, pushes). -1 means depends on arg.
    #[inline]
    pub fn stack_effect(self) -> (i8, i8) {
        match self {
            // Shared: Arithmetic
            VMOpCode::Add => (2, 1),
            VMOpCode::Sub => (2, 1),
            VMOpCode::Mul => (2, 1),
            VMOpCode::FloorDiv => (2, 1),
            VMOpCode::Mod => (2, 1),
            VMOpCode::Pow => (2, 1),
            VMOpCode::Neg => (1, 1),
            VMOpCode::Pos => (1, 1),
            // Shared: Comparison
            VMOpCode::Eq => (2, 1),
            VMOpCode::Ne => (2, 1),
            VMOpCode::Lt => (2, 1),
            VMOpCode::Le => (2, 1),
            VMOpCode::Gt => (2, 1),
            VMOpCode::Ge => (2, 1),
            // Shared: Unary logic
            VMOpCode::Not => (1, 1),
            // Shared: Bitwise
            VMOpCode::BAnd => (2, 1),
            VMOpCode::BOr => (2, 1),
            VMOpCode::BXor => (2, 1),
            VMOpCode::BNot => (1, 1),
            VMOpCode::LShift => (2, 1),
            VMOpCode::RShift => (2, 1),
            // Shared: Access
            VMOpCode::GetAttr => (1, 1),
            VMOpCode::SetAttr => (2, 0),
            VMOpCode::GetItem => (2, 1),
            VMOpCode::SetItem => (3, 0),
            // Shared: Broadcasting & ND
            VMOpCode::Broadcast => (-1, 1),
            VMOpCode::NdRecursion => (-1, 1),
            VMOpCode::NdMap => (-1, 1),
            VMOpCode::NdEmptyTopos => (0, 1),
            // Shared: Meta
            VMOpCode::Nop => (0, 0),
            VMOpCode::Breakpoint => (0, 0),
            // VM: Arithmetic
            VMOpCode::Div => (2, 1),
            // VM: Comparison
            VMOpCode::In => (2, 1),
            VMOpCode::NotIn => (2, 1),
            VMOpCode::Is => (2, 1),
            VMOpCode::IsNot => (2, 1),
            // VM: Conversion
            VMOpCode::ToBool => (1, 1),
            VMOpCode::TypeOf => (1, 1),
            // VM: Load/Store
            VMOpCode::LoadConst => (0, 1),
            VMOpCode::LoadLocal => (0, 1),
            VMOpCode::StoreLocal => (1, 0),
            VMOpCode::LoadScope => (0, 1),
            VMOpCode::StoreScope => (1, 0),
            VMOpCode::LoadGlobal => (0, 1),
            // VM: Stack
            VMOpCode::PopTop => (1, 0),
            VMOpCode::DupTop => (1, 2),
            VMOpCode::RotTwo => (2, 2),
            // VM: Jumps
            VMOpCode::Jump => (0, 0),
            VMOpCode::JumpIfFalse => (1, 0),
            VMOpCode::JumpIfTrue => (1, 0),
            VMOpCode::JumpIfFalseOrPop => (1, 0),
            VMOpCode::JumpIfTrueOrPop => (1, 0),
            VMOpCode::JumpIfNone => (1, 0),
            VMOpCode::JumpIfNotNoneOrPop => (1, 0),
            // VM: Iteration
            VMOpCode::GetIter => (1, 1),
            VMOpCode::ForIter => (0, 1),
            VMOpCode::ForRangeInt => (0, 0),
            VMOpCode::ForRangeStep => (0, 0),
            // VM: Functions
            VMOpCode::Call => (-1, 1),
            VMOpCode::CallKw => (-1, 1),
            VMOpCode::CallMethod => (-1, 1),
            VMOpCode::TailCall => (-1, 0),
            VMOpCode::Return => (1, 0),
            VMOpCode::MakeFunction => (1, 1),
            // VM: Collections
            VMOpCode::BuildList => (-1, 1),
            VMOpCode::BuildTuple => (-1, 1),
            VMOpCode::BuildSet => (-1, 1),
            VMOpCode::BuildDict => (-1, 1),
            VMOpCode::BuildSlice => (-1, 1),
            // VM: String formatting
            VMOpCode::FormatValue => (-1, 1), // pops value [+spec], pushes string
            VMOpCode::BuildString => (-1, 1), // pops n strings, pushes concatenated
            // VM: Blocks
            VMOpCode::PushBlock => (0, 0),
            VMOpCode::PopBlock => (0, 0),
            VMOpCode::Break => (0, 0),
            VMOpCode::Continue => (0, 0),
            // VM: Match
            VMOpCode::MatchPattern => (1, 1),
            VMOpCode::MatchPatternVM => (1, 1),
            VMOpCode::MatchAssignPatternVM => (1, 1),
            VMOpCode::BindMatch => (1, 0),
            VMOpCode::MatchFail => (0, 0),
            // VM: Unpack
            VMOpCode::UnpackSequence => (1, -1),
            VMOpCode::UnpackEx => (1, -1),
            // VM: Definitions
            VMOpCode::MakeStruct => (0, 0),
            VMOpCode::MakeTrait => (0, 0),
            VMOpCode::MakeEnum => (0, 0),
            VMOpCode::MakeUnion => (0, 0),
            VMOpCode::PatchClosure => (2, 0),
            // VM: Control
            VMOpCode::Halt => (0, 0),
            VMOpCode::Exit => (-1, 0),
            // VM: Intrinsics
            VMOpCode::Globals => (0, 1),
            VMOpCode::Locals => (0, 1),
            // VM: Exception handling
            VMOpCode::SetupExcept => (0, 0),
            VMOpCode::SetupFinally => (0, 0),
            VMOpCode::PopHandler => (0, 0),
            VMOpCode::Raise => (1, 0),         // pops exception value (or 0 for bare raise)
            VMOpCode::CheckExcMatch => (0, 1), // pushes bool
            VMOpCode::LoadException => (0, 1), // pushes exception message
            VMOpCode::ResumeUnwind => (0, 0),
            VMOpCode::ClearException => (0, 0),
            // VM: Type boundary
            VMOpCode::CheckType => (1, 1),
            // VM: Typed arithmetic
            VMOpCode::AddInt => (2, 1),
            VMOpCode::AddFloat => (2, 1),
            VMOpCode::SubInt => (2, 1),
            VMOpCode::SubFloat => (2, 1),
            VMOpCode::MulInt => (2, 1),
            VMOpCode::MulFloat => (2, 1),
            VMOpCode::DivFloat => (2, 1),
            // VM: Nominal boundary
            VMOpCode::CheckNominal => (1, 1),
            // VM: Type-union boundary
            VMOpCode::CheckUnion => (1, 1),
            // VM: Composite boundary
            VMOpCode::CheckComposite => (1, 1),
            // VM: Generic-nominal boundary
            VMOpCode::CheckGeneric => (1, 1),
            VMOpCode::CheckCallable => (1, 1),
        }
    }
}

impl std::fmt::Display for VMOpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Instruction: opcode + optional argument
#[derive(Debug, Clone, Copy)]
pub struct Instruction {
    pub op: VMOpCode,
    pub arg: u32,
}

impl Instruction {
    #[inline]
    pub fn new(op: VMOpCode, arg: u32) -> Self {
        Self { op, arg }
    }

    #[inline]
    pub fn simple(op: VMOpCode) -> Self {
        Self { op, arg: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_bounds() {
        // Shared zone
        assert_eq!(VMOpCode::Add as u8, 1);
        assert_eq!(VMOpCode::Breakpoint as u8, 31);
        assert_eq!(VMOpCode::SHARED_MAX, 31);

        // VM-only zone: spot-check category boundaries
        assert_eq!(VMOpCode::Div as u8, 32);
        assert_eq!(VMOpCode::TypeOf as u8, 38);
        assert_eq!(VMOpCode::MakeStruct as u8, 83);
        assert_eq!(VMOpCode::MakeEnum as u8, 85);
        assert_eq!(VMOpCode::Halt as u8, 86);
        assert_eq!(VMOpCode::Exit as u8, 87);
        assert_eq!(VMOpCode::Globals as u8, 88);
        assert_eq!(VMOpCode::Locals as u8, 89);
        assert_eq!(VMOpCode::SetupExcept as u8, 90);
        assert_eq!(VMOpCode::ResumeUnwind as u8, 96);
        assert_eq!(VMOpCode::ClearException as u8, 97);
        assert_eq!(VMOpCode::MakeUnion as u8, 98);
        assert_eq!(VMOpCode::PatchClosure as u8, 99);
        assert_eq!(VMOpCode::CheckType as u8, 100);
        assert_eq!(VMOpCode::AddInt as u8, 101);
        assert_eq!(VMOpCode::AddFloat as u8, 102);
        assert_eq!(VMOpCode::SubInt as u8, 103);
        assert_eq!(VMOpCode::SubFloat as u8, 104);
        assert_eq!(VMOpCode::MulInt as u8, 105);
        assert_eq!(VMOpCode::MulFloat as u8, 106);
        assert_eq!(VMOpCode::DivFloat as u8, 107);
        assert_eq!(VMOpCode::CheckNominal as u8, 108);
        assert_eq!(VMOpCode::CheckUnion as u8, 109);
        assert_eq!(VMOpCode::CheckComposite as u8, 110);
        assert_eq!(VMOpCode::CheckGeneric as u8, 111);
        assert_eq!(VMOpCode::CheckCallable as u8, 112);
        assert_eq!(VMOpCode::MAX, VMOpCode::CheckCallable as u8);
    }

    #[test]
    fn test_contiguous() {
        // Verify no gaps in the enum (required for transmute safety)
        for i in 1..=VMOpCode::MAX {
            assert!(VMOpCode::from_u8(i).is_some(), "gap at value {i}");
        }
    }

    #[test]
    fn test_from_u8_roundtrip() {
        // Spot-check each category
        let opcodes = [
            VMOpCode::Add,
            VMOpCode::Breakpoint,
            VMOpCode::Div,
            VMOpCode::In,
            VMOpCode::ToBool,
            VMOpCode::LoadConst,
            VMOpCode::Jump,
            VMOpCode::JumpIfNotNoneOrPop,
            VMOpCode::GetIter,
            VMOpCode::ForRangeStep,
            VMOpCode::Call,
            VMOpCode::CallMethod,
            VMOpCode::BuildList,
            VMOpCode::Break,
            VMOpCode::MatchPatternVM,
            VMOpCode::MatchAssignPatternVM,
            VMOpCode::UnpackSequence,
            VMOpCode::MakeStruct,
            VMOpCode::MakeTrait,
            VMOpCode::MakeEnum,
            VMOpCode::Halt,
            VMOpCode::Exit,
            VMOpCode::FormatValue,
            VMOpCode::BuildString,
            VMOpCode::TypeOf,
            VMOpCode::Globals,
            VMOpCode::Locals,
            VMOpCode::CheckType,
        ];
        for opcode in opcodes {
            assert_eq!(VMOpCode::from_u8(opcode as u8), Some(opcode));
        }
    }

    #[test]
    fn test_from_u8_invalid() {
        assert_eq!(VMOpCode::from_u8(0), None);
        assert_eq!(VMOpCode::from_u8(VMOpCode::MAX + 1), None);
        assert_eq!(VMOpCode::from_u8(255), None);
    }

    #[test]
    fn param_check_classifies_annotations() {
        // Primitives compile to a CheckType code.
        assert_eq!(
            ParamCheck::from_annotation("int"),
            ParamCheck::Primitive(type_code::INT)
        );
        assert_eq!(
            ParamCheck::from_annotation("None"),
            ParamCheck::Primitive(type_code::NONE)
        );
        // Bare identifiers (struct/enum/union/trait names) are nominal.
        assert_eq!(
            ParamCheck::from_annotation("Point"),
            ParamCheck::Nominal("Point".to_string())
        );
        assert_eq!(
            ParamCheck::from_annotation("_Private"),
            ParamCheck::Nominal("_Private".to_string())
        );
        // list/set/dict composites carry their classified type parameters; a
        // bare `list`/`set`/`dict` carries none (container-only contract).
        assert_eq!(
            ParamCheck::from_annotation("list[int]"),
            ParamCheck::Composite {
                head: type_code::LIST,
                params: Box::new([ParamCheck::Primitive(type_code::INT)]),
            }
        );
        assert_eq!(
            ParamCheck::from_annotation("list"),
            ParamCheck::Composite {
                head: type_code::LIST,
                params: Box::new([]),
            }
        );
        assert_eq!(
            ParamCheck::from_annotation("dict[str, int]"),
            ParamCheck::Composite {
                head: type_code::DICT,
                params: Box::new([
                    ParamCheck::Primitive(type_code::STR),
                    ParamCheck::Primitive(type_code::INT),
                ]),
            }
        );
        assert_eq!(
            ParamCheck::from_annotation("set[int]"),
            ParamCheck::Composite {
                head: type_code::SET,
                params: Box::new([ParamCheck::Primitive(type_code::INT)]),
            }
        );
        // A tuple is positional: one classified check per position (heterogeneous),
        // and a bare `tuple` carries none (arity unconstrained).
        assert_eq!(
            ParamCheck::from_annotation("tuple[int, str]"),
            ParamCheck::Composite {
                head: type_code::TUPLE,
                params: Box::new([
                    ParamCheck::Primitive(type_code::INT),
                    ParamCheck::Primitive(type_code::STR),
                ]),
            }
        );
        assert_eq!(
            ParamCheck::from_annotation("tuple"),
            ParamCheck::Composite {
                head: type_code::TUPLE,
                params: Box::new([]),
            }
        );
        // A generic nominal is classified structurally (head + recursive args);
        // an unknown head is resolved (inert-or-not) only at runtime.
        assert_eq!(
            ParamCheck::from_annotation("Option[int]"),
            ParamCheck::Generic {
                name: "Option".to_string(),
                args: Box::new([ParamCheck::Primitive(type_code::INT)]),
            }
        );
        assert_eq!(
            ParamCheck::from_annotation("Result[int, str]"),
            ParamCheck::Generic {
                name: "Result".to_string(),
                args: Box::new([
                    ParamCheck::Primitive(type_code::INT),
                    ParamCheck::Primitive(type_code::STR)
                ]),
            }
        );
        // Nested: the argument is classified recursively.
        assert_eq!(
            ParamCheck::from_annotation("Option[list[int]]"),
            ParamCheck::Generic {
                name: "Option".to_string(),
                args: Box::new([ParamCheck::Composite {
                    head: type_code::LIST,
                    params: Box::new([ParamCheck::Primitive(type_code::INT)]),
                }]),
            }
        );
        // Qualified names and junk stay inert; a primitive head with brackets is
        // not a generic (it is not a nominal type).
        assert_eq!(ParamCheck::from_annotation("Mod.Type"), ParamCheck::None);
        assert_eq!(ParamCheck::from_annotation(""), ParamCheck::None);
        assert_eq!(ParamCheck::from_annotation("3bad"), ParamCheck::None);
    }

    #[test]
    fn composite_params_splits_top_level_commas() {
        assert!(composite_params("list").is_empty());
        assert!(composite_params("dict").is_empty());
        assert_eq!(composite_params("list[int]"), vec!["int"]);
        assert_eq!(composite_params("dict[str, int]"), vec!["str", "int"]);
        // A `,` nested in a composite param is not a separator.
        assert_eq!(composite_params("dict[str, list[int]]"), vec!["str", "list[int]"]);
        // A `|` (union) inside a param stays within that param.
        assert_eq!(composite_params("dict[int | str, int]"), vec!["int | str", "int"]);
        // Empty brackets degenerate to no params.
        assert!(composite_params("list[]").is_empty());
    }

    #[test]
    fn composite_head_recognizes_list_set_dict_tuple() {
        assert_eq!(composite_head("list"), Some("list"));
        assert_eq!(composite_head("list[int]"), Some("list"));
        assert_eq!(composite_head("set[int]"), Some("set"));
        assert_eq!(composite_head("set"), Some("set"));
        assert_eq!(composite_head("dict[str, int]"), Some("dict"));
        assert_eq!(composite_head("tuple[int, str]"), Some("tuple"));
        assert_eq!(composite_head("tuple"), Some("tuple"));
        assert_eq!(composite_head("Option[T]"), None);
        assert_eq!(composite_head("int"), None);
        assert_eq!(composite_head("Point"), None);
    }

    #[test]
    fn split_union_is_bracket_depth_aware() {
        assert_eq!(split_union_members("int"), vec!["int"]);
        assert_eq!(split_union_members("int | str"), vec!["int", "str"]);
        // A `|` inside composite brackets is not a separator.
        assert_eq!(
            split_union_members("dict[int | str, int]"),
            vec!["dict[int | str, int]"]
        );
        // A composite as a top-level union member splits correctly.
        assert_eq!(split_union_members("list[int] | str"), vec!["list[int]", "str"]);
    }

    #[test]
    fn param_check_classifies_unions() {
        use type_code::{INT, NONE, STR};
        // Mixed primitive/nominal union, members in source order.
        assert_eq!(
            ParamCheck::from_annotation("int | str"),
            ParamCheck::Union(Box::new([ParamCheck::Primitive(INT), ParamCheck::Primitive(STR)]))
        );
        assert_eq!(
            ParamCheck::from_annotation("Point | None"),
            ParamCheck::Union(Box::new([
                ParamCheck::Nominal("Point".to_string()),
                ParamCheck::Primitive(NONE)
            ]))
        );
        // A list/dict member is enforceable inside a union (carrying its params).
        use type_code::LIST;
        assert_eq!(
            ParamCheck::from_annotation("int | list[int]"),
            ParamCheck::Union(Box::new([
                ParamCheck::Primitive(INT),
                ParamCheck::Composite {
                    head: LIST,
                    params: Box::new([ParamCheck::Primitive(INT)]),
                },
            ]))
        );
        // A set member is enforceable inside a union (carrying its element).
        use type_code::SET;
        assert_eq!(
            ParamCheck::from_annotation("int | set[str]"),
            ParamCheck::Union(Box::new([
                ParamCheck::Primitive(INT),
                ParamCheck::Composite {
                    head: SET,
                    params: Box::new([ParamCheck::Primitive(STR)]),
                },
            ]))
        );
        // A tuple member is enforceable inside a union (carrying its positions).
        use type_code::TUPLE;
        assert_eq!(
            ParamCheck::from_annotation("int | tuple[int, str]"),
            ParamCheck::Union(Box::new([
                ParamCheck::Primitive(INT),
                ParamCheck::Composite {
                    head: TUPLE,
                    params: Box::new([ParamCheck::Primitive(INT), ParamCheck::Primitive(STR)]),
                },
            ]))
        );
        // A generic nominal member is enforceable, so the union stays enforced
        // (the head's existence is resolved at runtime, not here).
        assert_eq!(
            ParamCheck::from_annotation("int | Option[int]"),
            ParamCheck::Union(Box::new([
                ParamCheck::Primitive(INT),
                ParamCheck::Generic {
                    name: "Option".to_string(),
                    args: Box::new([ParamCheck::Primitive(INT)]),
                },
            ]))
        );
        // A qualified-name member the boundary cannot enforce still makes the
        // whole union inert (unchanged).
        assert_eq!(ParamCheck::from_annotation("int | Mod.Type"), ParamCheck::None);
        // Deduplication collapses a single surviving member to that member.
        assert_eq!(ParamCheck::from_annotation("int | int"), ParamCheck::Primitive(INT));
    }

    #[test]
    fn compute_field_template_maps_params_and_fixes() {
        use type_code::{INT, LIST};
        let tps = vec!["T".to_string(), "E".to_string()];
        // A field whose type is exactly a parameter -> Param(k).
        assert_eq!(compute_field_template(&tps, Some("T")), FieldTemplate::Param(0));
        assert_eq!(compute_field_template(&tps, Some("E")), FieldTemplate::Param(1));
        // A concrete type -> Fixed(classified).
        assert_eq!(
            compute_field_template(&tps, Some("int")),
            FieldTemplate::Fixed(ParamCheck::Primitive(INT))
        );
        // A parameter nested in a composite is NOT substituted in v1: the
        // container is fixed, the inner `T` classifies as an (inert) nominal.
        assert_eq!(
            compute_field_template(&tps, Some("list[T]")),
            FieldTemplate::Fixed(ParamCheck::Composite {
                head: LIST,
                params: Box::new([ParamCheck::Nominal("T".to_string())]),
            })
        );
        // An unannotated field -> Fixed(None), inert.
        assert_eq!(
            compute_field_template(&tps, None),
            FieldTemplate::Fixed(ParamCheck::None)
        );
    }
}

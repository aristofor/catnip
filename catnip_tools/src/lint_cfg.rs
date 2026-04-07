// FILE: catnip_tools/src/lint_cfg.rs
//! CFG-based deep analysis for the linter.
//!
//! Builds a lightweight control flow graph directly from the tree-sitter CST
//! (no IR dependency). Used when `--deep` / `check_ir` is active.
//!
//! Rules: W310 (variable possibly uninitialized), W311 (unreachable code).

use crate::config::LintConfig;
use crate::linter::{Diagnostic, Severity};
use catnip_grammar::node_kinds as NK;
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// CFG structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Unconditional,
    CondTrue,
    CondFalse,
    LoopBack,
    LoopExit,
    Exception,
}

#[derive(Debug)]
struct LintEdge {
    target: usize,
    #[allow(dead_code)] // used for debugging and future edge-type-aware analysis
    kind: EdgeKind,
}

#[derive(Debug)]
struct LintBlock {
    id: usize,
    /// Variables definitely assigned in this block.
    /// Maps name -> byte offset of the first def (for intra-block ordering).
    defs: HashMap<String, usize>,
    /// Variables read in this block.
    reads: Vec<(String, usize, usize, usize)>, // (name, line, col, byte_offset)
    /// Outgoing edges.
    successors: Vec<LintEdge>,
    /// Incoming block ids (filled after build).
    predecessors: Vec<usize>,
    /// Block terminates (return/raise without fallthrough).
    terminates: bool,
    /// True when termination comes from all branches of a control flow
    /// construct (if/match/try) terminating, not from a direct return/raise.
    merge_terminates: bool,
}

impl LintBlock {
    fn new(id: usize) -> Self {
        Self {
            id,
            defs: HashMap::new(),
            reads: Vec::new(),
            successors: Vec::new(),
            predecessors: Vec::new(),
            terminates: false,
            merge_terminates: false,
        }
    }
}

struct LintCFG {
    blocks: Vec<LintBlock>,
    entry: usize,
    exit: usize,
    /// Positions of first unreachable statement after a terminating block.
    dead_code: Vec<(usize, usize)>, // (line, col)
}

impl LintCFG {
    fn new_block(&mut self) -> usize {
        let id = self.blocks.len();
        self.blocks.push(LintBlock::new(id));
        id
    }

    fn add_edge(&mut self, from: usize, to: usize, kind: EdgeKind) {
        self.blocks[from].successors.push(LintEdge { target: to, kind });
    }

    /// Fill predecessor lists from successor edges.
    fn compute_predecessors(&mut self) {
        for b in &mut self.blocks {
            b.predecessors.clear();
        }
        // Collect (target, source) pairs first to avoid borrow issues.
        let pairs: Vec<(usize, usize)> = self
            .blocks
            .iter()
            .flat_map(|b| b.successors.iter().map(move |e| (e.target, b.id)))
            .collect();
        for (target, source) in pairs {
            self.blocks[target].predecessors.push(source);
        }
    }
}

// ---------------------------------------------------------------------------
// CFG builder -- CST walk
// ---------------------------------------------------------------------------

struct CfgBuilder<'a> {
    cfg: LintCFG,
    source: &'a str,
    /// Stack of (loop_header, loop_exit) for break/continue.
    loop_stack: Vec<(usize, usize)>,
}

impl<'a> CfgBuilder<'a> {
    fn new(source: &'a str) -> Self {
        let mut cfg = LintCFG {
            blocks: Vec::new(),
            entry: 0,
            exit: 0,
            dead_code: Vec::new(),
        };
        let entry = cfg.new_block();
        let exit = cfg.new_block();
        cfg.entry = entry;
        cfg.exit = exit;
        Self {
            cfg,
            source,
            loop_stack: Vec::new(),
        }
    }

    fn node_text(&self, node: Node) -> String {
        node.utf8_text(self.source.as_bytes()).unwrap_or("").to_string()
    }

    /// Build CFG from a source_file or block node.
    /// Returns the completed CFG.
    fn build(mut self, root: Node) -> LintCFG {
        let current = self.cfg.entry;
        let after = self.walk_children(root, current);
        // Connect final block to exit if not terminated.
        if !self.cfg.blocks[after].terminates {
            self.cfg.add_edge(after, self.cfg.exit, EdgeKind::Unconditional);
        }
        self.cfg.compute_predecessors();
        self.cfg
    }

    /// Walk children of a container node (source_file, block).
    /// Returns the block id where control continues after.
    fn walk_children(&mut self, node: Node, mut current: usize) -> usize {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if self.cfg.blocks[current].terminates {
                // W311: only for inter-branch termination (if/match/try
                // where all branches terminate). Intra-block return/raise
                // is already covered by W300.
                if child.is_named() && self.cfg.blocks[current].merge_terminates {
                    let line = child.start_position().row + 1;
                    let col = child.start_position().column + 1;
                    self.cfg.dead_code.push((line, col));
                }
                break;
            }
            current = self.walk_stmt(child, current);
        }
        current
    }

    /// Process a single CST statement. Returns the block where control
    /// continues after this statement.
    fn walk_stmt(&mut self, node: Node, current: usize) -> usize {
        match node.kind() {
            NK::IF_EXPR => self.walk_if(node, current),
            NK::WHILE_STMT => self.walk_while(node, current),
            NK::FOR_STMT => self.walk_for(node, current),
            NK::MATCH_EXPR => self.walk_match(node, current),
            NK::TRY_STMT => self.walk_try(node, current),
            NK::RETURN_STMT | NK::RAISE_STMT => {
                self.collect_reads(node, current);
                self.cfg.blocks[current].terminates = true;
                self.cfg.add_edge(current, self.cfg.exit, EdgeKind::Unconditional);
                current
            }
            NK::BREAK_STMT => {
                if let Some(&(_, loop_exit)) = self.loop_stack.last() {
                    self.cfg.blocks[current].terminates = true;
                    self.cfg.add_edge(current, loop_exit, EdgeKind::LoopExit);
                }
                current
            }
            NK::CONTINUE_STMT => {
                if let Some(&(loop_header, _)) = self.loop_stack.last() {
                    self.cfg.blocks[current].terminates = true;
                    self.cfg.add_edge(current, loop_header, EdgeKind::LoopBack);
                }
                current
            }
            NK::ASSIGNMENT => {
                // RHS reads first, then LHS defs.
                self.collect_assignment(node, current);
                current
            }
            NK::BLOCK => self.walk_children(node, current),
            NK::LAMBDA_EXPR | NK::STRUCT_STMT | NK::TRAIT_STMT => {
                // Opaque -- don't descend into nested scopes.
                // The name itself is a def in the current scope.
                self.collect_toplevel_def(node, current);
                current
            }
            NK::STATEMENT => {
                // Wrapper node -- walk its children
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if !self.cfg.blocks[current].terminates {
                        return self.walk_stmt(child, current);
                    }
                }
                current
            }
            _ => {
                // Expression statement or other -- collect reads.
                self.collect_reads(node, current);
                current
            }
        }
    }

    fn walk_if(&mut self, node: Node, current: usize) -> usize {
        let merge = self.cfg.new_block();
        let mut all_branches_terminate = true;
        let mut has_else = false;

        // Condition reads in current block.
        if let Some(cond) = node.child_by_field_name("condition") {
            self.collect_reads(cond, current);
        }

        // True branch (field name is "consequence" in grammar)
        if let Some(body) = node.child_by_field_name("consequence") {
            let true_block = self.cfg.new_block();
            self.cfg.add_edge(current, true_block, EdgeKind::CondTrue);
            let after_true = self.walk_children(body, true_block);
            if !self.cfg.blocks[after_true].terminates {
                self.cfg.add_edge(after_true, merge, EdgeKind::Unconditional);
                all_branches_terminate = false;
            }
        } else {
            all_branches_terminate = false;
        }

        // elif/else branches
        let mut cursor = node.walk();
        let mut fallthrough_from = current;
        for child in node.children(&mut cursor) {
            match child.kind() {
                NK::ELIF_CLAUSE => {
                    let elif_test = self.cfg.new_block();
                    self.cfg.add_edge(fallthrough_from, elif_test, EdgeKind::CondFalse);
                    if let Some(cond) = child.child_by_field_name("condition") {
                        self.collect_reads(cond, elif_test);
                    }
                    if let Some(body) = child.child_by_field_name("consequence") {
                        let branch = self.cfg.new_block();
                        self.cfg.add_edge(elif_test, branch, EdgeKind::CondTrue);
                        let after = self.walk_children(body, branch);
                        if !self.cfg.blocks[after].terminates {
                            self.cfg.add_edge(after, merge, EdgeKind::Unconditional);
                            all_branches_terminate = false;
                        }
                    } else {
                        all_branches_terminate = false;
                    }
                    fallthrough_from = elif_test;
                }
                NK::ELSE_CLAUSE => {
                    has_else = true;
                    let else_block = self.cfg.new_block();
                    self.cfg.add_edge(fallthrough_from, else_block, EdgeKind::CondFalse);
                    // else_clause uses field "body"
                    if let Some(body) = child.child_by_field_name("body") {
                        let after_else = self.walk_children(body, else_block);
                        if !self.cfg.blocks[after_else].terminates {
                            self.cfg.add_edge(after_else, merge, EdgeKind::Unconditional);
                            all_branches_terminate = false;
                        }
                    } else {
                        all_branches_terminate = false;
                    }
                    fallthrough_from = else_block;
                }
                _ => {}
            }
        }

        // If no else, the false path falls through to merge.
        if !has_else {
            self.cfg.add_edge(fallthrough_from, merge, EdgeKind::CondFalse);
            all_branches_terminate = false;
        }

        if all_branches_terminate {
            self.cfg.blocks[merge].terminates = true;
            self.cfg.blocks[merge].merge_terminates = true;
        }

        merge
    }

    fn walk_while(&mut self, node: Node, current: usize) -> usize {
        let header = self.cfg.new_block();
        let body_block = self.cfg.new_block();
        let exit = self.cfg.new_block();

        self.cfg.add_edge(current, header, EdgeKind::Unconditional);

        // while_stmt: 'while' expression block  (positional children)
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        // child 0 = "while" keyword, child 1 = condition, child 2 = block
        if let Some(&cond) = children.get(1) {
            if cond.kind() != NK::BLOCK {
                self.collect_reads(cond, header);
            }
        }
        self.cfg.add_edge(header, body_block, EdgeKind::CondTrue);
        self.cfg.add_edge(header, exit, EdgeKind::CondFalse);

        self.loop_stack.push((header, exit));
        if let Some(body) = children.iter().find(|c| c.kind() == NK::BLOCK) {
            let after_body = self.walk_children(*body, body_block);
            if !self.cfg.blocks[after_body].terminates {
                self.cfg.add_edge(after_body, header, EdgeKind::LoopBack);
            }
        }
        self.loop_stack.pop();

        exit
    }

    fn walk_for(&mut self, node: Node, current: usize) -> usize {
        let header = self.cfg.new_block();
        let body_block = self.cfg.new_block();
        let exit = self.cfg.new_block();

        // for_stmt: 'for' unpack_target 'in' expression block  (positional)
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();

        // Collect iterable reads (the expression after 'in') and loop var defs.
        let mut found_in = false;
        for &child in &children {
            if child.kind() == NK::BLOCK {
                break;
            }
            if child.is_named() && !found_in {
                if child.kind() == NK::UNPACK_TARGET {
                    self.collect_lvalue_defs(child, header);
                }
            }
            if !child.is_named() && self.node_text(child) == "in" {
                found_in = true;
                continue;
            }
            if found_in && child.kind() != NK::BLOCK {
                self.collect_reads(child, current);
            }
        }

        self.cfg.add_edge(current, header, EdgeKind::Unconditional);
        self.cfg.add_edge(header, body_block, EdgeKind::CondTrue);
        self.cfg.add_edge(header, exit, EdgeKind::CondFalse);

        self.loop_stack.push((header, exit));
        if let Some(body) = children.iter().find(|c| c.kind() == NK::BLOCK) {
            let after_body = self.walk_children(*body, body_block);
            if !self.cfg.blocks[after_body].terminates {
                self.cfg.add_edge(after_body, header, EdgeKind::LoopBack);
            }
        }
        self.loop_stack.pop();

        exit
    }

    fn walk_match(&mut self, node: Node, current: usize) -> usize {
        let merge = self.cfg.new_block();

        // Subject reads (field "value" in grammar).
        if let Some(subj) = node.child_by_field_name("value") {
            self.collect_reads(subj, current);
        }

        let mut outer_cursor = node.walk();
        let mut has_wildcard = false;
        let mut all_branches_terminate = true;

        for child in node.children(&mut outer_cursor) {
            if child.kind() != NK::MATCH_CASE {
                continue;
            }
            let case_block = self.cfg.new_block();
            self.cfg.add_edge(current, case_block, EdgeKind::Unconditional);

            // A guarded wildcard (`_ if cond`) is not exhaustive.
            let case_has_guard = child.child_by_field_name("guard").is_some();

            // match_case: pattern [if guard] '=>' block  (positional)
            let mut case_cursor = child.walk();
            for case_child in child.children(&mut case_cursor) {
                match case_child.kind() {
                    NK::PATTERN
                    | NK::PATTERN_OR
                    | NK::PATTERN_VAR
                    | NK::PATTERN_WILDCARD
                    | NK::PATTERN_LITERAL
                    | NK::PATTERN_TUPLE
                    | NK::PATTERN_STRUCT
                    | NK::PATTERN_STAR => {
                        self.collect_pattern_defs(case_child, case_block);
                        if !case_has_guard && self.is_wildcard_pattern(case_child) {
                            has_wildcard = true;
                        }
                    }
                    NK::BLOCK => {
                        let after = self.walk_children(case_child, case_block);
                        if !self.cfg.blocks[after].terminates {
                            self.cfg.add_edge(after, merge, EdgeKind::Unconditional);
                            all_branches_terminate = false;
                        }
                    }
                    _ => {}
                }
            }

            // Guard reads.
            if let Some(guard) = child.child_by_field_name("guard") {
                self.collect_reads(guard, case_block);
            }
        }

        // If no wildcard, there's an implicit fallthrough path.
        if !has_wildcard {
            self.cfg.add_edge(current, merge, EdgeKind::Unconditional);
            all_branches_terminate = false;
        }

        if all_branches_terminate {
            self.cfg.blocks[merge].terminates = true;
            self.cfg.blocks[merge].merge_terminates = true;
        }

        merge
    }

    fn walk_try(&mut self, node: Node, current: usize) -> usize {
        let merge = self.cfg.new_block();
        let mut all_terminate = true;

        // Try body (field "body")
        let try_block = self.cfg.new_block();
        self.cfg.add_edge(current, try_block, EdgeKind::Unconditional);

        if let Some(body) = node.child_by_field_name("body") {
            let after_try = self.walk_children(body, try_block);
            if !self.cfg.blocks[after_try].terminates {
                self.cfg.add_edge(after_try, merge, EdgeKind::Unconditional);
                all_terminate = false;
            }
        }

        // except_block contains except_clause children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == NK::EXCEPT_BLOCK {
                // Walk each except_clause inside the except_block.
                let mut ec_cursor = child.walk();
                for ec in child.children(&mut ec_cursor) {
                    if ec.kind() == NK::EXCEPT_CLAUSE {
                        let except_block = self.cfg.new_block();
                        // G1.1: exception can occur before any try body def,
                        // so except sees only pre-try defs.
                        self.cfg.add_edge(current, except_block, EdgeKind::Exception);
                        // Track the except binding (e.g. `e` in `except e: Error => { ... }`).
                        if let Some(binding) = ec.child_by_field_name("binding") {
                            if binding.kind() == NK::IDENTIFIER {
                                let name = self.node_text(binding);
                                self.cfg.blocks[except_block]
                                    .defs
                                    .entry(name)
                                    .or_insert(binding.start_byte());
                            }
                        }
                        // except_clause has field "handler" for the body block.
                        if let Some(handler) = ec.child_by_field_name("handler") {
                            let after_except = self.walk_children(handler, except_block);
                            if !self.cfg.blocks[after_except].terminates {
                                self.cfg.add_edge(after_except, merge, EdgeKind::Unconditional);
                                all_terminate = false;
                            }
                        } else {
                            all_terminate = false;
                        }
                    }
                }
            } else if child.kind() == NK::FINALLY_CLAUSE {
                // Finally always executes -- sequential after merge.
                if let Some(body) = child.child_by_field_name("body") {
                    let finally_block = self.cfg.new_block();
                    self.cfg.add_edge(merge, finally_block, EdgeKind::Unconditional);
                    let after_finally = self.walk_children(body, finally_block);
                    let new_merge = self.cfg.new_block();
                    if !self.cfg.blocks[after_finally].terminates {
                        self.cfg.add_edge(after_finally, new_merge, EdgeKind::Unconditional);
                    }
                    // Propagate termination through finally: if all try/except
                    // paths terminate (deferred return), code after the
                    // try/finally is unreachable even though finally ran.
                    if all_terminate && !self.cfg.blocks[after_finally].terminates {
                        self.cfg.blocks[new_merge].terminates = true;
                        self.cfg.blocks[new_merge].merge_terminates = true;
                    }
                    return new_merge;
                }
            }
        }

        if all_terminate {
            self.cfg.blocks[merge].terminates = true;
            self.cfg.blocks[merge].merge_terminates = true;
        }

        merge
    }

    // --- Variable collection helpers ---

    /// Collect all identifier reads in a subtree, skipping nested lambdas.
    fn collect_reads(&mut self, node: Node, block: usize) {
        if node.kind() == NK::LAMBDA_EXPR || node.kind() == NK::STRUCT_STMT || node.kind() == NK::TRAIT_STMT {
            return; // Don't descend into nested scopes.
        }
        if node.kind() == NK::IDENTIFIER {
            let name = self.node_text(node);
            let line = node.start_position().row + 1;
            let col = node.start_position().column + 1;
            let byte_offset = node.start_byte();
            self.cfg.blocks[block].reads.push((name, line, col, byte_offset));
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_reads(child, block);
        }
    }

    /// Collect defs from assignment LHS.
    fn collect_lvalue_defs(&mut self, node: Node, block: usize) {
        match node.kind() {
            NK::IDENTIFIER => {
                let name = self.node_text(node);
                self.cfg.blocks[block].defs.entry(name).or_insert(node.start_byte());
            }
            NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_lvalue_defs(child, block);
                }
            }
            NK::SETATTR | NK::INDEX => {} // attribute/index assignment, not a local def
            _ => {}
        }
    }

    /// Handle an assignment node: reads from RHS first, then defs from LHS.
    /// Grammar: assignment = decorator* lvalue ('=' lvalue)* '=' expression
    /// No field names -- positional children.
    fn collect_assignment(&mut self, node: Node, block: usize) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        // Last named child is the RHS expression, everything before the last '='
        // is LHS. Walk backwards to find the RHS.
        let mut lvalues = Vec::new();
        let mut rhs = None;
        let mut found_last_eq = false;
        for child in children.iter().rev() {
            if !found_last_eq {
                if !child.is_named() && self.node_text(*child) == "=" {
                    found_last_eq = true;
                } else if child.is_named() {
                    rhs = Some(*child);
                }
            } else if child.kind() == NK::LVALUE || child.kind() == NK::UNPACK_TARGET || child.kind() == NK::IDENTIFIER
            {
                lvalues.push(*child);
            }
        }
        // RHS reads first.
        if let Some(rhs_node) = rhs {
            self.collect_reads(rhs_node, block);
        }
        // LHS defs.
        for lv in &lvalues {
            self.collect_lvalue_defs(*lv, block);
        }
    }

    /// Collect defs from top-level lambda/struct/trait (just the name).
    fn collect_toplevel_def(&mut self, node: Node, block: usize) {
        if let Some(name_node) = node.child_by_field_name("name") {
            if name_node.kind() == NK::IDENTIFIER {
                let name = self.node_text(name_node);
                self.cfg.blocks[block]
                    .defs
                    .entry(name)
                    .or_insert(name_node.start_byte());
            }
        }
    }

    /// Collect variable bindings from match patterns.
    fn collect_pattern_defs(&mut self, node: Node, block: usize) {
        match node.kind() {
            NK::PATTERN_VAR => {
                if let Some(id) = node.child(0) {
                    if id.kind() == NK::IDENTIFIER {
                        let name = self.node_text(id);
                        self.cfg.blocks[block].defs.entry(name).or_insert(id.start_byte());
                    }
                }
            }
            NK::IDENTIFIER => {
                // Struct pattern fields are bare identifiers.
                let name = self.node_text(node);
                // Skip the struct type name (first identifier in pattern_struct).
                // Type names start with uppercase by convention.
                if !name.chars().next().is_some_and(|c| c.is_uppercase()) && name != "_" {
                    self.cfg.blocks[block].defs.entry(name).or_insert(node.start_byte());
                }
            }
            // All container pattern types: descend recursively.
            NK::PATTERN | NK::PATTERN_OR | NK::PATTERN_TUPLE | NK::PATTERN_STRUCT => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.collect_pattern_defs(child, block);
                }
            }
            _ => {
                // Catch pattern_items and other wrappers.
                if node.is_named() && node.kind().starts_with("pattern") {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.collect_pattern_defs(child, block);
                    }
                }
            }
        }
    }

    fn is_wildcard_pattern(&self, node: Node) -> bool {
        match node.kind() {
            NK::PATTERN_WILDCARD => true,
            NK::PATTERN_VAR => {
                // A pattern_var with name "_" is also a wildcard.
                node.child(0)
                    .map(|id| id.kind() == NK::IDENTIFIER && self.node_text(id) == "_")
                    .unwrap_or(false)
            }
            // PATTERN and PATTERN_OR are wrappers -- recurse.
            NK::PATTERN | NK::PATTERN_OR => {
                let mut cursor = node.walk();
                let children: Vec<_> = node.children(&mut cursor).collect();
                children.iter().any(|child| self.is_wildcard_pattern(*child))
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// W310 -- variable possibly uninitialized
// ---------------------------------------------------------------------------

// GENERATED FROM catnip/context.py - do not edit manually.
// Run: python catnip_tools/gen_builtins.py
// @generated-cfg-builtins-start
const BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AttributeError",
    "Exception",
    "False",
    "IndexError",
    "KeyError",
    "LookupError",
    "META",
    "MemoryError",
    "ND",
    "NameError",
    "None",
    "RUNTIME",
    "RuntimeError",
    "True",
    "TypeError",
    "ValueError",
    "ZeroDivisionError",
    "_cache",
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "cached",
    "callable",
    "chr",
    "classmethod",
    "complex",
    "debug",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "fold",
    "format",
    "freeze",
    "frozenset",
    "getattr",
    "hasattr",
    "hash",
    "hex",
    "id",
    "import",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "jit",
    "len",
    "list",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "pragma",
    "print",
    "property",
    "pure",
    "range",
    "reduce",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "thaw",
    "tuple",
    "typeof",
    "vars",
    "zip",
];
// @generated-cfg-builtins-end

/// Forward dataflow: compute the set of variables that are *definitely*
/// defined on all paths reaching each block.
fn compute_definitely_defined(cfg: &LintCFG) -> Vec<HashSet<String>> {
    let n = cfg.blocks.len();
    // Initialize: entry has empty defs, everything else starts as "all" (top).
    // We use Option<HashSet> where None means "all" (top/unrestricted).
    let mut def_in: Vec<Option<HashSet<String>>> = vec![None; n];
    def_in[cfg.entry] = Some(HashSet::new());

    // Iterative fixpoint with iteration guard.
    let max_iterations = n * 2 + 10;
    let mut iterations = 0;
    let mut changed = true;
    while changed {
        iterations += 1;
        assert!(
            iterations <= max_iterations,
            "fixpoint did not converge after {} iterations ({} blocks)",
            iterations,
            n,
        );
        changed = false;
        for id in 0..n {
            if id == cfg.exit {
                continue;
            }
            // Meet: intersection of all predecessors' def_out.
            let preds = &cfg.blocks[id].predecessors;
            let meet = if preds.is_empty() {
                if id == cfg.entry {
                    Some(HashSet::new())
                } else {
                    None // unreachable block
                }
            } else {
                let mut result: Option<HashSet<String>> = None;
                for &pred in preds {
                    let pred_out = match &def_in[pred] {
                        Some(s) => {
                            let mut out = s.clone();
                            out.extend(cfg.blocks[pred].defs.keys().cloned());
                            out
                        }
                        None => continue, // pred not yet reached
                    };
                    result = Some(match result {
                        Some(r) => r.intersection(&pred_out).cloned().collect(),
                        None => pred_out,
                    });
                }
                result
            };

            if meet != def_in[id] {
                def_in[id] = meet;
                changed = true;
            }
        }
    }

    // Convert to concrete sets (None -> empty for safety).
    def_in.into_iter().map(|opt| opt.unwrap_or_default()).collect()
}

/// Check for possibly uninitialized variable reads.
fn check_possibly_uninitialized(cfg: &LintCFG, source_lines: &[&str]) -> Vec<Diagnostic> {
    let def_in = compute_definitely_defined(cfg);
    let builtins: HashSet<&str> = BUILTINS.iter().copied().collect();
    // Pre-compute all variable names defined anywhere in the CFG (O(1) lookup).
    let defined_anywhere: HashSet<&str> = cfg
        .blocks
        .iter()
        .flat_map(|b| b.defs.keys().map(|s| s.as_str()))
        .collect();
    let mut diagnostics = Vec::new();
    let mut reported: HashSet<String> = HashSet::new();

    for block in &cfg.blocks {
        if block.id == cfg.exit {
            continue;
        }
        let available = &def_in[block.id];
        for (name, line, col, byte_offset) in &block.reads {
            if builtins.contains(name.as_str()) {
                continue;
            }
            if available.contains(name) {
                continue;
            }
            // G1.3: check if a def in the same block precedes this read.
            if let Some(&def_pos) = block.defs.get(name) {
                if def_pos < *byte_offset {
                    continue;
                }
            }
            // If it's never defined anywhere, it's likely a global or E200 territory.
            if !defined_anywhere.contains(name.as_str()) {
                continue; // Not our problem -- E200 handles truly undefined names.
            }
            // Avoid duplicate reports for same variable.
            if !reported.insert(name.clone()) {
                continue;
            }
            let source_line = source_lines.get(line.saturating_sub(1)).map(|s| s.to_string());
            diagnostics.push(Diagnostic {
                code: "W310".to_string(),
                message: format!("'{}' may be uninitialized", name),
                severity: Severity::Warning,
                line: *line,
                column: *col,
                end_line: None,
                end_column: None,
                source_line,
                suggestion: None,
            });
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run deep CFG-based analysis on parsed source.
pub fn check_deep(root: Node, source: &str, _config: &LintConfig) -> Vec<Diagnostic> {
    let source_lines: Vec<&str> = source.lines().collect();
    let builder = CfgBuilder::new(source);
    let cfg = builder.build(root);
    let mut diagnostics = check_possibly_uninitialized(&cfg, &source_lines);

    // W311: unreachable code after terminating branches.
    for &(line, col) in &cfg.dead_code {
        let source_line = source_lines.get(line.saturating_sub(1)).map(|s| s.to_string());
        diagnostics.push(Diagnostic {
            code: "W311".to_string(),
            message: "unreachable code".to_string(),
            severity: Severity::Warning,
            line,
            column: col,
            end_line: None,
            end_column: None,
            source_line,
            suggestion: None,
        });
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&catnip_grammar::get_language()).unwrap();
        parser.parse(source, None).unwrap()
    }

    fn lint_deep(source: &str) -> Vec<Diagnostic> {
        let tree = parse(source);
        let config = LintConfig::default();
        check_deep(tree.root_node(), source, &config)
    }

    fn codes(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.code.as_str()).collect()
    }

    fn names(diags: &[Diagnostic]) -> Vec<String> {
        diags
            .iter()
            .map(|d| {
                // Extract variable name from "'name' may be uninitialized"
                d.message.split('\'').nth(1).unwrap_or("").to_string()
            })
            .collect()
    }

    // -- W310 positive cases --

    #[test]
    fn test_w310_if_without_else() {
        let diags = lint_deep("if cond {\n    x = 1\n}\nprint(x)");
        assert_eq!(codes(&diags), vec!["W310"]);
        assert_eq!(names(&diags), vec!["x"]);
    }

    #[test]
    fn test_w310_elif_missing_branch() {
        let diags = lint_deep("if a {\n    x = 1\n} elif b {\n    x = 2\n}\nprint(x)");
        assert_eq!(codes(&diags), vec!["W310"]);
    }

    // -- W310 negative cases --

    #[test]
    fn test_w310_if_else_both_define() {
        let diags = lint_deep("if cond {\n    x = 1\n} else {\n    x = 2\n}\nprint(x)");
        assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
    }

    #[test]
    fn test_w310_defined_before_if() {
        let diags = lint_deep("x = 0\nif cond {\n    x = 1\n}\nprint(x)");
        assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
    }

    #[test]
    fn test_w310_no_warning_for_undefined() {
        // Variable never defined anywhere -> E200 territory, not W310.
        let diags = lint_deep("print(x)");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_w310_no_warning_for_builtins() {
        let diags = lint_deep("if cond {\n    x = 1\n}\nprint(len(x))");
        // Only x should trigger, not len or print.
        let ns = names(&diags);
        assert_eq!(ns, vec!["x"]);
    }

    #[test]
    fn test_w310_while_loop() {
        let diags = lint_deep("while cond {\n    x = 1\n}\nprint(x)");
        // x is defined inside loop body which may not execute.
        assert_eq!(codes(&diags), vec!["W310"]);
    }

    #[test]
    fn test_w310_for_loop_variable_safe() {
        // For loop variable is defined in the header, but the loop may not execute.
        // However, reads of the iterable are fine.
        let diags = lint_deep("items = [1, 2, 3]\nfor x in items {\n    print(x)\n}");
        assert!(diags.is_empty(), "expected no W310, got: {:?}", diags);
    }

    #[test]
    fn test_w310_except_binding_no_false_positive() {
        // The except binding variable should be tracked as a def.
        let diags = lint_deep("try {\n    x = 1\n} except {\n    e: Error => {\n        print(e)\n    }\n}");
        // `e` is bound by the except clause -- no W310 for it.
        let ns = names(&diags);
        assert!(
            !ns.contains(&"e".to_string()),
            "except binding 'e' should not trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_except_binding_after_handler() {
        // Reading the except binding after the try/except block.
        // `e` is only defined on the except path, so W310 is correct here.
        let diags = lint_deep("try {\n    x = 1\n} except {\n    e: Error => {\n        y = 1\n    }\n}\nprint(e)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"e".to_string()),
            "except binding outside handler should trigger W310, got: {:?}",
            ns
        );
    }

    // -- G4.1: try/except tests --

    #[test]
    fn test_w310_try_var_read_in_except() {
        // Variable defined in try only -- except may run before the def.
        let diags = lint_deep("try {\n    x = risky()\n} except {\n    _: Error => {\n        print(x)\n    }\n}");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined in try should be possibly uninitialized in except, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_try_except_both_define() {
        // Variable defined in both try and except -- safe after the block.
        let diags = lint_deep("try {\n    x = 1\n} except {\n    _: Error => {\n        x = 2\n    }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            !ns.contains(&"x".to_string()),
            "x defined in both try and except should be safe, got: {:?}",
            ns
        );
    }

    #[test]
    #[ignore = "finally false negative: modeling exception path requires block duplication"]
    fn test_w310_try_finally_var_from_try() {
        // Variable defined in try, read in finally -- may not be defined.
        let diags = lint_deep("try {\n    x = 1\n} finally {\n    print(x)\n}");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined in try should be possibly uninitialized in finally, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_try_finally_no_false_positive_after() {
        // Reaching code after try/finally means try completed -- no false W310.
        let diags = lint_deep("try {\n    x = 1\n} finally {\n    cleanup()\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            !ns.contains(&"x".to_string()),
            "x defined in try should be safe after try/finally, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_multiple_except_clauses() {
        // Variable defined in some except clauses but not all.
        let diags = lint_deep(
            "try {\n    risky()\n} except {\n    _: TypeError => {\n        x = 1\n    }\n    _: ValueError => {\n        y = 2\n    }\n}\nprint(x)",
        );
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined only in one except clause should trigger W310, got: {:?}",
            ns
        );
    }

    // -- G4.2: break/continue tests --

    #[test]
    fn test_w310_break_only_path() {
        // Variable defined only on the break path.
        let diags = lint_deep("while True {\n    if cond {\n        x = 1\n        break\n    }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined only in break path should trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    #[ignore = "dead code after continue: def never registered, needs W311 or dead-def collection"]
    fn test_w310_after_continue_unreachable() {
        // Variable defined after continue -- never reached.
        let diags = lint_deep("while cond {\n    continue\n    x = 1\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x after continue is unreachable, should trigger W310, got: {:?}",
            ns
        );
    }

    // -- G4.3: match tests --

    #[test]
    fn test_w310_match_all_cases_define() {
        // All cases (including wildcard) define x -- safe.
        let diags = lint_deep("val = 1\nmatch val {\n    1 => { x = 1 }\n    _ => { x = 2 }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            !ns.contains(&"x".to_string()),
            "x defined in all match cases should be safe, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_match_partial_cases() {
        // x defined in one case only, no wildcard.
        let diags = lint_deep("match val {\n    1 => { x = 1 }\n    2 => { y = 2 }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined in only one match case should trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_match_guarded_wildcard_not_exhaustive() {
        // A guarded wildcard is not exhaustive -- the guard may fail.
        let diags = lint_deep("val = 1\nmatch val {\n    1 => { x = 1 }\n    _ if cond => { x = 2 }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "guarded wildcard should not make match exhaustive, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_match_with_wildcard_defines() {
        // Wildcard case defines x but specific case doesn't.
        let diags = lint_deep("match val {\n    1 => { y = 1 }\n    _ => { x = 2 }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x not defined in all match cases should trigger W310, got: {:?}",
            ns
        );
    }

    // -- G1.3: intra-block ordering --

    #[test]
    fn test_w310_read_before_def_same_block() {
        // Read before def in the same block should trigger W310.
        let diags = lint_deep("print(x)\nx = 1");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "read before def in same block should trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_def_before_read_same_block() {
        // Def before read in the same block should NOT trigger W310.
        let diags = lint_deep("x = 1\nprint(x)");
        assert!(diags.is_empty(), "def before read should be safe, got: {:?}", diags);
    }

    // -- G4.4: nested control flow --

    #[test]
    fn test_w310_if_inside_while() {
        let diags = lint_deep("while cond {\n    if flag {\n        x = 1\n    }\n}\nprint(x)");
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined only in if-inside-while should trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_try_inside_if() {
        // x defined in try inside one if branch only.
        let diags = lint_deep(
            "if cond {\n    try {\n        x = 1\n    } except {\n        _: Error => { x = 2 }\n    }\n}\nprint(x)",
        );
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined inside if-without-else should trigger W310, got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_match_inside_for() {
        let diags = lint_deep(
            "items = [1]\nfor i in items {\n    match i {\n        1 => { x = 1 }\n        _ => { x = 2 }\n    }\n}\nprint(x)",
        );
        let ns = names(&diags);
        assert!(
            ns.contains(&"x".to_string()),
            "x defined only inside for body should trigger W310 (loop may not execute), got: {:?}",
            ns
        );
    }

    #[test]
    fn test_w310_return_in_nested_if() {
        // All branches of outer if either define x or return.
        let diags = lint_deep(
            "if a {\n    if b {\n        return 0\n    } else {\n        x = 1\n    }\n} else {\n    x = 2\n}\nprint(x)",
        );
        let ns = names(&diags);
        assert!(
            !ns.contains(&"x".to_string()),
            "x defined on all non-returning paths should be safe, got: {:?}",
            ns
        );
    }

    // -- G4.5: edge cases --

    #[test]
    fn test_w310_empty_source() {
        let diags = lint_deep("");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_w310_single_statement() {
        let diags = lint_deep("x = 1");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_w310_duplicate_read_single_alert() {
        // Variable read in two branches -- only one W310 alert expected.
        let diags = lint_deep("if cond {\n    x = 1\n}\nprint(x)\nprint(x)");
        let ns = names(&diags);
        assert_eq!(ns.len(), 1, "should only report x once, got: {:?}", ns);
    }

    #[test]
    fn test_w310_destructuring_assignment() {
        // Destructuring defines both variables.
        let diags = lint_deep("(a, b) = (1, 2)\nprint(a)\nprint(b)");
        assert!(diags.is_empty(), "destructured vars should be safe, got: {:?}", diags);
    }

    // -- G5.1: W311 unreachable code --

    #[test]
    fn test_w311_not_after_direct_return() {
        // Direct return is intra-block dead code (W300 territory), not W311.
        let diags = lint_deep("return 1\nprint(42)");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert!(
            w311.is_empty(),
            "direct return should not trigger W311 (W300 handles it): {:?}",
            w311
        );
    }

    #[test]
    fn test_w311_after_all_branches_return() {
        let diags = lint_deep("if cond {\n    return 1\n} else {\n    return 2\n}\nx = 3");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert_eq!(w311.len(), 1, "expected one W311, got: {:?}", w311);
        assert_eq!(w311[0].line, 6);
    }

    #[test]
    fn test_w311_no_false_positive_with_else() {
        let diags = lint_deep("if cond {\n    return 1\n}\nx = 3");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert!(
            w311.is_empty(),
            "only one branch returns, code is reachable: {:?}",
            w311
        );
    }

    #[test]
    fn test_w311_not_after_direct_raise() {
        // Direct raise is intra-block dead code (W300 territory), not W311.
        let diags = lint_deep("raise Error()\nx = 1");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert!(w311.is_empty(), "direct raise should not trigger W311: {:?}", w311);
    }

    #[test]
    fn test_w311_match_all_cases_return() {
        let diags = lint_deep("match val {\n    1 => { return 1 }\n    _ => { return 2 }\n}\nprint(42)");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert_eq!(w311.len(), 1, "all match cases return, code after is dead: {:?}", w311);
    }

    #[test]
    fn test_w311_through_finally() {
        // All try paths return, finally runs but deferred return makes code after unreachable.
        let diags = lint_deep("try {\n    return 1\n} finally {\n    cleanup()\n}\nprint(42)");
        let w311: Vec<_> = diags.iter().filter(|d| d.code == "W311").collect();
        assert_eq!(
            w311.len(),
            1,
            "code after try/finally with return should be dead: {:?}",
            w311
        );
    }
}

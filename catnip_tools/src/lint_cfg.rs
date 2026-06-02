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

#[derive(Debug, Clone)]
struct WriteRec {
    name: String,
    line: usize,
    column: usize,
    offset: usize,
    /// True when the write comes from an explicit assignment statement
    /// (e.g. `x = ...`, `(a, b) = ...`). False for implicit bindings
    /// (for-var, match-pattern var, except-binding, lambda/struct/trait
    /// name). Only explicit writes are W312 candidates.
    explicit: bool,
}

#[derive(Debug)]
struct LintBlock {
    id: usize,
    /// Variables definitely assigned in this block.
    /// Maps name -> byte offset of the first def (for intra-block ordering).
    defs: HashMap<String, usize>,
    /// Variables read in this block.
    reads: Vec<(String, usize, usize, usize)>, // (name, line, col, byte_offset)
    /// All writes in source order. May contain multiple writes to the same
    /// variable. Used by liveness analysis (W312).
    writes: Vec<WriteRec>,
    /// Names that appear as both LHS and (somewhere in) RHS of the same
    /// assignment in this block (`x = ... x ...`). Used by capture detection
    /// to distinguish capture mutation (self-referential) from a local
    /// shadow (`x = 2` with no read of x on the RHS).
    self_ref_writes: HashSet<String>,
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
            writes: Vec::new(),
            self_ref_writes: HashSet::new(),
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

    /// Record a write to `name` in `block`. `position_node` carries the
    /// diagnostic location (line/column); `order_offset` carries the
    /// dataflow ordering anchor -- for assignments this should be the END
    /// byte of the assignment so the write is sequenced after the RHS reads
    /// (e.g. `x = x + 1`).
    fn add_write(&mut self, block: usize, name: String, position_node: Node, order_offset: usize, explicit: bool) {
        let line = position_node.start_position().row + 1;
        let column = position_node.start_position().column + 1;
        self.cfg.blocks[block].defs.entry(name.clone()).or_insert(order_offset);
        self.cfg.blocks[block].writes.push(WriteRec {
            name,
            line,
            column,
            offset: order_offset,
            explicit,
        });
    }

    /// Build CFG with a list of parameter bindings seeded as implicit defs
    /// in the entry block. Used for lambda bodies so reads of parameters
    /// don't fire W310 and live-out analysis sees them as defined on every
    /// path reaching exit (a function can't enter without its arguments
    /// bound). Pass an empty slice for the top-level scope.
    fn build_with_params(mut self, root: Node, params: &[(String, Node)]) -> LintCFG {
        let entry = self.cfg.entry;
        for (name, node) in params {
            self.add_write(entry, name.clone(), *node, node.end_byte(), false);
        }
        let after = self.walk_children(root, entry);
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
        // Order the for-var bind after the iterable expression so reads
        // inside it precede the bind (e.g. shadowing via `for x in [x]`).
        let bind_order = children
            .iter()
            .find(|c| c.kind() == NK::BLOCK)
            .map(|b| b.start_byte())
            .unwrap_or_else(|| node.end_byte());

        // Collect iterable reads (the expression after 'in') and loop var defs.
        let mut found_in = false;
        for &child in &children {
            if child.kind() == NK::BLOCK {
                break;
            }
            if child.is_named() && !found_in {
                if child.kind() == NK::UNPACK_TARGET {
                    self.collect_lvalue_defs(child, header, false, bind_order);
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
                        // Pattern bindings sequence at the END of the pattern
                        // node (semantic order: pattern matches first, binds,
                        // then guard runs, then body). End-of-pattern keeps
                        // body reads ordered AFTER the binding, which matters
                        // for capture detection so pattern-bound vars are not
                        // misclassified as captures in nested scopes.
                        self.collect_pattern_defs(case_child, case_block, case_child.end_byte());
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
                                let order = binding.end_byte();
                                self.add_write(except_block, name, binding, order, false);
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

    /// Collect all identifier reads in a subtree, skipping nested scopes.
    /// Inside `getattr` / `callattr` the inner identifier is an attribute
    /// or method **name**, not a variable read -- skip it. The host of the
    /// attribute access lives as a sibling in the surrounding expression
    /// and is still picked up by the normal recurse.
    fn collect_reads(&mut self, node: Node, block: usize) {
        if node.kind() == NK::LAMBDA_EXPR
            || node.kind() == NK::STRUCT_STMT
            || node.kind() == NK::TRAIT_STMT
            || node.kind() == NK::STRUCT_METHOD
        {
            return; // Don't descend into nested scopes.
        }
        if node.kind() == NK::GETATTR {
            // Pure attribute name lookup -- no variable reads inside.
            return;
        }
        if node.kind() == NK::CALLATTR {
            // `.method(args)` -- skip the method identifier, but the
            // arguments may contain variable reads.
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            for child in children {
                if child.kind() == NK::IDENTIFIER {
                    continue;
                }
                self.collect_reads(child, block);
            }
            return;
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
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        for child in children {
            self.collect_reads(child, block);
        }
    }

    /// Collect defs from assignment LHS. `explicit` is true for source-level
    /// `x = ...` (W312 candidate), false for loop-var / pattern bindings.
    /// `order_offset` is the byte position used to sequence this write
    /// relative to reads in the same block (use the END byte of the
    /// enclosing assignment for `x = x + 1`-style cases).
    fn collect_lvalue_defs(&mut self, node: Node, block: usize, explicit: bool, order_offset: usize) {
        match node.kind() {
            NK::IDENTIFIER => {
                let name = self.node_text(node);
                self.add_write(block, name, node, order_offset, explicit);
            }
            NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
                let mut cursor = node.walk();
                let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
                for child in children {
                    self.collect_lvalue_defs(child, block, explicit, order_offset);
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
        // Self-referential check: for each LHS name, record whether the RHS
        // syntactically reads that same name. Used by capture detection to
        // separate `x = x + 1` (capture mutation) from `x = 1` (local shadow).
        let bytes = self.source.as_bytes();
        let mut rhs_names: HashSet<String> = HashSet::new();
        if let Some(rhs_node) = rhs {
            gather_read_idents(rhs_node, bytes, &mut rhs_names);
        }
        if !rhs_names.is_empty() {
            let mut lhs_names: HashSet<String> = HashSet::new();
            for lv in &lvalues {
                gather_lhs_idents_set(*lv, bytes, &mut lhs_names);
            }
            for name in lhs_names {
                if rhs_names.contains(&name) {
                    self.cfg.blocks[block].self_ref_writes.insert(name);
                }
            }
        }
        // LHS defs (explicit user assignment -- W312 candidates). The order
        // anchor is the END of the assignment so RHS reads come before the
        // LHS write (matters for `x = x + 1`).
        let order = node.end_byte();
        for lv in &lvalues {
            self.collect_lvalue_defs(*lv, block, true, order);
        }
    }

    /// Collect defs from top-level lambda/struct/trait (just the name).
    fn collect_toplevel_def(&mut self, node: Node, block: usize) {
        if let Some(name_node) = node.child_by_field_name("name") {
            if name_node.kind() == NK::IDENTIFIER {
                let name = self.node_text(name_node);
                // Named lambda/struct/trait: implicit so we don't surface
                // W312 when a function/type is reassigned without being
                // called (W200 covers the unused case). Order anchor is the
                // end of the whole definition node.
                self.add_write(block, name, name_node, node.end_byte(), false);
            }
        }
    }

    /// Collect variable bindings from match patterns. All pattern bindings
    /// are implicit (not W312 candidates). `order_offset` should be set so
    /// the binding sequences after any reads in this case.
    fn collect_pattern_defs(&mut self, node: Node, block: usize, order_offset: usize) {
        match node.kind() {
            NK::PATTERN_VAR => {
                if let Some(id) = node.child(0) {
                    if id.kind() == NK::IDENTIFIER {
                        let name = self.node_text(id);
                        self.add_write(block, name, id, order_offset, false);
                    }
                }
            }
            NK::IDENTIFIER => {
                // Struct pattern fields are bare identifiers.
                let name = self.node_text(node);
                // Skip the struct type name (first identifier in pattern_struct).
                // Type names start with uppercase by convention.
                if !name.chars().next().is_some_and(|c| c.is_uppercase()) && name != "_" {
                    self.add_write(block, name, node, order_offset, false);
                }
            }
            // Container pattern kinds: descend recursively. PATTERN_LITERAL
            // and PATTERN_WILDCARD have no bindings (no-op).
            NK::PATTERN
            | NK::PATTERN_OR
            | NK::PATTERN_TUPLE
            | NK::PATTERN_ITEMS
            | NK::PATTERN_STRUCT
            | NK::PATTERN_ENUM
            | NK::PATTERN_STAR => {
                let mut cursor = node.walk();
                let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
                for child in children {
                    self.collect_pattern_defs(child, block, order_offset);
                }
            }
            _ => {}
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
// W312 -- dead store (backward liveness)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum EventKind {
    Read,
    Write,
}

/// Build the ordered list of (read, write) events for a block, sorted by
/// source byte offset. Each entry carries the variable name plus, for writes,
/// a pointer back into `block.writes` so we can recover position metadata.
fn block_events(block: &LintBlock) -> Vec<(usize, EventKind, String, Option<usize>)> {
    let mut events: Vec<(usize, EventKind, String, Option<usize>)> = Vec::new();
    for (name, _line, _col, offset) in &block.reads {
        events.push((*offset, EventKind::Read, name.clone(), None));
    }
    for (idx, w) in block.writes.iter().enumerate() {
        events.push((w.offset, EventKind::Write, w.name.clone(), Some(idx)));
    }
    events.sort_by_key(|e| e.0);
    events
}

/// Compute per-block `use` (upward exposed reads) and `def` (any write).
fn compute_use_def(cfg: &LintCFG) -> (Vec<HashSet<String>>, Vec<HashSet<String>>) {
    let n = cfg.blocks.len();
    let mut use_b: Vec<HashSet<String>> = vec![HashSet::new(); n];
    let mut def_b: Vec<HashSet<String>> = vec![HashSet::new(); n];
    for (bi, block) in cfg.blocks.iter().enumerate() {
        let events = block_events(block);
        let mut killed: HashSet<String> = HashSet::new();
        for (_, kind, name, _) in &events {
            match kind {
                EventKind::Read => {
                    if !killed.contains(name) {
                        use_b[bi].insert(name.clone());
                    }
                }
                EventKind::Write => {
                    def_b[bi].insert(name.clone());
                    killed.insert(name.clone());
                }
            }
        }
    }
    (use_b, def_b)
}

/// Backward dataflow: compute `live_out` for each block.
/// `live_out[B] = ⋃ live_in[S]` over successors,
/// `live_in[B] = use[B] ∪ (live_out[B] \ def[B])`.
fn compute_live_out(cfg: &LintCFG) -> Vec<HashSet<String>> {
    let n = cfg.blocks.len();
    let (use_b, def_b) = compute_use_def(cfg);
    let mut live_in: Vec<HashSet<String>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<String>> = vec![HashSet::new(); n];

    let max_iterations = n * 4 + 10;
    let mut iterations = 0;
    let mut changed = true;
    while changed {
        iterations += 1;
        assert!(
            iterations <= max_iterations,
            "liveness fixpoint did not converge after {} iterations ({} blocks)",
            iterations,
            n,
        );
        changed = false;
        for bi in (0..n).rev() {
            let mut new_out: HashSet<String> = HashSet::new();
            for edge in &cfg.blocks[bi].successors {
                for v in &live_in[edge.target] {
                    new_out.insert(v.clone());
                }
            }
            if new_out != live_out[bi] {
                live_out[bi] = new_out;
                changed = true;
            }
            let mut new_in = use_b[bi].clone();
            for v in &live_out[bi] {
                if !def_b[bi].contains(v) {
                    new_in.insert(v.clone());
                }
            }
            if new_in != live_in[bi] {
                live_in[bi] = new_in;
                changed = true;
            }
        }
    }
    live_out
}

/// Report W312 (dead store) for every explicit write whose target is not
/// live afterwards. Walks each block's events in reverse to track the live
/// set at each program point.
/// Heuristic: in a sub-scope (lambda or method body), classify a variable
/// as captured from an enclosing scope when ALL of:
///   1. Its earliest event in source-byte order is a **read**.
///   2. The name is actually defined somewhere in a parent scope
///      (`parent_visible`).
///   3. EITHER the sub-scope never writes the name (pure read-only
///      capture), OR at least one write to that name is self-referential
///      (its RHS reads the same name -- the `x = x + 1` mutation pattern).
///
/// Condition (3) keeps shadowing patterns like `b = () => { print(x); x = 2 }`
/// out of the capture set: the write to `x` does not read `x`, so under
/// Python-style lexical scoping it locally shadows the outer `x`, and the
/// `print(x)` read is genuinely uninitialised -- W310 must fire.
fn detect_captures(cfg: &LintCFG, parent_visible: &HashSet<String>) -> HashSet<String> {
    let mut earliest: HashMap<String, (usize, bool)> = HashMap::new();
    for block in &cfg.blocks {
        for (name, _, _, offset) in &block.reads {
            let entry = earliest.entry(name.clone()).or_insert((*offset, true));
            if *offset < entry.0 {
                *entry = (*offset, true);
            }
        }
        for w in &block.writes {
            let entry = earliest.entry(w.name.clone()).or_insert((w.offset, false));
            if w.offset < entry.0 {
                *entry = (w.offset, false);
            }
        }
    }

    let any_explicit_write = |name: &str| -> bool {
        cfg.blocks
            .iter()
            .flat_map(|b| b.writes.iter())
            .any(|w| w.explicit && w.name == name)
    };
    let any_self_ref = |name: &str| -> bool { cfg.blocks.iter().any(|b| b.self_ref_writes.contains(name)) };

    earliest
        .into_iter()
        .filter_map(|(name, (_, is_read))| {
            if !is_read || !parent_visible.contains(&name) {
                return None;
            }
            if any_explicit_write(&name) && !any_self_ref(&name) {
                // Local shadow pattern (`x = 2` without RHS read of x) --
                // x is locally bound; don't suppress W310 on the prior read.
                return None;
            }
            Some(name)
        })
        .collect()
}

/// Walk up from `body` to the source root, accumulating the set of names
/// reachable from this lambda body through lexical scope: parameters of
/// every enclosing lambda + every binding (assignment LHS, for-var,
/// pattern, except, struct/trait name) found in sibling subtrees of any
/// enclosing scope. We never descend into other lambda bodies -- their
/// inner defs are not visible from us.
fn collect_parent_visible(body: Node, source: &str) -> HashSet<String> {
    let mut visible: HashSet<String> = HashSet::new();
    let bytes = source.as_bytes();
    let mut excluded = body;
    while let Some(parent) = excluded.parent() {
        if parent.kind() == NK::LAMBDA_EXPR || parent.kind() == NK::STRUCT_METHOD {
            // Crossing a function-scope boundary: add the function's params
            // (lambda lambda_params, method (self, ...)) to what's visible
            // from this body and from any deeper nested scope.
            for (name, _) in collect_lambda_param_names(parent, source) {
                visible.insert(name);
            }
        } else {
            collect_defs_in_scope(parent, excluded, bytes, &mut visible);
        }
        excluded = parent;
    }
    visible
}

fn collect_defs_in_scope(node: Node, exclude: Node, bytes: &[u8], visible: &mut HashSet<String>) {
    if node.id() == exclude.id() {
        return;
    }
    if node.kind() == NK::LAMBDA_EXPR || node.kind() == NK::STRUCT_METHOD {
        // Other function-scope body -- its inner bindings (locals, params)
        // aren't visible from outside.
        return;
    }
    match node.kind() {
        NK::ASSIGNMENT => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            let mut last_eq: Option<usize> = None;
            for (i, child) in children.iter().enumerate().rev() {
                if !child.is_named() && child.utf8_text(bytes).unwrap_or("") == "=" {
                    last_eq = Some(i);
                    break;
                }
            }
            if let Some(eq) = last_eq {
                for child in &children[..eq] {
                    collect_lhs_idents(*child, bytes, visible);
                }
            }
        }
        NK::FOR_STMT => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == NK::UNPACK_TARGET {
                    collect_lhs_idents(child, bytes, visible);
                    break;
                }
            }
        }
        NK::MATCH_CASE => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if matches!(
                    child.kind(),
                    NK::PATTERN
                        | NK::PATTERN_OR
                        | NK::PATTERN_VAR
                        | NK::PATTERN_WILDCARD
                        | NK::PATTERN_LITERAL
                        | NK::PATTERN_TUPLE
                        | NK::PATTERN_STRUCT
                        | NK::PATTERN_STAR
                ) {
                    collect_pattern_idents(child, bytes, visible);
                    break;
                }
            }
        }
        NK::EXCEPT_CLAUSE => {
            if let Some(binding) = node.child_by_field_name("binding") {
                if binding.kind() == NK::IDENTIFIER {
                    visible.insert(binding.utf8_text(bytes).unwrap_or("").to_string());
                }
            }
        }
        NK::STRUCT_STMT | NK::TRAIT_STMT => {
            if let Some(name) = node.child_by_field_name("name") {
                if name.kind() == NK::IDENTIFIER {
                    visible.insert(name.utf8_text(bytes).unwrap_or("").to_string());
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    for child in children {
        collect_defs_in_scope(child, exclude, bytes, visible);
    }
}

/// Walk a subtree and add every identifier **read** into `out`. Skip
/// nested function scopes (their reads belong to a different scope) and
/// attribute-name identifiers inside `getattr` / `callattr` (those are
/// attribute names, not variable references).
fn gather_read_idents(node: Node, bytes: &[u8], out: &mut HashSet<String>) {
    if node.kind() == NK::LAMBDA_EXPR
        || node.kind() == NK::STRUCT_STMT
        || node.kind() == NK::TRAIT_STMT
        || node.kind() == NK::STRUCT_METHOD
    {
        return;
    }
    if node.kind() == NK::GETATTR {
        return; // `.attribute` -- no variable reads inside.
    }
    if node.kind() == NK::CALLATTR {
        // `.method(args)` -- skip the method identifier, recurse into args.
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        for child in children {
            if child.kind() == NK::IDENTIFIER {
                continue;
            }
            gather_read_idents(child, bytes, out);
        }
        return;
    }
    if node.kind() == NK::IDENTIFIER {
        out.insert(node.utf8_text(bytes).unwrap_or("").to_string());
        return;
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    for child in children {
        gather_read_idents(child, bytes, out);
    }
}

/// Walk an LHS subtree and add LHS identifier names into `out`. Mirrors
/// `collect_lvalue_defs` but produces a `HashSet<String>` instead of CFG
/// writes -- used for self-referential write detection.
fn gather_lhs_idents_set(node: Node, bytes: &[u8], out: &mut HashSet<String>) {
    match node.kind() {
        NK::IDENTIFIER => {
            out.insert(node.utf8_text(bytes).unwrap_or("").to_string());
        }
        NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            for child in children {
                gather_lhs_idents_set(child, bytes, out);
            }
        }
        _ => {}
    }
}

fn collect_lhs_idents(node: Node, bytes: &[u8], visible: &mut HashSet<String>) {
    match node.kind() {
        NK::IDENTIFIER => {
            visible.insert(node.utf8_text(bytes).unwrap_or("").to_string());
        }
        NK::LVALUE | NK::UNPACK_TARGET | NK::UNPACK_TUPLE | NK::UNPACK_SEQUENCE | NK::UNPACK_ITEMS => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            for child in children {
                collect_lhs_idents(child, bytes, visible);
            }
        }
        _ => {}
    }
}

fn collect_pattern_idents(node: Node, bytes: &[u8], visible: &mut HashSet<String>) {
    match node.kind() {
        NK::PATTERN_VAR => {
            if let Some(id) = node.child(0) {
                if id.kind() == NK::IDENTIFIER {
                    visible.insert(id.utf8_text(bytes).unwrap_or("").to_string());
                }
            }
        }
        NK::IDENTIFIER => {
            let name = node.utf8_text(bytes).unwrap_or("");
            if !name.chars().next().is_some_and(|c| c.is_uppercase()) && name != "_" {
                visible.insert(name.to_string());
            }
        }
        NK::PATTERN
        | NK::PATTERN_OR
        | NK::PATTERN_TUPLE
        | NK::PATTERN_ITEMS
        | NK::PATTERN_STRUCT
        | NK::PATTERN_ENUM
        | NK::PATTERN_STAR => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            for child in children {
                collect_pattern_idents(child, bytes, visible);
            }
        }
        _ => {}
    }
}

/// Inject `captures` as implicit defs at the entry block of `cfg`. Used for
/// nested scopes so a read of a captured variable doesn't trip W310 and the
/// liveness analysis sees the value as defined on every path reaching exit.
/// We `insert()` with offset 0 (rather than `entry().or_insert()`) because
/// captures generally co-exist with a later self-mutating write in the same
/// block (`count = count + 1`), and the intra-block W310 ordering check
/// (`def_pos < read_byte_offset`) must see the capture as defined BEFORE
/// the RHS read.
fn seed_captures(cfg: &mut LintCFG, captures: &HashSet<String>) {
    let entry = cfg.entry;
    for name in captures {
        cfg.blocks[entry].defs.insert(name.clone(), 0);
    }
}

fn check_dead_stores(cfg: &LintCFG, source_lines: &[&str], skip: &HashSet<String>) -> Vec<Diagnostic> {
    let live_out_all = compute_live_out(cfg);
    // Variables that are read somewhere in the CFG. Writes to never-read
    // variables are W200 territory, not W312 -- skip them here to avoid
    // double-reporting.
    let read_anywhere: HashSet<&str> = cfg
        .blocks
        .iter()
        .flat_map(|b| b.reads.iter().map(|(name, _, _, _)| name.as_str()))
        .collect();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut reported: HashSet<(usize, usize)> = HashSet::new();

    for block in &cfg.blocks {
        if block.id == cfg.exit {
            continue;
        }
        if block.writes.is_empty() {
            continue;
        }
        let events = block_events(block);
        let mut live = live_out_all[block.id].clone();

        // Walk events in reverse to compute, at each write, the liveness
        // state immediately after that write.
        for (_, kind, name, idx) in events.iter().rev() {
            match kind {
                EventKind::Read => {
                    live.insert(name.clone());
                }
                EventKind::Write => {
                    let w = &block.writes[idx.expect("write events carry an index")];
                    if w.explicit
                        && !live.contains(&w.name)
                        && read_anywhere.contains(w.name.as_str())
                        && !skip.contains(&w.name)
                    {
                        let key = (w.line, w.column);
                        if reported.insert(key) {
                            let source_line = source_lines.get(w.line.saturating_sub(1)).map(|s| s.to_string());
                            diagnostics.push(Diagnostic {
                                code: "W312".to_string(),
                                message: format!("dead store: '{}' is overwritten before being read", w.name),
                                severity: Severity::Warning,
                                line: w.line,
                                column: w.column,
                                end_line: None,
                                end_column: None,
                                source_line,
                                suggestion: None,
                            });
                        }
                    }
                    live.remove(&w.name);
                }
            }
        }
    }
    diagnostics
}

// ---------------------------------------------------------------------------
// W313 -- redundant else after terminating branch (guard clause hint)
// ---------------------------------------------------------------------------

/// True if a block always exits its enclosing scope (return/raise) or its
/// enclosing loop (break/continue), through every path. Structural check on
/// the CST: looks at the last significant child, recursing into nested
/// if/match/try.
fn block_always_terminates(node: Node, source: &str) -> bool {
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).filter(|c| c.is_named()).collect();
    let Some(stmt) = children.last() else { return false };
    stmt_always_terminates(*stmt, source)
}

fn stmt_always_terminates(node: Node, source: &str) -> bool {
    match node.kind() {
        NK::RETURN_STMT | NK::RAISE_STMT | NK::BREAK_STMT | NK::CONTINUE_STMT => true,
        NK::STATEMENT => {
            // Wrapper -- check the inner child.
            let mut cursor = node.walk();
            let first_named: Option<Node<'_>> = node.children(&mut cursor).find(|c| c.is_named());
            first_named.is_some_and(|c| stmt_always_terminates(c, source))
        }
        NK::BLOCK => block_always_terminates(node, source),
        NK::IF_EXPR => {
            // All branches (then, elif*, else) must terminate AND an else
            // must exist (no fallthrough on missing else).
            let Some(cons) = node.child_by_field_name("consequence") else {
                return false;
            };
            if !block_always_terminates(cons, source) {
                return false;
            }
            let mut has_else = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    NK::ELIF_CLAUSE => {
                        let Some(body) = child.child_by_field_name("consequence") else {
                            return false;
                        };
                        if !block_always_terminates(body, source) {
                            return false;
                        }
                    }
                    NK::ELSE_CLAUSE => {
                        has_else = true;
                        let Some(body) = child.child_by_field_name("body") else {
                            return false;
                        };
                        if !block_always_terminates(body, source) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            has_else
        }
        NK::MATCH_EXPR => {
            // All match_case blocks must terminate AND a wildcard must exist.
            let mut has_wildcard = false;
            let mut all_terminate = true;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != NK::MATCH_CASE {
                    continue;
                }
                let has_guard = child.child_by_field_name("guard").is_some();
                let mut case_cursor = child.walk();
                let mut case_block: Option<Node> = None;
                for case_child in child.children(&mut case_cursor) {
                    match case_child.kind() {
                        NK::BLOCK => case_block = Some(case_child),
                        k if !has_guard && is_wildcard_pattern_kind(k, case_child, source) => {
                            has_wildcard = true;
                        }
                        _ => {}
                    }
                }
                if let Some(b) = case_block {
                    if !block_always_terminates(b, source) {
                        all_terminate = false;
                    }
                } else {
                    all_terminate = false;
                }
            }
            has_wildcard && all_terminate
        }
        _ => false,
    }
}

fn is_wildcard_pattern_kind(kind: &str, node: Node, source: &str) -> bool {
    match kind {
        NK::PATTERN_WILDCARD => true,
        NK::PATTERN_VAR => node
            .child(0)
            .map(|id| id.kind() == NK::IDENTIFIER && id.utf8_text(source.as_bytes()).unwrap_or("") == "_")
            .unwrap_or(false),
        NK::PATTERN | NK::PATTERN_OR => {
            let mut cursor = node.walk();
            let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
            children.iter().any(|c| is_wildcard_pattern_kind(c.kind(), *c, source))
        }
        _ => false,
    }
}

/// Walk the CST and report W313 hints: an `if` branch that always terminates
/// makes the following elif/else branches redundant -- they can be flattened
/// out as guard clauses.
fn check_redundant_else(root: Node, source: &str, diagnostics: &mut Vec<Diagnostic>) {
    let source_lines: Vec<&str> = source.lines().collect();
    walk_for_w313(root, source, &source_lines, diagnostics);
}

fn walk_for_w313(node: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    if node.kind() == NK::IF_EXPR {
        report_if_w313(node, source, source_lines, diagnostics);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_w313(child, source, source_lines, diagnostics);
    }
}

fn report_if_w313(node: Node, source: &str, source_lines: &[&str], diagnostics: &mut Vec<Diagnostic>) {
    // Collect branches in order: (label, body_block, position_node).
    // The position_node is where the diagnostic anchors (the `elif`/`else`
    // keyword or clause for follow-ups, the `if` keyword for the consequence).
    struct Branch<'a> {
        body: Node<'a>,
        head: Node<'a>,
        is_else: bool,
    }
    let mut branches: Vec<Branch<'_>> = Vec::new();
    if let Some(cons) = node.child_by_field_name("consequence") {
        branches.push(Branch {
            body: cons,
            head: node,
            is_else: false,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            NK::ELIF_CLAUSE => {
                if let Some(body) = child.child_by_field_name("consequence") {
                    branches.push(Branch {
                        body,
                        head: child,
                        is_else: false,
                    });
                }
            }
            NK::ELSE_CLAUSE => {
                if let Some(body) = child.child_by_field_name("body") {
                    branches.push(Branch {
                        body,
                        head: child,
                        is_else: true,
                    });
                }
            }
            _ => {}
        }
    }

    if branches.len() < 2 {
        return;
    }
    let terminates: Vec<bool> = branches
        .iter()
        .map(|b| block_always_terminates(b.body, source))
        .collect();

    let mut prior_terminates = false;
    let mut reported_once = false;
    for (i, branch) in branches.iter().enumerate() {
        if prior_terminates && !terminates[i] && !reported_once {
            // Report this branch as redundant.
            let pos = branch.head.start_position();
            let line = pos.row + 1;
            let col = pos.column + 1;
            let source_line = source_lines.get(line.saturating_sub(1)).map(|s| s.to_string());
            let message = if branch.is_else {
                "redundant else: previous branch always exits; flatten this block to the enclosing scope".to_string()
            } else {
                "redundant elif: previous branch always exits; this can be a top-level if".to_string()
            };
            diagnostics.push(Diagnostic {
                code: "W313".to_string(),
                message,
                severity: Severity::Hint,
                line,
                column: col,
                end_line: None,
                end_column: None,
                source_line,
                suggestion: None,
            });
            reported_once = true;
        }
        if terminates[i] {
            prior_terminates = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Walk the tree and yield each function-scope body (with its parameter
/// names) so every function gets its own CFG. A function scope here means
/// either a `lambda_expr` or a `struct_method` -- both introduce a fresh
/// lexical scope with parameters that the body references. The root scope
/// is handled separately by the caller (root cannot capture; sub-scopes
/// can). For `struct_method`, abstract declarations without a `=> { body }`
/// have nothing to analyse and are skipped.
fn collect_lambda_scopes<'a>(node: Node<'a>, source: &str, scopes: &mut Vec<(Node<'a>, Vec<(String, Node<'a>)>)>) {
    if node.kind() == NK::LAMBDA_EXPR || node.kind() == NK::STRUCT_METHOD {
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        if let Some(body) = children.iter().find(|c| c.kind() == NK::BLOCK) {
            let params = collect_lambda_param_names(node, source);
            scopes.push((*body, params));
        }
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
    for child in children {
        collect_lambda_scopes(child, source, scopes);
    }
}

/// Extract parameter names + name nodes from a `lambda_expr` or
/// `struct_method`. Returns `(name, identifier_node)` pairs in source order.
/// In Catnip, methods declare `self` explicitly in their parameter list, so
/// the same `lambda_params` walk works for both node kinds.
fn collect_lambda_param_names<'a>(func: Node<'a>, source: &str) -> Vec<(String, Node<'a>)> {
    let mut names = Vec::new();
    let mut cursor = func.walk();
    let children: Vec<Node<'_>> = func.children(&mut cursor).collect();
    let Some(params_node) = children.iter().find(|c| c.kind() == NK::LAMBDA_PARAMS) else {
        return names;
    };
    let mut pcursor = params_node.walk();
    for child in params_node.children(&mut pcursor) {
        if matches!(child.kind(), NK::LAMBDA_PARAM | NK::VARIADIC_PARAM) {
            if let Some(name_node) = child.child_by_field_name("name") {
                if name_node.kind() == NK::IDENTIFIER {
                    let text = name_node.utf8_text(source.as_bytes()).unwrap_or("").to_string();
                    names.push((text, name_node));
                }
            }
        }
    }
    names
}

/// Run deep CFG-based analysis on parsed source.
pub fn check_deep(root: Node, source: &str, _config: &LintConfig) -> Vec<Diagnostic> {
    let source_lines: Vec<&str> = source.lines().collect();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Root scope: no captures (top level can't capture from anywhere).
    {
        let builder = CfgBuilder::new(source);
        let cfg = builder.build_with_params(root, &[]);
        run_scope_checks(&cfg, &source_lines, &HashSet::new(), &mut diagnostics);
    }

    // Each lambda body is its own CFG. Parameters are seeded as implicit
    // defs in the entry block; captures (variables whose first event is a
    // read) are seeded too and excluded from W312.
    let mut lambda_scopes: Vec<(Node<'_>, Vec<(String, Node<'_>)>)> = Vec::new();
    collect_lambda_scopes(root, source, &mut lambda_scopes);
    for (body, params) in lambda_scopes {
        let parent_visible = collect_parent_visible(body, source);
        let builder = CfgBuilder::new(source);
        let mut cfg = builder.build_with_params(body, &params);
        let captures = detect_captures(&cfg, &parent_visible);
        seed_captures(&mut cfg, &captures);
        run_scope_checks(&cfg, &source_lines, &captures, &mut diagnostics);
    }

    // W313: structural check, walks the entire tree once.
    check_redundant_else(root, source, &mut diagnostics);

    diagnostics
}

fn run_scope_checks(
    cfg: &LintCFG,
    source_lines: &[&str],
    skip_w312: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.extend(check_possibly_uninitialized(cfg, source_lines));
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
    diagnostics.extend(check_dead_stores(cfg, source_lines, skip_w312));
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

    // -- G5.3: W313 redundant else after terminating branch --

    fn w313(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
        diags.iter().filter(|d| d.code == "W313").collect()
    }

    #[test]
    fn test_w313_else_after_return() {
        let diags = lint_deep(
            "f = () => {\n    if cond {\n        return 1\n    } else {\n        x = 2\n        do(x)\n    }\n}",
        );
        let hits = w313(&diags);
        assert_eq!(hits.len(), 1, "expected one W313, got: {:?}", hits);
        assert_eq!(hits[0].severity, Severity::Hint);
    }

    #[test]
    fn test_w313_else_after_raise() {
        let diags = lint_deep("if cond {\n    raise Error()\n} else {\n    do_stuff()\n}");
        let hits = w313(&diags);
        assert_eq!(hits.len(), 1, "raise should trigger W313 on else: {:?}", hits);
    }

    #[test]
    fn test_w313_both_branches_return_no_hint() {
        // Both branches terminate -- no W313 (symmetric, can't simplify by extraction).
        let diags = lint_deep("f = () => {\n    if cond {\n        return 1\n    } else {\n        return 2\n    }\n}");
        let hits = w313(&diags);
        assert!(hits.is_empty(), "both-return should not trigger W313: {:?}", hits);
    }

    #[test]
    fn test_w313_elif_after_return() {
        // if X returns, elif Y has follow-up code -> redundant elif.
        let diags = lint_deep("f = () => {\n    if a {\n        return 1\n    } elif b {\n        x = 2\n    }\n}");
        let hits = w313(&diags);
        assert_eq!(hits.len(), 1, "elif after return should trigger W313: {:?}", hits);
    }

    #[test]
    fn test_w313_no_else_no_hint() {
        // No else branch -- nothing redundant.
        let diags = lint_deep("f = () => {\n    if cond {\n        return 1\n    }\n    x = 2\n}");
        let hits = w313(&diags);
        assert!(hits.is_empty(), "if without else shouldn't trigger W313: {:?}", hits);
    }

    #[test]
    fn test_w313_then_does_not_terminate() {
        // Then doesn't terminate -- else is not redundant.
        let diags = lint_deep("if cond {\n    x = 1\n} else {\n    x = 2\n}");
        let hits = w313(&diags);
        assert!(hits.is_empty(), "no terminating branch -> no W313: {:?}", hits);
    }

    #[test]
    fn test_w313_break_in_loop_triggers() {
        // break also terminates the branch w.r.t. the enclosing loop body.
        let diags = lint_deep("while cond {\n    if stop {\n        break\n    } else {\n        step()\n    }\n}");
        let hits = w313(&diags);
        assert_eq!(hits.len(), 1, "break should trigger W313 on else: {:?}", hits);
    }

    #[test]
    fn test_w313_nested_if_all_branches_terminate() {
        // Inner if where all branches return -> outer then terminates.
        let diags = lint_deep(
            "f = () => {\n    if a {\n        if b {\n            return 1\n        } else {\n            return 2\n        }\n    } else {\n        x = 1\n    }\n}",
        );
        let hits = w313(&diags);
        assert_eq!(
            hits.len(),
            1,
            "nested if all-return should propagate termination: {:?}",
            hits
        );
    }

    #[test]
    fn test_w313_only_else_terminates_no_hint() {
        // Only else terminates -- we don't propose flipping the condition (V1
        // restriction). No W313.
        let diags = lint_deep("if cond {\n    x = 1\n} else {\n    return 2\n}");
        let hits = w313(&diags);
        assert!(
            hits.is_empty(),
            "only-else terminates should not trigger W313: {:?}",
            hits
        );
    }

    // -- G5.2: W312 dead store (backward liveness) --

    fn w312(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
        diags.iter().filter(|d| d.code == "W312").collect()
    }

    #[test]
    fn test_w312_overwrite_before_read() {
        // x = 1 is killed by x = 2 before being read.
        let diags = lint_deep("x = 1\nx = 2\nprint(x)");
        let hits = w312(&diags);
        assert_eq!(hits.len(), 1, "expected one W312 on first write: {:?}", hits);
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn test_w312_no_alert_when_read_between() {
        let diags = lint_deep("x = 1\nprint(x)\nx = 2\nprint(x)");
        let hits = w312(&diags);
        assert!(hits.is_empty(), "intermediate read keeps store live: {:?}", hits);
    }

    #[test]
    fn test_w312_inter_branches_killed_after_merge() {
        // Both branches assign y, then y is unconditionally overwritten -- the
        // branch writes are dead stores.
        let diags = lint_deep("if cond {\n    y = 1\n} else {\n    y = 2\n}\ny = 3\nprint(y)");
        let hits = w312(&diags);
        let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
        assert!(
            lines.contains(&2) && lines.contains(&4),
            "expected W312 on both branch writes, got lines: {:?}",
            lines
        );
    }

    #[test]
    fn test_w312_no_alert_when_branch_used_after_merge() {
        // Reading y after the if keeps both branch writes live.
        let diags = lint_deep("if cond {\n    y = 1\n} else {\n    y = 2\n}\nprint(y)");
        let hits = w312(&diags);
        assert!(hits.is_empty(), "merged branches are live: {:?}", hits);
    }

    #[test]
    fn test_w312_no_alert_when_variable_never_read() {
        // x is never read anywhere -- W200 territory, not W312.
        let diags = lint_deep("x = 1");
        let hits = w312(&diags);
        assert!(hits.is_empty(), "never-read variable is W200, not W312: {:?}", hits);
    }

    #[test]
    fn test_w312_for_var_binding_not_flagged() {
        // Loop variables are implicit bindings -- don't trigger W312 even if
        // the body reassigns them.
        let diags = lint_deep("for x in items {\n    x = transform(x)\n    print(x)\n}");
        let hits = w312(&diags);
        // x is read inside the body, the binding rewrites are user-driven on
        // a captured value; we don't want to chase that here.
        assert!(hits.is_empty(), "for-var binding shouldn't trigger W312: {:?}", hits);
    }

    #[test]
    fn test_w312_dest_before_overwrite() {
        // Destructuring then immediate overwrite of one component.
        let diags = lint_deep("(a, b) = pair\na = 99\nprint(a)\nprint(b)");
        let hits = w312(&diags);
        let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
        assert!(
            lines.contains(&1),
            "destructured 'a' is killed by line-2 overwrite, got lines: {:?}",
            lines
        );
    }

    #[test]
    fn test_w312_inside_function_body() {
        // W312 fires inside a lambda body.
        let diags = lint_deep("f = (z) => {\n    x = 1\n    x = z + 1\n    return x\n}");
        let hits = w312(&diags);
        let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
        assert!(
            lines.contains(&2),
            "first write inside lambda should be flagged: {:?}",
            lines
        );
    }

    // -- Lambda parameter seeding (regression: bodies must see their params) --

    #[test]
    fn test_lambda_param_read_no_w310() {
        // `z` is a parameter -- reading it inside the body must not trigger
        // W310 even though the body has no def for `z`.
        let diags = lint_deep("f = (z) => {\n    print(z)\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(w310.is_empty(), "param read shouldn't trigger W310: {:?}", w310);
    }

    #[test]
    fn test_lambda_param_partial_reassign_no_w310() {
        // `z` is reassigned in one branch of an if but still has the param
        // value on the other path. No W310.
        let diags = lint_deep("f = (z) => {\n    if cond {\n        z = 1\n    }\n    print(z)\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "param partially reassigned should not trigger W310: {:?}",
            w310
        );
    }

    // -- Captured-variable mutation (regression: must not regress W310 nor W312) --

    #[test]
    fn test_capture_mutation_no_w310() {
        // `count` is bound in the parent scope; mutating it inside the lambda
        // must not trigger W310 even though the body has no def before the
        // RHS read.
        let diags = lint_deep("count = 0\ninc = () => {\n    count = count + 1\n}\nprint(count)");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(w310.is_empty(), "capture mutation shouldn't trigger W310: {:?}", w310);
    }

    #[test]
    fn test_capture_mutation_no_w312() {
        // The write to `count` is observable from the parent scope -- not a
        // dead store from the linter's POV.
        let diags = lint_deep("count = 0\ninc = () => {\n    count = count + 1\n}\nprint(count)");
        let w312: Vec<_> = diags.iter().filter(|d| d.code == "W312").collect();
        assert!(w312.is_empty(), "capture write isn't a dead store: {:?}", w312);
    }

    #[test]
    fn test_capture_pure_read_no_w310() {
        // Reading a captured variable inside a lambda (no write) is the most
        // common closure pattern.
        let diags = lint_deep("x = 10\nf = () => {\n    return x + 1\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(w310.is_empty(), "capture read shouldn't trigger W310: {:?}", w310);
    }

    #[test]
    fn test_nested_w310_still_fires_for_local() {
        // A truly partial def of a LOCAL var inside a lambda must still
        // trigger W310 -- capture detection is keyed on read-first events.
        let diags = lint_deep("f = () => {\n    if cond {\n        x = 1\n    }\n    print(x)\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "partial def of a local should still fire W310: {:?}",
            w310
        );
    }

    #[test]
    fn test_nested_read_before_def_fires_w310() {
        // Regression: a read-before-def inside a lambda whose name does NOT
        // exist in any enclosing scope must fire W310. The earlier heuristic
        // (read-first event = capture) silently suppressed this -- capture
        // classification must require an actual parent-scope binding.
        let diags = lint_deep("f = () => {\n    print(x)\n    x = 1\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "read-before-local-def inside a lambda must fire W310: {:?}",
            w310
        );
        assert_eq!(w310[0].line, 2);
    }

    #[test]
    fn test_nested_capture_visible_in_sibling_lambda_not_seeded() {
        // `x` is defined inside one lambda's body, NOT visible from a sibling
        // lambda's enclosing scope. The sibling reads `x` then writes it --
        // x is in defined_anywhere of the sibling's sub-CFG, so the
        // read-before-def must surface as W310 (not get swallowed by the
        // capture heuristic).
        let diags = lint_deep("a = () => { x = 1 }\nb = () => {\n    print(x)\n    x = 5\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "x defined in sibling lambda must still trip W310 in `b`: {:?}",
            w310
        );
    }

    #[test]
    fn test_nested_capture_from_outer_lambda_local() {
        // `local_x` is bound in the outer lambda's body; the inner lambda
        // legitimately captures it.
        let diags =
            lint_deep("outer = () => {\n    local_x = 1\n    inner = () => {\n        print(local_x)\n    }\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "inner lambda reading outer's local should not trip W310: {:?}",
            w310
        );
    }

    // -- struct_method scope (regression: methods get their own CFG and params) --

    #[test]
    fn test_method_body_param_seeded() {
        // A method's `self` parameter must not trigger W310 inside the body.
        let diags =
            lint_deep("struct Point {\n    x; y;\n\n    sum(self) => {\n        return self.x + self.y\n    }\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(w310.is_empty(), "method param `self` should be seeded: {:?}", w310);
    }

    #[test]
    fn test_method_local_read_before_def_w310() {
        // A real read-before-local-def inside a method must still fire W310.
        let diags = lint_deep("struct S {\n    f(self) => {\n        print(x)\n        x = 1\n    }\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "read-before-local-def inside a method must fire W310: {:?}",
            w310
        );
    }

    #[test]
    fn test_nested_lambda_in_method_sees_self() {
        // A lambda nested inside a method body should see the method's params
        // (`self`, plus declared params) as captures, not as undefined reads.
        let diags = lint_deep(
            "struct S {\n    f(self, k) => {\n        g = () => {\n            return self.x + k\n        }\n    }\n}",
        );
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "lambda inside method should capture self and k from parent method scope: {:?}",
            w310
        );
    }

    // -- Attribute-name handling (regression: getattr/callattr identifiers
    //    are not variable reads) --

    #[test]
    fn test_attribute_name_not_variable_read() {
        // `obj.x` reads `obj`, not a variable named `x`. With `x` defined
        // only after, an attribute access mustn't fabricate a W310 on `x`.
        let diags = lint_deep("obj = make()\nobj.x = 1\nx = 2\nprint(x)");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "attribute name `x` mustn't be tracked as a read of variable x: {:?}",
            w310
        );
    }

    #[test]
    fn test_method_call_attribute_name_not_variable_read() {
        // `obj.x(arg)` -- `x` is a method name, not a variable read.
        let diags = lint_deep("obj = make()\nobj.x(arg)\nx = 2\nprint(x)");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        // arg is undefined → E200's job, not W310. x is defined-after but
        // not read before its def (the attribute access doesn't count).
        let names: Vec<&str> = w310
            .iter()
            .map(|d| d.message.split('\'').nth(1).unwrap_or(""))
            .collect();
        assert!(
            !names.contains(&"x"),
            "method name shouldn't be tracked as read of variable x: {:?}",
            w310
        );
    }

    #[test]
    fn test_self_ref_detection_ignores_attribute_names() {
        // The shadow heuristic must NOT treat `self.x` in the RHS as a read
        // of `x`. In this method, the LHS `result` is locally bound; the
        // RHS `self.x + self.y` has no read of `result`. Therefore
        // `result = self.x + self.y` is NOT a self-referential write of
        // `result`. The prior read `print(result)` IS a real W310 (shadow
        // pattern).
        let diags = lint_deep(
            "result = 0\nstruct P {\n    x; y;\n    m(self) => {\n        print(result)\n        result = self.x + self.y\n    }\n}",
        );
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "shadow-style local write inside method must keep prior read flagged: {:?}",
            w310
        );
    }

    #[test]
    fn test_local_shadow_with_prior_read_fires_w310() {
        // `x` is a module-level binding, but the lambda writes `x = 2`
        // without reading `x` in the RHS -- Python-style scoping makes `x`
        // a local for the entire lambda, so the prior `print(x)` is a real
        // read-before-local-def. Must fire W310 (the capture filter must
        // NOT swallow this).
        let diags = lint_deep("x = 0\nb = () => {\n    print(x)\n    x = 2\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert_eq!(
            w310.len(),
            1,
            "shadow-style local write must keep prior read flagged: {:?}",
            w310
        );
        assert_eq!(w310[0].line, 3);
    }

    #[test]
    fn test_capture_mutation_with_explicit_read_in_rhs_no_w310() {
        // `x = x + 1` IS self-referential -- the W310 capture filter must
        // still suppress the read warning here even though there's an
        // explicit write to x.
        let diags = lint_deep("x = 0\nb = () => {\n    x = x + 1\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "self-referential write should keep capture classification: {:?}",
            w310
        );
    }

    #[test]
    fn test_method_capture_mutation_no_w310_no_w312() {
        // `count` is bound at module level; a method that mutates the capture
        // must not trigger W310 (RHS read) or W312 (LHS write).
        let diags = lint_deep("count = 0\nstruct Counter {\n    bump(self) => {\n        count = count + 1\n    }\n}");
        let codes: Vec<&str> = diags
            .iter()
            .filter(|d| d.code == "W310" || d.code == "W312")
            .map(|d| d.code.as_str())
            .collect();
        assert!(
            codes.is_empty(),
            "method capturing module-level var shouldn't fire W310/W312: {:?}",
            codes
        );
    }

    #[test]
    fn test_match_pattern_binding_not_capture() {
        // `x` is a pattern binding inside a lambda. The binding is sequenced
        // at the end of the PATTERN (not the case clause), so body reads
        // happen AFTER the binding -- so x is not misclassified as capture.
        // W310 isn't expected; W312 isn't expected either.
        let diags = lint_deep("f = (val) => {\n    match val {\n        x => { return x + 1 }\n    }\n}");
        let names: Vec<_> = diags.iter().filter(|d| d.code == "W310" || d.code == "W312").collect();
        assert!(
            names.is_empty(),
            "pattern binding should not surface W310/W312: {:?}",
            names
        );
    }

    #[test]
    fn test_lambda_param_variadic_seeded() {
        let diags = lint_deep("f = (*args) => {\n    print(args)\n}");
        let w310: Vec<_> = diags.iter().filter(|d| d.code == "W310").collect();
        assert!(
            w310.is_empty(),
            "variadic param read shouldn't trigger W310: {:?}",
            w310
        );
    }

    #[test]
    fn test_w312_self_overwrite_uses_rhs() {
        // x = x + 1 reads x in the RHS -- the previous write is live.
        let diags = lint_deep("x = 1\nx = x + 1\nprint(x)");
        let hits = w312(&diags);
        assert!(
            hits.is_empty(),
            "RHS-using overwrite keeps prior write live: {:?}",
            hits
        );
    }

    #[test]
    fn test_w312_match_all_cases_overwritten() {
        let diags = lint_deep("match v {\n    1 => { y = 1 }\n    _ => { y = 2 }\n}\ny = 9\nprint(y)");
        let hits = w312(&diags);
        let lines: Vec<usize> = hits.iter().map(|d| d.line).collect();
        assert!(
            lines.contains(&2) && lines.contains(&3),
            "both match arms write y but it's overwritten -- expected W312 on both, got: {:?}",
            lines
        );
    }

    #[test]
    fn test_w313_match_all_cases_terminate_in_branch() {
        // Then branch contains a match where all cases return + wildcard.
        let diags = lint_deep(
            "f = () => {\n    if cond {\n        match x {\n            1 => { return 1 }\n            _ => { return 2 }\n        }\n    } else {\n        y = 1\n    }\n}",
        );
        let hits = w313(&diags);
        assert_eq!(hits.len(), 1, "match-all-return should propagate: {:?}", hits);
    }
}

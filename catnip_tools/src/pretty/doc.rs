// FILE: catnip_tools/src/pretty/doc.rs
//! Document algebra for Wadler-Leijen pretty-printer.

/// Handle into the arena - lightweight copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Doc(pub(crate) u32);

/// Arena-allocated document nodes.
pub struct Arena {
    nodes: Vec<DocNode>,
}

/// A single node in the document tree.
pub enum DocNode {
    /// Empty document.
    Nil,
    /// Literal text (must not contain newlines).
    Text(String),
    /// Break mode: `\n` + indent. Flat mode: `" "`.
    Line,
    /// Break mode: `\n` + indent. Flat mode: `""`.
    #[allow(dead_code)]
    SoftLine,
    /// Always emits `\n` + indent.
    HardLine,
    /// Increase indent by `i` for the inner doc.
    Nest(i32, Doc),
    /// Try to render `inner` flat; if it doesn't fit, render broken.
    Group(Doc),
    /// Sequence of two docs.
    Concat(Doc, Doc),
    /// Choose between flat and broken rendering.
    #[allow(dead_code)]
    IfFlat { flat: Doc, broken: Doc },
    /// Opaque multi-line content (e.g. triple-quoted strings), emitted verbatim.
    Verbatim(String),
    /// Source line annotation for line_map construction.
    SourceLine(usize, Doc),
    /// Fill mode: pack items with separators, breaking only when needed.
    /// Each separator independently decides flat (`, `) or break (`,\n`)
    /// based on whether the next item fits on the current line.
    /// Stored as alternating [item, sep, item, sep, ..., item].
    Fill(Vec<Doc>),
}

impl Arena {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn alloc(&mut self, node: DocNode) -> Doc {
        let idx = self.nodes.len();
        self.nodes.push(node);
        Doc(idx as u32)
    }

    pub fn get(&self, doc: Doc) -> &DocNode {
        &self.nodes[doc.0 as usize]
    }

    // -- Primitives --

    pub fn nil(&mut self) -> Doc {
        self.alloc(DocNode::Nil)
    }

    pub fn text(&mut self, s: impl Into<String>) -> Doc {
        self.alloc(DocNode::Text(s.into()))
    }

    pub fn line(&mut self) -> Doc {
        self.alloc(DocNode::Line)
    }

    #[allow(dead_code)]
    pub fn softline(&mut self) -> Doc {
        self.alloc(DocNode::SoftLine)
    }

    pub fn hardline(&mut self) -> Doc {
        self.alloc(DocNode::HardLine)
    }

    pub fn nest(&mut self, indent: i32, doc: Doc) -> Doc {
        self.alloc(DocNode::Nest(indent, doc))
    }

    pub fn group(&mut self, doc: Doc) -> Doc {
        self.alloc(DocNode::Group(doc))
    }

    pub fn concat(&mut self, a: Doc, b: Doc) -> Doc {
        self.alloc(DocNode::Concat(a, b))
    }

    #[allow(dead_code)]
    pub fn if_flat(&mut self, flat: Doc, broken: Doc) -> Doc {
        self.alloc(DocNode::IfFlat { flat, broken })
    }

    pub fn verbatim(&mut self, s: impl Into<String>) -> Doc {
        self.alloc(DocNode::Verbatim(s.into()))
    }

    pub fn source_line(&mut self, line: usize, doc: Doc) -> Doc {
        self.alloc(DocNode::SourceLine(line, doc))
    }

    /// Fill: alternating [item, sep, item, sep, ..., item].
    pub fn fill(&mut self, parts: Vec<Doc>) -> Doc {
        self.alloc(DocNode::Fill(parts))
    }
}

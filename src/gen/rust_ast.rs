//! Typed Rust emission surface.
//!
//! Per ★★ TYPED EMISSION, generated Rust is built as a typed tree and
//! rendered once, never spliced together with `format!()`. This module is
//! the third sanctioned shape: a typed AST builder.
//!
//! It follows the `NixValue` precedent in `iac-forge/src/nix.rs` — a
//! target-language-specific value tree owning its own renderer — rather
//! than the `SExpr` precedent in `lava-api-forge/src/emit.rs`. `SExpr` is
//! the right surface for a homoiconic target, where every form is a list.
//! Rust is not homoiconic: it has distinct identifier, type, expression,
//! statement and item categories, and keeping those categories as separate
//! Rust types is exactly what makes a category error un-writable. An
//! `SExpr`-shaped surface would flatten all five into `List` and give that
//! guarantee away.
//!
//! # What this surface makes impossible
//!
//! - **A category error.** [`Expr`], [`TypeExpr`], [`Ident`] and [`Stmt`]
//!   are distinct types. Passing an identifier where an expression belongs,
//!   or an expression where a type belongs, does not compile. There is no
//!   `Raw(String)` variant anywhere in this module — no escape hatch back
//!   into untyped syntax.
//! - **An unescaped string literal.** [`StrLit`] holds the *raw* value.
//!   Escaping happens in the renderer and nowhere else, so a literal
//!   carrying a quote or a backslash cannot be emitted unescaped. There is
//!   no constructor that accepts pre-escaped text.
//! - **A malformed `format!` template.** [`FormatTemplate`] is a sequence of
//!   literal chunks and holes. Braces inside a literal chunk are doubled by
//!   the renderer, so a literal `{` in generated output cannot accidentally
//!   open a format hole — the failure mode of building a nested `format!`
//!   template by hand.
//!
//! # What it does not catch
//!
//! [`Ident::new`] rejects text that is not a Rust identifier, but it accepts
//! keywords (`crate`, `Self`, `match`), which are legal in path position and
//! illegal in binding position. That distinction is not modelled. Callers
//! that build an identifier from unvalidated input get a rejection at the
//! parse boundary; callers that build one from a keyword get valid-looking
//! output that may not compile.

use std::fmt::Write as _;

use crate::ir::RustType;

// ── Identifiers and paths ──────────────────────────────────────────────────

/// Text that is not a legal Rust identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidIdent {
    text: String,
    reason: &'static str,
}

impl std::fmt::Display for InvalidIdent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid Rust identifier {:?}: {}",
            self.text, self.reason
        )
    }
}

impl std::error::Error for InvalidIdent {}

/// A validated Rust identifier.
///
/// The only way to build one is [`Ident::new`], which rejects anything that
/// is not `[A-Za-z_][A-Za-z0-9_]*`. This is the parse boundary for every
/// name that reaches generated output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ident(String);

impl Ident {
    /// Validate `text` as a Rust identifier.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidIdent`] if `text` is empty, starts with a digit, or
    /// contains a character outside `[A-Za-z0-9_]`.
    pub fn new(text: &str) -> Result<Self, InvalidIdent> {
        let invalid = |reason| InvalidIdent {
            text: text.to_string(),
            reason,
        };

        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return Err(invalid("identifiers cannot be empty"));
        };
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err(invalid(
                "identifiers must start with a letter or underscore",
            ));
        }
        if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
            return Err(match bad {
                ' ' => invalid("identifiers cannot contain spaces"),
                '-' => invalid("identifiers cannot contain hyphens"),
                '.' | ':' => invalid("identifiers cannot contain path separators"),
                _ => invalid("identifiers may only contain letters, digits and underscores"),
            });
        }
        Ok(Self(text.to_string()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A `::`-separated path, e.g. `std::time::Duration` or `Self`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path(Vec<Ident>);

impl Path {
    /// Build a path from its segments.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidIdent`] if any segment is not a legal identifier, or
    /// if `segments` is empty.
    pub fn new(segments: &[&str]) -> Result<Self, InvalidIdent> {
        if segments.is_empty() {
            return Err(InvalidIdent {
                text: String::new(),
                reason: "a path needs at least one segment",
            });
        }
        segments
            .iter()
            .map(|s| Ident::new(s))
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    /// A single-segment path.
    #[must_use]
    pub fn from_ident(ident: Ident) -> Self {
        Self(vec![ident])
    }

    /// Append a segment, e.g. `Error` onto `crate::error`.
    #[must_use]
    pub fn join(mut self, segment: Ident) -> Self {
        self.0.push(segment);
        self
    }
}

// ── String literals ────────────────────────────────────────────────────────

/// A Rust string literal, holding its **raw** value.
///
/// Escaping is performed by the renderer. There is no constructor that takes
/// pre-escaped text, so an unescaped literal is not representable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrLit {
    value: String,
    /// Collapse newlines to spaces before escaping.
    ///
    /// Rust string literals may span lines, but a literal inside a
    /// single-line attribute (`#[doc = "…"]`, `#[tool(description = "…")]`)
    /// must not. This flag is the difference, named in the type rather than
    /// hidden in a helper applied by hand at each call site.
    one_line: bool,
}

impl StrLit {
    /// A literal that preserves its value exactly, escaping newlines as `\n`.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            one_line: false,
        }
    }

    /// A literal for single-line attribute position: newlines become spaces.
    #[must_use]
    pub fn one_line(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            one_line: true,
        }
    }

    /// Render as a Rust string literal, quotes included.
    #[must_use]
    pub fn to_rust(&self) -> String {
        let mut out = String::new();
        self.render(&mut out);
        out
    }

    fn render(&self, out: &mut String) {
        out.push('"');
        for c in self.value.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' if self.one_line => out.push(' '),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
}

// ── format! templates ──────────────────────────────────────────────────────

/// One piece of a [`FormatTemplate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    /// Literal text. Braces are doubled by the renderer.
    Lit(String),
    /// A substitution hole: `{}` positional, or `{name}` captured.
    Hole(Option<Ident>),
}

/// The template of a generated `format!` invocation.
///
/// Modelling holes separately from literal text is what stops a literal brace
/// in the output from being read as a hole — the classic failure of writing
/// `"{}={{}}"` to emit `name={}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormatTemplate(Vec<TemplatePart>);

impl FormatTemplate {
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn lit(mut self, text: impl Into<String>) -> Self {
        self.0.push(TemplatePart::Lit(text.into()));
        self
    }

    /// A positional hole, `{}`.
    #[must_use]
    pub fn hole(mut self) -> Self {
        self.0.push(TemplatePart::Hole(None));
        self
    }

    /// A captured hole, `{name}`.
    #[must_use]
    pub fn captured(mut self, name: Ident) -> Self {
        self.0.push(TemplatePart::Hole(Some(name)));
        self
    }

    /// The template text, with literal braces doubled.
    #[must_use]
    pub fn text(&self) -> String {
        let mut s = String::new();
        for part in &self.0 {
            match part {
                TemplatePart::Lit(t) => {
                    for c in t.chars() {
                        match c {
                            '{' => s.push_str("{{"),
                            '}' => s.push_str("}}"),
                            c => s.push(c),
                        }
                    }
                }
                TemplatePart::Hole(None) => s.push_str("{}"),
                TemplatePart::Hole(Some(id)) => {
                    s.push('{');
                    s.push_str(id.as_str());
                    s.push('}');
                }
            }
        }
        s
    }
}

// ── Types ──────────────────────────────────────────────────────────────────

/// A Rust type expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    /// A type carried by the IR, rendered through its `Display` impl.
    Ir(RustType),
    /// A named path type, e.g. `reqwest::Response`.
    Path(Path),
    /// `&T`.
    Ref(Box<TypeExpr>),
    /// `Name<A, B>`.
    App(Path, Vec<TypeExpr>),
    /// `dyn Trait`.
    Dyn(Path),
    /// `()`.
    Unit,
}

impl TypeExpr {
    /// Render this type on its own, e.g. `Option<String>`.
    #[must_use]
    pub fn to_rust(&self) -> String {
        let mut out = String::new();
        self.render(&mut out);
        out
    }

    /// `&str`.
    ///
    /// # Panics
    ///
    /// Never: `str` is a valid identifier.
    #[must_use]
    pub fn str_ref() -> Self {
        Self::Ref(Box::new(Self::Path(
            Path::new(&["str"]).expect("`str` is a valid identifier"),
        )))
    }

    fn render(&self, out: &mut String) {
        match self {
            Self::Ir(rt) => {
                let _ = write!(out, "{rt}");
            }
            Self::Path(p) => render_path(p, out),
            Self::Ref(inner) => {
                out.push('&');
                inner.render(out);
            }
            Self::App(p, args) => {
                render_path(p, out);
                out.push('<');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    a.render(out);
                }
                out.push('>');
            }
            Self::Dyn(p) => {
                out.push_str("dyn ");
                render_path(p, out);
            }
            Self::Unit => out.push_str("()"),
        }
    }
}

fn render_path(p: &Path, out: &mut String) {
    for (i, seg) in p.0.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(seg.as_str());
    }
}

// ── Expressions ────────────────────────────────────────────────────────────

/// One link in a wrapped method chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainLink {
    /// `.name(args)`.
    Call(Ident, Vec<Expr>),
    /// `.name` — a field access, distinct from a nullary method call.
    Field(Ident),
    /// `.await`.
    Await,
    /// A link that contributes no text but still occupies a line.
    ///
    /// This exists to model an absent optional step — an unauthenticated
    /// client emits no auth call, but the blank line it leaves behind is part
    /// of the established output and is preserved deliberately.
    Absent,
}

/// A struct-literal field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldInit {
    /// `name` — shorthand for `name: name`.
    Shorthand(Ident),
    /// `name: value`.
    Named(Ident, Expr),
    /// `..expr` — functional update, e.g. `..Default::default()`.
    Rest(Expr),
}

/// How a struct literal is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Braces {
    /// `X { a, b }`.
    Inline,
    /// One field per line.
    Multiline,
}

/// A Rust expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A path or variable reference.
    Path(Path),
    /// A string literal.
    Str(StrLit),
    /// An integer literal.
    Int(i64),
    /// A character literal. Escaped by the renderer, as strings are.
    Char(char),
    /// `true` / `false`.
    Bool(bool),
    /// `receiver.field`.
    Field(Box<Expr>, Ident),
    /// `&expr`.
    Ref(Box<Expr>),
    /// `&ref` binding-free shared reference to a `ref` pattern binding.
    Not(Box<Expr>),
    /// `expr?`.
    Try(Box<Expr>),
    /// `expr.await`.
    Await(Box<Expr>),
    /// `func(args)`.
    Call(Path, Vec<Expr>),
    /// `receiver.name(args)`, rendered inline.
    MethodCall(Box<Expr>, Ident, Vec<Expr>),
    /// A method chain broken across lines, one link per line.
    Chain {
        receiver: Box<Expr>,
        links: Vec<ChainLink>,
    },
    /// `Path { fields }`.
    StructLit {
        path: Path,
        fields: Vec<FieldInit>,
        braces: Braces,
    },
    /// `format!(template, args)`.
    Format(FormatTemplate, Vec<Expr>),
    /// `if cond { a } else { b }`, rendered inline.
    IfElseInline {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    /// `|param| body`.
    Closure(Ident, Box<Expr>),
    /// A call whose single argument sits on its own line, with a trailing
    /// comma, as the established output breaks it out.
    CallWrapped { func: Path, arg: Box<Expr> },
    /// `()` — the unit value.
    Unit,
    /// A turbofish member access: `Option::<&str>::None`.
    ///
    /// The type argument is a [`TypeExpr`], not text, so the turbofish cannot
    /// be spelled with something that is not a type.
    Turbofish {
        base: Path,
        ty: Box<TypeExpr>,
        member: Ident,
    },
}

impl Expr {
    /// A bare variable reference.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidIdent`] if `name` is not a legal identifier.
    pub fn var(name: &str) -> Result<Self, InvalidIdent> {
        Path::new(&[name]).map(Self::Path)
    }

    /// A string literal from a raw value.
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::Str(StrLit::new(value))
    }

    fn render(&self, out: &mut String, indent: usize) {
        match self {
            Self::Path(p) => render_path(p, out),
            Self::Str(s) => s.render(out),
            Self::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Self::Char(c) => {
                out.push('\'');
                match c {
                    '\'' => out.push_str("\\'"),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    c => out.push(*c),
                }
                out.push('\'');
            }
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Field(recv, name) => {
                recv.render(out, indent);
                out.push('.');
                out.push_str(name.as_str());
            }
            Self::Ref(inner) => {
                out.push('&');
                inner.render(out, indent);
            }
            Self::Not(inner) => {
                out.push('!');
                inner.render(out, indent);
            }
            Self::Try(inner) => {
                inner.render(out, indent);
                out.push('?');
            }
            Self::Await(inner) => {
                inner.render(out, indent);
                out.push_str(".await");
            }
            Self::Call(p, args) => {
                render_path(p, out);
                render_args(args, out, indent);
            }
            Self::MethodCall(recv, name, args) => {
                recv.render(out, indent);
                out.push('.');
                out.push_str(name.as_str());
                render_args(args, out, indent);
            }
            Self::Chain { receiver, links } => {
                receiver.render(out, indent);
                for link in links {
                    newline(out, indent + 1);
                    match link {
                        ChainLink::Call(name, args) => {
                            out.push('.');
                            out.push_str(name.as_str());
                            render_args(args, out, indent + 1);
                        }
                        ChainLink::Field(name) => {
                            out.push('.');
                            out.push_str(name.as_str());
                        }
                        ChainLink::Await => out.push_str(".await"),
                        ChainLink::Absent => {}
                    }
                }
            }
            Self::StructLit {
                path,
                fields,
                braces,
            } => {
                render_path(path, out);
                match braces {
                    Braces::Inline => {
                        out.push_str(" { ");
                        for (i, f) in fields.iter().enumerate() {
                            if i > 0 {
                                out.push_str(", ");
                            }
                            render_field_init(f, out, indent);
                        }
                        out.push_str(" }");
                    }
                    Braces::Multiline => {
                        out.push_str(" {");
                        for f in fields {
                            newline(out, indent + 1);
                            render_field_init(f, out, indent + 1);
                            if !matches!(f, FieldInit::Rest(_)) {
                                out.push(',');
                            }
                        }
                        newline(out, indent);
                        out.push('}');
                    }
                }
            }
            Self::Format(template, args) => {
                out.push_str("format!(");
                StrLit::new(template.text()).render(out);
                for a in args {
                    out.push_str(", ");
                    a.render(out, indent);
                }
                out.push(')');
            }
            Self::IfElseInline {
                cond,
                then,
                otherwise,
            } => {
                out.push_str("if ");
                cond.render(out, indent);
                out.push_str(" { ");
                then.render(out, indent);
                out.push_str(" } else { ");
                otherwise.render(out, indent);
                out.push_str(" }");
            }
            Self::Closure(param, body) => {
                out.push('|');
                out.push_str(param.as_str());
                out.push_str("| ");
                body.render(out, indent);
            }
            Self::CallWrapped { func, arg } => {
                render_path(func, out);
                out.push('(');
                newline(out, indent + 1);
                arg.render(out, indent + 1);
                out.push(',');
                newline(out, indent);
                out.push(')');
            }
            Self::Unit => out.push_str("()"),
            Self::Turbofish { base, ty, member } => {
                render_path(base, out);
                out.push_str("::<");
                ty.render(out);
                out.push_str(">::");
                out.push_str(member.as_str());
            }
        }
    }
}

fn render_field_init(f: &FieldInit, out: &mut String, indent: usize) {
    match f {
        FieldInit::Shorthand(name) => out.push_str(name.as_str()),
        FieldInit::Named(name, value) => {
            out.push_str(name.as_str());
            out.push_str(": ");
            value.render(out, indent);
        }
        FieldInit::Rest(value) => {
            out.push_str("..");
            value.render(out, indent);
        }
    }
}

fn render_args(args: &[Expr], out: &mut String, indent: usize) {
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        a.render(out, indent);
    }
    out.push(')');
}

fn newline(out: &mut String, indent: usize) {
    out.push('\n');
    for _ in 0..indent {
        out.push_str("    ");
    }
}

// ── Statements ─────────────────────────────────────────────────────────────

/// One arm of a `match`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    /// The pattern text, e.g. `Ok(result)`. Built as an [`Expr`] so patterns
    /// that are really call shapes stay typed.
    pub pattern: Expr,
    pub body: Expr,
}

/// A statement inside a [`Block`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// `let [mut] name[: Ty] = init;`
    Let {
        name: Ident,
        mutable: bool,
        ty: Option<TypeExpr>,
        init: Expr,
    },
    /// `let name =` followed by the initialiser on the next line, indented.
    LetWrapped { name: Ident, init: Expr },
    /// `lhs = rhs;`
    Assign { lhs: Expr, rhs: Expr },
    /// `expr;`
    Semi(Expr),
    /// A trailing expression with no semicolon.
    Tail(Expr),
    /// `return expr;`
    Return(Expr),
    /// An empty line.
    Blank,
    /// `if cond { … }`
    If { cond: Expr, then: Block },
    /// `if let Some(ref name) = scrutinee { … }`
    IfLetSomeRef {
        name: Ident,
        scrutinee: Expr,
        then: Block,
    },
    /// `match scrutinee { arms }`
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
    },
}

impl Stmt {
    fn render(&self, out: &mut String, indent: usize) {
        match self {
            Self::Blank => {}
            Self::Let {
                name,
                mutable,
                ty,
                init,
            } => {
                out.push_str("let ");
                if *mutable {
                    out.push_str("mut ");
                }
                out.push_str(name.as_str());
                if let Some(t) = ty {
                    out.push_str(": ");
                    t.render(out);
                }
                out.push_str(" = ");
                init.render(out, indent);
                out.push(';');
            }
            Self::LetWrapped { name, init } => {
                out.push_str("let ");
                out.push_str(name.as_str());
                out.push_str(" =");
                newline(out, indent + 1);
                init.render(out, indent + 1);
                out.push(';');
            }
            Self::Assign { lhs, rhs } => {
                lhs.render(out, indent);
                out.push_str(" = ");
                rhs.render(out, indent);
                out.push(';');
            }
            Self::Semi(e) => {
                e.render(out, indent);
                out.push(';');
            }
            Self::Tail(e) => e.render(out, indent),
            Self::Return(e) => {
                out.push_str("return ");
                e.render(out, indent);
                out.push(';');
            }
            Self::If { cond, then } => {
                out.push_str("if ");
                cond.render(out, indent);
                out.push_str(" {");
                then.render(out, indent + 1);
                newline(out, indent);
                out.push('}');
            }
            Self::IfLetSomeRef {
                name,
                scrutinee,
                then,
            } => {
                out.push_str("if let Some(ref ");
                out.push_str(name.as_str());
                out.push_str(") = ");
                scrutinee.render(out, indent);
                out.push_str(" {");
                then.render(out, indent + 1);
                newline(out, indent);
                out.push('}');
            }
            Self::Match { scrutinee, arms } => {
                out.push_str("match ");
                scrutinee.render(out, indent);
                out.push_str(" {");
                for arm in arms {
                    newline(out, indent + 1);
                    arm.pattern.render(out, indent + 1);
                    out.push_str(" => ");
                    arm.body.render(out, indent + 1);
                    out.push(',');
                }
                newline(out, indent);
                out.push('}');
            }
        }
    }
}

/// A brace-delimited sequence of statements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block(pub Vec<Stmt>);

impl Block {
    /// Render each statement on its own line at `indent`.
    ///
    /// The caller has already emitted the opening brace; this starts with a
    /// newline and does **not** emit the closing brace.
    fn render(&self, out: &mut String, indent: usize) {
        for stmt in &self.0 {
            if matches!(stmt, Stmt::Blank) {
                out.push('\n');
            } else {
                newline(out, indent);
                stmt.render(out, indent);
            }
        }
    }
}

// ── Items ──────────────────────────────────────────────────────────────────

/// A generic parameter with an optional single bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParam {
    pub name: Ident,
    pub bound: Option<Path>,
}

/// A function parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    /// `&self`.
    SelfRef,
    /// `name: Ty`.
    Typed(Ident, TypeExpr),
    /// A destructuring parameter, e.g. `Parameters(input): Parameters<T>`.
    Destructured { pattern: Expr, ty: TypeExpr },
}

/// How a parameter list is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Params {
    /// All on the signature line.
    Inline,
    /// One per line, trailing comma.
    OnePerLine,
}

/// Documentation attached to an item, emitted verbatim after `/// `.
///
/// The text is **not** re-wrapped or escaped: a doc string containing a
/// newline produces a line that is no longer a comment. That is the
/// established behaviour of this generator and is preserved rather than
/// silently corrected; fixing it changes emitted bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc(pub Vec<String>);

/// A function definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnDef {
    pub doc: Doc,
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub is_async: bool,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub params_layout: Params,
    pub ret: TypeExpr,
    pub body: Block,
}

/// An attribute, e.g. `#[derive(Debug, Clone)]` or `#[tool(description = "…")]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attr {
    /// `#[word]`, e.g. `#[tool_router]`.
    Word(Ident),
    /// `#[name(a, b, c)]`, e.g. `#[derive(Debug, Clone)]`.
    List(Ident, Vec<Path>),
    /// `#[name(key = "value")]`, e.g. `#[schemars(description = "…")]`.
    KeyValue {
        name: Path,
        key: Ident,
        value: StrLit,
    },
}

impl Attr {
    fn render(&self, out: &mut String) {
        out.push_str("#[");
        match self {
            Self::Word(w) => out.push_str(w.as_str()),
            Self::List(name, items) => {
                out.push_str(name.as_str());
                out.push('(');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render_path(it, out);
                }
                out.push(')');
            }
            Self::KeyValue { name, key, value } => {
                render_path(name, out);
                out.push('(');
                out.push_str(key.as_str());
                out.push_str(" = ");
                value.render(out);
                out.push(')');
            }
        }
        out.push(']');
    }
}

/// A struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub name: Ident,
    pub ty: TypeExpr,
}

/// A struct definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub doc: Doc,
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub name: Ident,
    pub fields: Vec<FieldDecl>,
}

/// A member of an `impl` block.
///
/// Blank lines are explicit members rather than separators inserted between
/// items. Vertical spacing is part of the emitted bytes, so the caller states
/// it instead of the renderer guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImplItem {
    Fn(FnDef),
    /// A `//` comment line.
    Comment(String),
    /// An empty line.
    Blank,
}

/// An `impl` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplBlock {
    pub attrs: Vec<Attr>,
    /// `Some(Trait)` for `impl Trait for Ty`.
    pub trait_path: Option<Path>,
    pub self_ty: Path,
    pub items: Vec<ImplItem>,
}

/// One entry in a `use` group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseTree {
    /// A plain path, e.g. `ServerHandler` or `transport::stdio`.
    Leaf(Path),
    /// A nested group, e.g. `handler::server::{a, b}`.
    Group(Path, Vec<UseTree>),
}

impl UseTree {
    fn render(&self, out: &mut String) {
        match self {
            Self::Leaf(p) => render_path(p, out),
            Self::Group(p, children) => {
                render_path(p, out);
                out.push_str("::{");
                for (i, c) in children.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    c.render(out);
                }
                out.push('}');
            }
        }
    }
}

/// A top-level item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// `use path::{a, b};` — `leaves` empty means `use path;`.
    Use {
        path: Path,
        leaves: Vec<Path>,
        glob: bool,
    },
    /// A `use` whose group is broken across lines, one `lines` entry per line.
    UseLines {
        path: Path,
        lines: Vec<Vec<UseTree>>,
    },
    Struct(StructDef),
    Impl(ImplBlock),
    Fn(FnDef),
    /// A `//` comment line. Empty string emits a bare `//`.
    Comment(String),
    /// An empty line.
    Blank,
}

// ── Rendering ──────────────────────────────────────────────────────────────

fn render_doc(doc: &Doc, out: &mut String, indent: usize, first: &mut bool) {
    for line in &doc.0 {
        if *first {
            *first = false;
        } else {
            newline(out, indent);
        }
        if line.is_empty() {
            out.push_str("///");
        } else {
            out.push_str("/// ");
            out.push_str(line);
        }
    }
}

impl FnDef {
    fn render(&self, out: &mut String, indent: usize) {
        let mut first = true;
        render_doc(&self.doc, out, indent, &mut first);

        for attr in &self.attrs {
            if first {
                first = false;
            } else {
                newline(out, indent);
            }
            attr.render(out);
        }

        if !first {
            newline(out, indent);
        }
        if self.public {
            out.push_str("pub ");
        }
        if self.is_async {
            out.push_str("async ");
        }
        out.push_str("fn ");
        out.push_str(self.name.as_str());

        if !self.generics.is_empty() {
            out.push('<');
            for (i, g) in self.generics.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(g.name.as_str());
                if let Some(b) = &g.bound {
                    out.push_str(": ");
                    render_path(b, out);
                }
            }
            out.push('>');
        }

        out.push('(');
        match self.params_layout {
            Params::Inline => {
                for (i, p) in self.params.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render_param(p, out);
                }
            }
            Params::OnePerLine => {
                for p in &self.params {
                    newline(out, indent + 1);
                    render_param(p, out);
                    out.push(',');
                }
                newline(out, indent);
            }
        }
        out.push(')');

        if !matches!(self.ret, TypeExpr::Unit) {
            out.push_str(" -> ");
            self.ret.render(out);
        }

        out.push_str(" {");
        self.body.render(out, indent + 1);
        newline(out, indent);
        out.push('}');
    }
}

fn render_param(p: &Param, out: &mut String) {
    match p {
        Param::SelfRef => out.push_str("&self"),
        Param::Typed(name, ty) => {
            out.push_str(name.as_str());
            out.push_str(": ");
            ty.render(out);
        }
        Param::Destructured { pattern, ty } => {
            pattern.render(out, 0);
            out.push_str(": ");
            ty.render(out);
        }
    }
}

impl Item {
    fn render(&self, out: &mut String) {
        match self {
            Self::Blank => {}
            Self::Comment(text) => {
                if text.is_empty() {
                    out.push_str("//");
                } else {
                    out.push_str("// ");
                    out.push_str(text);
                }
            }
            Self::Use { path, leaves, glob } => {
                out.push_str("use ");
                render_path(path, out);
                if *glob {
                    out.push_str("::*");
                } else if !leaves.is_empty() {
                    out.push_str("::{");
                    for (i, l) in leaves.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        render_path(l, out);
                    }
                    out.push('}');
                }
                out.push(';');
            }
            Self::UseLines { path, lines } => {
                out.push_str("use ");
                render_path(path, out);
                out.push_str("::{");
                for line in lines {
                    newline(out, 1);
                    for (i, t) in line.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        t.render(out);
                    }
                    out.push(',');
                }
                newline(out, 0);
                out.push_str("};");
            }
            Self::Struct(s) => {
                let mut first = true;
                render_doc(&s.doc, out, 0, &mut first);
                for attr in &s.attrs {
                    if first {
                        first = false;
                    } else {
                        newline(out, 0);
                    }
                    attr.render(out);
                }
                if !first {
                    newline(out, 0);
                }
                if s.public {
                    out.push_str("pub ");
                }
                out.push_str("struct ");
                out.push_str(s.name.as_str());
                out.push_str(" {");
                for f in &s.fields {
                    for attr in &f.attrs {
                        newline(out, 1);
                        attr.render(out);
                    }
                    newline(out, 1);
                    if f.public {
                        out.push_str("pub ");
                    }
                    out.push_str(f.name.as_str());
                    out.push_str(": ");
                    f.ty.render(out);
                    out.push(',');
                }
                newline(out, 0);
                out.push('}');
            }
            Self::Impl(b) => {
                for attr in &b.attrs {
                    attr.render(out);
                    newline(out, 0);
                }
                out.push_str("impl ");
                if let Some(t) = &b.trait_path {
                    render_path(t, out);
                    out.push_str(" for ");
                }
                render_path(&b.self_ty, out);
                out.push_str(" {");
                for item in &b.items {
                    match item {
                        ImplItem::Blank => out.push('\n'),
                        ImplItem::Comment(text) => {
                            newline(out, 1);
                            out.push_str("// ");
                            out.push_str(text);
                        }
                        ImplItem::Fn(f) => {
                            newline(out, 1);
                            f.render(out, 1);
                        }
                    }
                }
                newline(out, 0);
                out.push('}');
            }
            Self::Fn(f) => f.render(out, 0),
        }
    }
}

/// Render a sequence of items as a source file.
///
/// Each item is followed by a newline; [`Item::Blank`] contributes an empty
/// line. The result always ends with a newline.
#[must_use]
pub fn render_file(items: &[Item]) -> String {
    let mut out = String::with_capacity(16384);
    for item in items {
        item.render(&mut out);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Ident {
        Ident::new(s).unwrap()
    }

    fn path(segs: &[&str]) -> Path {
        Path::new(segs).unwrap()
    }

    // -- Ident validation (the parse boundary) --

    #[test]
    fn accepts_ordinary_identifiers() {
        for good in ["x", "_x", "snake_case", "Pascal", "a1", "_", "T"] {
            assert!(
                Ident::new(good).is_ok(),
                "{good} should be a valid identifier"
            );
        }
    }

    #[test]
    fn rejects_non_identifiers() {
        for bad in ["", "2bad", "foo bar", "a-b", "a.b", "a::b", "há", "x!"] {
            assert!(Ident::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn rejection_explains_itself() {
        let err = Ident::new("foo bar").unwrap_err();
        assert!(err.to_string().contains("spaces"), "got: {err}");
    }

    #[test]
    fn empty_path_is_rejected() {
        assert!(Path::new(&[]).is_err());
    }

    // -- String literals: escaping is the renderer's job --

    fn rendered(lit: &StrLit) -> String {
        let mut s = String::new();
        lit.render(&mut s);
        s
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(rendered(&StrLit::new(r#"a"b\c"#)), r#""a\"b\\c""#);
    }

    #[test]
    fn escapes_newline_by_default() {
        assert_eq!(rendered(&StrLit::new("a\nb")), r#""a\nb""#);
    }

    #[test]
    fn one_line_flattens_newline_to_space() {
        assert_eq!(rendered(&StrLit::one_line("a\nb")), r#""a b""#);
    }

    #[test]
    fn one_line_still_escapes_quotes() {
        assert_eq!(
            rendered(&StrLit::one_line(r#"say "hi"\ok"#)),
            r#""say \"hi\"\\ok""#
        );
    }

    // -- format! templates --

    #[test]
    fn positional_hole_renders_empty_braces() {
        assert_eq!(
            FormatTemplate::new().lit("limit=").hole().text(),
            "limit={}"
        );
    }

    #[test]
    fn captured_hole_renders_its_name() {
        assert_eq!(
            FormatTemplate::new()
                .lit("/items/")
                .captured(id("item_id"))
                .text(),
            "/items/{item_id}"
        );
    }

    #[test]
    fn literal_braces_are_doubled() {
        // The whole point: a literal brace in the output cannot open a hole.
        assert_eq!(FormatTemplate::new().lit("a{b}c").text(), "a{{b}}c");
    }

    #[test]
    fn literal_brace_survives_a_round_trip_through_a_hole() {
        let t = FormatTemplate::new().lit("{").hole().lit("}");
        assert_eq!(t.text(), "{{{}}}");
    }

    // -- TypeExpr conformance with the IR's own Display --
    //
    // The generator hands IR types straight to the emitter. If TypeExpr ever
    // rendered them differently the output would silently drift, so pin the
    // correspondence across every RustType variant.

    #[test]
    fn ir_types_render_exactly_as_the_ir_displays_them() {
        let variants = [
            RustType::String,
            RustType::I64,
            RustType::U64,
            RustType::F64,
            RustType::Bool,
            RustType::Value,
            RustType::Named("Pet".into()),
            RustType::Vec(Box::new(RustType::Named("Pet".into()))),
            RustType::Option(Box::new(RustType::String)),
            RustType::Option(Box::new(RustType::Vec(Box::new(RustType::I64)))),
        ];
        for rt in variants {
            let mut out = String::new();
            TypeExpr::Ir(rt.clone()).render(&mut out);
            assert_eq!(
                out,
                rt.to_string(),
                "TypeExpr drifted from RustType::Display"
            );
        }
    }

    #[test]
    fn str_ref_renders() {
        let mut out = String::new();
        TypeExpr::str_ref().render(&mut out);
        assert_eq!(out, "&str");
    }

    // -- Expressions --

    fn expr_text(e: &Expr) -> String {
        let mut s = String::new();
        e.render(&mut s, 0);
        s
    }

    #[test]
    fn renders_a_method_call_chain_inline() {
        let e = Expr::Await(Box::new(Expr::MethodCall(
            Box::new(Expr::var("resp").unwrap()),
            id("text"),
            vec![],
        )));
        assert_eq!(expr_text(&e), "resp.text().await");
    }

    #[test]
    fn renders_a_wrapped_chain_one_link_per_line() {
        let e = Expr::Chain {
            receiver: Box::new(Expr::var("self").unwrap()),
            links: vec![ChainLink::Call(id("inner"), vec![]), ChainLink::Await],
        };
        assert_eq!(expr_text(&e), "self\n    .inner()\n    .await");
    }

    #[test]
    fn an_absent_chain_link_still_occupies_a_line() {
        let e = Expr::Chain {
            receiver: Box::new(Expr::var("x").unwrap()),
            links: vec![ChainLink::Absent, ChainLink::Call(id("send"), vec![])],
        };
        assert_eq!(expr_text(&e), "x\n    \n    .send()");
    }

    #[test]
    fn renders_format_with_args() {
        let e = Expr::Format(
            FormatTemplate::new().lit("limit=").hole(),
            vec![Expr::var("v").unwrap()],
        );
        assert_eq!(expr_text(&e), r#"format!("limit={}", v)"#);
    }

    #[test]
    fn renders_a_multiline_struct_literal() {
        let e = Expr::StructLit {
            path: path(&["Self"]),
            fields: vec![
                FieldInit::Shorthand(id("inner")),
                FieldInit::Named(id("n"), Expr::Bool(true)),
            ],
            braces: Braces::Multiline,
        };
        assert_eq!(expr_text(&e), "Self {\n    inner,\n    n: true,\n}");
    }

    #[test]
    fn renders_an_inline_struct_literal() {
        let e = Expr::StructLit {
            path: path(&["E", "Api"]),
            fields: vec![FieldInit::Shorthand(id("status"))],
            braces: Braces::Inline,
        };
        assert_eq!(expr_text(&e), "E::Api { status }");
    }

    #[test]
    fn renders_inline_if_else() {
        let e = Expr::IfElseInline {
            cond: Box::new(Expr::var("has_query").unwrap()),
            then: Box::new(Expr::string("&")),
            otherwise: Box::new(Expr::string("?")),
        };
        assert_eq!(expr_text(&e), r#"if has_query { "&" } else { "?" }"#);
    }

    #[test]
    fn renders_a_closure() {
        let e = Expr::Closure(
            id("e"),
            Box::new(Expr::MethodCall(
                Box::new(Expr::var("e").unwrap()),
                id("to_string"),
                vec![],
            )),
        );
        assert_eq!(expr_text(&e), "|e| e.to_string()");
    }

    // -- Items --

    #[test]
    fn renders_a_struct_with_attrs_and_doc() {
        let item = Item::Struct(StructDef {
            doc: Doc(vec!["A widget.".into()]),
            attrs: vec![Attr::List(
                id("derive"),
                vec![path(&["Debug"]), path(&["Clone"])],
            )],
            public: true,
            name: id("Widget"),
            fields: vec![FieldDecl {
                attrs: vec![],
                public: false,
                name: id("inner"),
                ty: TypeExpr::Path(path(&["reqwest", "Client"])),
            }],
        });
        assert_eq!(
            render_file(&[item]),
            "/// A widget.\n\
             #[derive(Debug, Clone)]\n\
             pub struct Widget {\n\
             \x20   inner: reqwest::Client,\n\
             }\n"
        );
    }

    #[test]
    fn renders_a_use_with_leaves() {
        let item = Item::Use {
            path: path(&["crate", "error"]),
            leaves: vec![path(&["ApiError"]), path(&["Result"])],
            glob: false,
        };
        assert_eq!(
            render_file(&[item]),
            "use crate::error::{ApiError, Result};\n"
        );
    }

    #[test]
    fn renders_a_glob_use() {
        let item = Item::Use {
            path: path(&["crate", "api", "types"]),
            leaves: vec![],
            glob: true,
        };
        assert_eq!(render_file(&[item]), "use crate::api::types::*;\n");
    }

    #[test]
    fn renders_an_async_fn_with_generics_and_one_param_per_line() {
        let f = FnDef {
            doc: Doc::default(),
            attrs: vec![],
            public: false,
            is_async: true,
            name: id("post"),
            generics: vec![GenericParam {
                name: id("T"),
                bound: Some(path(&["serde", "Serialize"])),
            }],
            params: vec![
                Param::SelfRef,
                Param::Typed(id("path"), TypeExpr::str_ref()),
            ],
            params_layout: Params::OnePerLine,
            ret: TypeExpr::App(path(&["Result"]), vec![TypeExpr::Path(path(&["T"]))]),
            body: Block(vec![Stmt::Tail(Expr::var("x").unwrap())]),
        };
        assert_eq!(
            render_file(&[Item::Fn(f)]),
            "async fn post<T: serde::Serialize>(\n\
             \x20   &self,\n\
             \x20   path: &str,\n\
             ) -> Result<T> {\n\
             \x20   x\n\
             }\n"
        );
    }

    #[test]
    fn a_unit_return_type_is_elided() {
        let f = FnDef {
            doc: Doc::default(),
            attrs: vec![],
            public: false,
            is_async: false,
            name: id("go"),
            generics: vec![],
            params: vec![],
            params_layout: Params::Inline,
            ret: TypeExpr::Unit,
            body: Block::default(),
        };
        assert_eq!(render_file(&[Item::Fn(f)]), "fn go() {\n}\n");
    }

    #[test]
    fn blank_statements_emit_an_empty_line() {
        let f = FnDef {
            doc: Doc::default(),
            attrs: vec![],
            public: false,
            is_async: false,
            name: id("go"),
            generics: vec![],
            params: vec![],
            params_layout: Params::Inline,
            ret: TypeExpr::Unit,
            body: Block(vec![
                Stmt::Semi(Expr::var("a").unwrap()),
                Stmt::Blank,
                Stmt::Tail(Expr::var("b").unwrap()),
            ]),
        };
        assert_eq!(
            render_file(&[Item::Fn(f)]),
            "fn go() {\n    a;\n\n    b\n}\n"
        );
    }

    #[test]
    fn renders_a_trait_impl_with_an_attribute() {
        let b = ImplBlock {
            attrs: vec![Attr::Word(id("tool_handler"))],
            trait_path: Some(path(&["ServerHandler"])),
            self_ty: path(&["Mcp"]),
            items: vec![],
        };
        assert_eq!(
            render_file(&[Item::Impl(b)]),
            "#[tool_handler]\nimpl ServerHandler for Mcp {\n}\n"
        );
    }

    #[test]
    fn impl_spacing_is_stated_by_the_caller_not_inferred() {
        let f = |name: &str| {
            ImplItem::Fn(FnDef {
                doc: Doc::default(),
                attrs: vec![],
                public: false,
                is_async: false,
                name: id(name),
                generics: vec![],
                params: vec![],
                params_layout: Params::Inline,
                ret: TypeExpr::Unit,
                body: Block::default(),
            })
        };
        let b = ImplBlock {
            attrs: vec![],
            trait_path: None,
            self_ty: path(&["C"]),
            items: vec![
                f("a"),
                ImplItem::Blank,
                ImplItem::Comment("-- section --".into()),
                ImplItem::Blank,
                f("b"),
                ImplItem::Blank,
            ],
        };
        assert_eq!(
            render_file(&[Item::Impl(b)]),
            "impl C {\n\
             \x20   fn a() {\n\
             \x20   }\n\
             \n\
             \x20   // -- section --\n\
             \n\
             \x20   fn b() {\n\
             \x20   }\n\
             \n\
             }\n"
        );
    }

    #[test]
    fn renders_a_key_value_attribute_with_escaping() {
        let a = Attr::KeyValue {
            name: path(&["schemars"]),
            key: id("description"),
            value: StrLit::one_line("says \"hi\"\nbye"),
        };
        let mut out = String::new();
        a.render(&mut out);
        assert_eq!(out, r#"#[schemars(description = "says \"hi\" bye")]"#);
    }

    #[test]
    fn renders_match_arms() {
        let s = Stmt::Match {
            scrutinee: Expr::var("x").unwrap(),
            arms: vec![MatchArm {
                pattern: Expr::Call(path(&["Ok"]), vec![Expr::var("v").unwrap()]),
                body: Expr::var("v").unwrap(),
            }],
        };
        let mut out = String::new();
        s.render(&mut out, 0);
        assert_eq!(out, "match x {\n    Ok(v) => v,\n}");
    }

    #[test]
    fn renders_if_let_some_ref() {
        let s = Stmt::IfLetSomeRef {
            name: id("v"),
            scrutinee: Expr::var("cursor").unwrap(),
            then: Block(vec![Stmt::Assign {
                lhs: Expr::var("has_query").unwrap(),
                rhs: Expr::Bool(true),
            }]),
        };
        let mut out = String::new();
        s.render(&mut out, 0);
        assert_eq!(
            out,
            "if let Some(ref v) = cursor {\n    has_query = true;\n}"
        );
    }
}

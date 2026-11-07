//! Parser for the schema language of `schema.prisma` files.
//!
//! It produces a small AST ([`Schema`]) with every block of the language: `datasource`,
//! `generator`, `model`, `view`, `enum` and `type` (composite types). Attributes keep
//! their arguments as expressions, so the code generator and the migration tooling can
//! interpret them without re-parsing.

use std::fmt;

/// A parsed `schema.prisma`.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub datasources: Vec<ConfigBlock>,
    pub generators: Vec<ConfigBlock>,
    pub models: Vec<Model>,
    pub enums: Vec<Enum>,
    /// Composite types (`type Address { ... }`).
    pub types: Vec<Model>,
}

/// `datasource db { ... }` / `generator client { ... }`.
#[derive(Debug, Clone)]
pub struct ConfigBlock {
    pub name: String,
    pub properties: Vec<(String, Expr)>,
}

impl ConfigBlock {
    pub fn get(&self, key: &str) -> Option<&Expr> {
        self.properties.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// `model` / `view` / `type` block.
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub fields: Vec<Field>,
    /// Block attributes (`@@map`, `@@id`, `@@unique`, `@@index`, `@@schema`, ...), without the `@@`.
    pub attributes: Vec<Attribute>,
    pub documentation: Option<String>,
    pub is_view: bool,
}

impl Model {
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }
    pub fn attributes_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Attribute> + 'a {
        self.attributes.iter().filter(move |a| a.name == name)
    }
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
    /// `@@map("...")` or the model name.
    pub fn db_name(&self) -> String {
        self.attribute("map").and_then(|a| a.arg_str(0, "name")).unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    Required,
    Optional,
    List,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    /// Type name (`String`, `Int`, a model, an enum, ...). For `Unsupported("...")` it is `Unsupported`.
    pub ty: String,
    pub arity: Arity,
    /// The database type of `Unsupported("vector(384)")`.
    pub unsupported: Option<String>,
    /// Field attributes without the `@` (`id`, `default`, `db.Decimal`, ...).
    pub attributes: Vec<Attribute>,
    pub documentation: Option<String>,
}

impl Field {
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name == name)
    }
    pub fn has(&self, name: &str) -> bool {
        self.attribute(name).is_some()
    }
    /// `@map("...")` or the field name.
    pub fn db_name(&self) -> String {
        self.attribute("map").and_then(|a| a.arg_str(0, "name")).unwrap_or_else(|| self.name.clone())
    }
    /// The `@db.X(...)` attribute, if any.
    pub fn native_type(&self) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.name.starts_with("db."))
    }
}

#[derive(Debug, Clone)]
pub struct Enum {
    pub name: String,
    pub values: Vec<EnumValue>,
    pub attributes: Vec<Attribute>,
    pub documentation: Option<String>,
}

impl Enum {
    pub fn db_name(&self) -> String {
        self.attributes
            .iter()
            .find(|a| a.name == "map")
            .and_then(|a| a.arg_str(0, "name"))
            .unwrap_or_else(|| self.name.clone())
    }
    pub fn schema(&self) -> Option<String> {
        self.attributes.iter().find(|a| a.name == "schema").and_then(|a| a.arg_str(0, "name"))
    }
}

#[derive(Debug, Clone)]
pub struct EnumValue {
    pub name: String,
    pub map: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<Arg>,
}

impl Attribute {
    /// Argument by name, or the `pos`-th positional argument.
    pub fn arg(&self, pos: usize, name: &str) -> Option<&Expr> {
        if let Some(a) = self.args.iter().find(|a| a.name.as_deref() == Some(name)) {
            return Some(&a.value);
        }
        self.args.iter().filter(|a| a.name.is_none()).nth(pos).map(|a| &a.value)
    }
    pub fn named(&self, name: &str) -> Option<&Expr> {
        self.args.iter().find(|a| a.name.as_deref() == Some(name)).map(|a| &a.value)
    }
    pub fn arg_str(&self, pos: usize, name: &str) -> Option<String> {
        self.arg(pos, name).and_then(|e| e.as_str().map(str::to_string))
    }
    /// A list of field references (`[a, b(sort: Desc)]`) as names.
    pub fn arg_fields(&self, pos: usize, name: &str) -> Vec<String> {
        self.arg(pos, name).map(|e| e.field_names()).unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    String(String),
    Number(String),
    Bool(bool),
    /// Identifier or dotted path (`Cascade`, `db.Uuid`, an enum value, a field name).
    Ident(String),
    Array(Vec<Expr>),
    /// Function call (`now()`, `dbgenerated("...")`, `env("X")`, `a(sort: Desc)`).
    Call(String, Vec<ArgValue>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArgValue {
    pub name: Option<String>,
    pub value: Expr,
}

impl Expr {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Expr::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Expr::Ident(s) => Some(s),
            _ => None,
        }
    }
    /// Field names in `[a, b(sort: Desc)]`, or a single `a`.
    pub fn field_names(&self) -> Vec<String> {
        match self {
            Expr::Array(items) => items.iter().filter_map(|e| e.ref_name()).collect(),
            other => other.ref_name().into_iter().collect(),
        }
    }
    fn ref_name(&self) -> Option<String> {
        match self {
            Expr::Ident(s) => Some(s.clone()),
            Expr::Call(n, _) => Some(n.clone()),
            Expr::String(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::String(s) => write!(f, "{s:?}"),
            Expr::Number(n) => write!(f, "{n}"),
            Expr::Bool(b) => write!(f, "{b}"),
            Expr::Ident(s) => write!(f, "{s}"),
            Expr::Array(items) => {
                write!(f, "[")?;
                for (i, e) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, "]")
            }
            Expr::Call(n, args) => {
                write!(f, "{n}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(k) = &a.name {
                        write!(f, "{k}: ")?;
                    }
                    write!(f, "{}", a.value)?;
                }
                write!(f, ")")
            }
        }
    }
}

/// A parse error with its line number (1-based).
#[derive(Debug, Clone)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "schema.prisma:{}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Doc(String),
    Newline,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Eq,
    Question,
    At,
    AtAt,
    Dot,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\n' => {
                out.push((Tok::Newline, line));
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' | '\u{feff}' => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                let doc = chars.get(i + 2) == Some(&'/');
                let start = if doc { i + 3 } else { i + 2 };
                let mut j = start;
                while j < chars.len() && chars[j] != '\n' {
                    j += 1;
                }
                if doc {
                    let text: String = chars[start..j].iter().collect();
                    out.push((Tok::Doc(text.trim_end_matches('\r').strip_prefix(' ').unwrap_or(&text).trim_end().to_string()), line));
                }
                i = j;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    let Some(&ch) = chars.get(i) else {
                        return Err(ParseError { line, message: "unterminated string".into() });
                    };
                    i += 1;
                    match ch {
                        '"' => break,
                        '\\' => {
                            let esc = chars.get(i).copied().unwrap_or('\\');
                            i += 1;
                            match esc {
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                'r' => s.push('\r'),
                                '0' => s.push('\0'),
                                'u' => {
                                    let hex: String = chars[i..(i + 4).min(chars.len())].iter().collect();
                                    if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                        s.push(c);
                                    }
                                    i += 4;
                                }
                                other => s.push(other),
                            }
                        }
                        '\n' => return Err(ParseError { line, message: "unterminated string".into() }),
                        other => s.push(other),
                    }
                }
                out.push((Tok::Str(s), line));
            }
            '{' => {
                out.push((Tok::LBrace, line));
                i += 1;
            }
            '}' => {
                out.push((Tok::RBrace, line));
                i += 1;
            }
            '(' => {
                out.push((Tok::LParen, line));
                i += 1;
            }
            ')' => {
                out.push((Tok::RParen, line));
                i += 1;
            }
            '[' => {
                out.push((Tok::LBracket, line));
                i += 1;
            }
            ']' => {
                out.push((Tok::RBracket, line));
                i += 1;
            }
            ',' => {
                out.push((Tok::Comma, line));
                i += 1;
            }
            ':' => {
                out.push((Tok::Colon, line));
                i += 1;
            }
            '=' => {
                out.push((Tok::Eq, line));
                i += 1;
            }
            '?' => {
                out.push((Tok::Question, line));
                i += 1;
            }
            '.' => {
                out.push((Tok::Dot, line));
                i += 1;
            }
            '@' => {
                if chars.get(i + 1) == Some(&'@') {
                    out.push((Tok::AtAt, line));
                    i += 2;
                } else {
                    out.push((Tok::At, line));
                    i += 1;
                }
            }
            c if c == '-' || c.is_ascii_digit() => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_digit() || matches!(chars[j], '.' | 'e' | 'E' | '+' | '-')) {
                    // A dot followed by a non-digit ends the number.
                    if chars[j] == '.' && !chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
                        break;
                    }
                    if matches!(chars[j], '+' | '-') && !matches!(chars[j - 1], 'e' | 'E') {
                        break;
                    }
                    j += 1;
                }
                out.push((Tok::Num(chars[i..j].iter().collect()), line));
                i = j;
            }
            c if c.is_alphanumeric() || c == '_' => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                out.push((Tok::Ident(chars[i..j].iter().collect()), line));
                i = j;
            }
            other => return Err(ParseError { line, message: format!("unexpected character `{other}`") }),
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    /// Nesting of arrays / function calls in the current expression.
    depth: usize,
}

/// Deepest nesting of `[...]` / `f(...)` in an attribute value. Real schemas use two or three
/// levels; the limit keeps a hostile schema from overflowing the stack.
const MAX_EXPR_DEPTH: usize = 64;

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }
    fn line(&self) -> usize {
        self.toks.get(self.pos).or(self.toks.last()).map(|(_, l)| *l).unwrap_or(1)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|(t, _)| t.clone());
        self.pos += 1;
        t
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError { line: self.line(), message: msg.into() })
    }
    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Tok::Newline)) {
            self.pos += 1;
        }
    }
    fn expect(&mut self, t: Tok) -> Result<(), ParseError> {
        match self.next() {
            Some(ref got) if *got == t => Ok(()),
            got => {
                self.pos -= 1;
                self.err(format!("expected {t:?}, found {got:?}"))
            }
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            got => {
                self.pos -= 1;
                self.err(format!("expected an identifier, found {got:?}"))
            }
        }
    }
    /// Doc comments immediately before the current position.
    fn take_docs(&mut self) -> Option<String> {
        let mut docs = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Doc(d)) => {
                    docs.push(d.clone());
                    self.pos += 1;
                }
                Some(Tok::Newline) => self.pos += 1,
                _ => break,
            }
        }
        if docs.is_empty() { None } else { Some(docs.join("\n")) }
    }

    fn schema(&mut self) -> Result<Schema, ParseError> {
        let mut s = Schema::default();
        loop {
            let docs = self.take_docs();
            let Some(tok) = self.next() else { break };
            let Tok::Ident(kw) = tok else {
                self.pos -= 1;
                return self.err(format!("expected a block (model, enum, ...), found {:?}", self.peek()));
            };
            match kw.as_str() {
                "datasource" | "generator" => {
                    let name = self.ident()?;
                    let block = self.config_block(name)?;
                    if kw == "datasource" { s.datasources.push(block) } else { s.generators.push(block) }
                }
                "model" | "view" | "type" => {
                    let name = self.ident()?;
                    let mut m = self.model_block(name)?;
                    m.documentation = docs;
                    m.is_view = kw == "view";
                    if kw == "type" { s.types.push(m) } else { s.models.push(m) }
                }
                "enum" => {
                    let name = self.ident()?;
                    let mut e = self.enum_block(name)?;
                    e.documentation = docs;
                    s.enums.push(e);
                }
                other => return self.err(format!("unknown block `{other}`")),
            }
        }
        Ok(s)
    }

    fn config_block(&mut self, name: String) -> Result<ConfigBlock, ParseError> {
        self.expect(Tok::LBrace)?;
        let mut properties = Vec::new();
        loop {
            self.take_docs();
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                None => return self.err("unterminated block"),
                _ => {}
            }
            let key = self.ident()?;
            self.expect(Tok::Eq)?;
            let v = self.expr()?;
            properties.push((key, v));
        }
        Ok(ConfigBlock { name, properties })
    }

    fn model_block(&mut self, name: String) -> Result<Model, ParseError> {
        self.expect(Tok::LBrace)?;
        let mut fields = Vec::new();
        let mut attributes = Vec::new();
        loop {
            let docs = self.take_docs();
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::AtAt) => {
                    self.pos += 1;
                    attributes.push(self.attribute()?);
                    continue;
                }
                None => return self.err("unterminated model"),
                _ => {}
            }
            let fname = self.ident()?;
            let mut ty = self.ident()?;
            while matches!(self.peek(), Some(Tok::Dot)) {
                self.pos += 1;
                ty.push('.');
                ty.push_str(&self.ident()?);
            }
            let mut unsupported = None;
            if ty == "Unsupported" {
                self.expect(Tok::LParen)?;
                match self.next() {
                    Some(Tok::Str(s)) => unsupported = Some(s),
                    _ => return self.err("expected Unsupported(\"type\")"),
                }
                self.expect(Tok::RParen)?;
            }
            let mut arity = Arity::Required;
            match self.peek() {
                Some(Tok::Question) => {
                    self.pos += 1;
                    arity = Arity::Optional;
                }
                Some(Tok::LBracket) => {
                    self.pos += 1;
                    self.expect(Tok::RBracket)?;
                    arity = Arity::List;
                    if matches!(self.peek(), Some(Tok::Question)) {
                        self.pos += 1;
                    }
                }
                _ => {}
            }
            let mut fattrs = Vec::new();
            let mut trailing_doc: Option<String> = None;
            loop {
                match self.peek() {
                    Some(Tok::At) => {
                        self.pos += 1;
                        fattrs.push(self.attribute()?);
                    }
                    Some(Tok::Doc(d)) => {
                        trailing_doc = Some(d.clone());
                        self.pos += 1;
                    }
                    Some(Tok::Newline) | Some(Tok::RBrace) | None => break,
                    other => return self.err(format!("unexpected {other:?} in field `{fname}`")),
                }
            }
            let documentation = match (docs, trailing_doc) {
                (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
                (a, b) => a.or(b),
            };
            fields.push(Field { name: fname, ty, arity, unsupported, attributes: fattrs, documentation });
        }
        Ok(Model { name, fields, attributes, documentation: None, is_view: false })
    }

    fn enum_block(&mut self, name: String) -> Result<Enum, ParseError> {
        self.expect(Tok::LBrace)?;
        let mut values = Vec::new();
        let mut attributes = Vec::new();
        loop {
            let docs = self.take_docs();
            match self.peek() {
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::AtAt) => {
                    self.pos += 1;
                    attributes.push(self.attribute()?);
                    continue;
                }
                None => return self.err("unterminated enum"),
                _ => {}
            }
            let vname = self.ident()?;
            let mut map = None;
            let mut trailing = None;
            loop {
                match self.peek() {
                    Some(Tok::At) => {
                        self.pos += 1;
                        let a = self.attribute()?;
                        if a.name == "map" {
                            map = a.arg_str(0, "name");
                        }
                    }
                    Some(Tok::Doc(d)) => {
                        trailing = Some(d.clone());
                        self.pos += 1;
                    }
                    Some(Tok::Newline) | Some(Tok::RBrace) | None => break,
                    // Several values on one line are not valid PSL, but be lenient with commas.
                    Some(Tok::Comma) => {
                        self.pos += 1;
                        break;
                    }
                    Some(Tok::Ident(_)) => break,
                    other => return self.err(format!("unexpected {other:?} in enum `{name}`")),
                }
            }
            values.push(EnumValue { name: vname, map, documentation: docs.or(trailing) });
        }
        Ok(Enum { name, values, attributes, documentation: None })
    }

    /// After `@` / `@@`: `name`, `db.Type`, with optional `(args)`.
    fn attribute(&mut self) -> Result<Attribute, ParseError> {
        let mut name = self.ident()?;
        while matches!(self.peek(), Some(Tok::Dot)) {
            self.pos += 1;
            name.push('.');
            name.push_str(&self.ident()?);
        }
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            for a in self.args(Tok::RParen)? {
                args.push(Arg { name: a.name, value: a.value });
            }
        }
        Ok(Attribute { name, args })
    }

    /// Arguments up to (and including) `close`.
    fn args(&mut self, close: Tok) -> Result<Vec<ArgValue>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            if self.peek() == Some(&close) {
                self.pos += 1;
                break;
            }
            // `name: value`
            let mut name = None;
            if let (Some(Tok::Ident(n)), Some((Tok::Colon, _))) = (self.peek().cloned(), self.toks.get(self.pos + 1)) {
                name = Some(n);
                self.pos += 2;
            }
            let value = self.expr()?;
            out.push(ArgValue { name, value });
            self.skip_newlines();
            match self.peek() {
                Some(Tok::Comma) => self.pos += 1,
                Some(t) if *t == close => {}
                other => return self.err(format!("expected `,` or {close:?}, found {other:?}")),
            }
        }
        Ok(out)
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        if self.depth >= MAX_EXPR_DEPTH {
            return self.err(format!("expression nested too deeply (more than {MAX_EXPR_DEPTH} levels)"));
        }
        self.depth += 1;
        let e = self.expr_inner();
        self.depth -= 1;
        e
    }

    fn expr_inner(&mut self) -> Result<Expr, ParseError> {
        self.skip_newlines();
        match self.next() {
            Some(Tok::Str(s)) => Ok(Expr::String(s)),
            Some(Tok::Num(n)) => Ok(Expr::Number(n)),
            Some(Tok::LBracket) => {
                let items = self.args(Tok::RBracket)?;
                Ok(Expr::Array(items.into_iter().map(|a| a.value).collect()))
            }
            Some(Tok::Ident(mut id)) => {
                while matches!(self.peek(), Some(Tok::Dot)) {
                    self.pos += 1;
                    id.push('.');
                    id.push_str(&self.ident()?);
                }
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.pos += 1;
                    let args = self.args(Tok::RParen)?;
                    return Ok(Expr::Call(id, args));
                }
                Ok(match id.as_str() {
                    "true" => Expr::Bool(true),
                    "false" => Expr::Bool(false),
                    _ => Expr::Ident(id),
                })
            }
            got => {
                self.pos -= 1;
                self.err(format!("expected a value, found {got:?}"))
            }
        }
    }
}

/// Parses a `schema.prisma` source.
pub fn parse(src: &str) -> Result<Schema, ParseError> {
    let toks = lex(src)?;
    Parser { toks, pos: 0, depth: 0 }.schema()
}

/// Parses the schema at `path`. If it is a directory (the `prismaSchemaFolder` layout),
/// every `*.prisma` file in it is merged.
pub fn parse_path(path: impl AsRef<std::path::Path>) -> Result<Schema, Box<dyn std::error::Error + Send + Sync>> {
    let path = path.as_ref();
    if path.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "prisma"))
            .collect();
        files.sort();
        let mut all = Schema::default();
        for f in files {
            let s = parse(&std::fs::read_to_string(&f)?).map_err(|e| format!("{}: {e}", f.display()))?;
            all.datasources.extend(s.datasources);
            all.generators.extend(s.generators);
            all.models.extend(s.models);
            all.enums.extend(s.enums);
            all.types.extend(s.types);
        }
        return Ok(all);
    }
    Ok(parse(&std::fs::read_to_string(path)?)?)
}

impl Schema {
    pub fn model(&self, name: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.name == name)
    }
    pub fn enum_(&self, name: &str) -> Option<&Enum> {
        self.enums.iter().find(|e| e.name == name)
    }
    pub fn composite(&self, name: &str) -> Option<&Model> {
        self.types.iter().find(|m| m.name == name)
    }
    /// The datasource `provider` (`postgresql`, `sqlite`, `mysql`, ...).
    pub fn provider(&self) -> Option<String> {
        self.datasources.first().and_then(|d| d.get("provider")).and_then(|e| e.as_str().map(str::to_string))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks() {
        let s = parse(
            r#"
            generator client { provider = "prisma-client-js" previewFeatures = ["views"] }
            datasource db {
              provider = "postgresql"
              url      = env("DATABASE_URL")
            }
            /// A user
            model User {
              id    Int    @id @default(autoincrement())
              email String @unique @db.VarChar(64) /// trailing
              tags  String[] @default([])
              posts Post[] @relation("a")
              vec   Unsupported("vector(3)")?
              @@index([email(sort: Desc), id], map: "ix")
              @@map("users")
            }
            enum Role {
              A
              B @map("b")
              @@map("role")
            }
            "#,
        )
        .unwrap();
        assert_eq!(s.provider().as_deref(), Some("postgresql"));
        let u = &s.models[0];
        assert_eq!(u.documentation.as_deref(), Some("A user"));
        assert_eq!(u.db_name(), "users");
        assert_eq!(u.fields.len(), 5);
        assert_eq!(u.fields[1].documentation.as_deref(), Some("trailing"));
        assert_eq!(u.fields[1].native_type().unwrap().name, "db.VarChar");
        assert_eq!(u.fields[2].arity, Arity::List);
        assert_eq!(u.fields[4].unsupported.as_deref(), Some("vector(3)"));
        let ix = u.attribute("index").unwrap();
        assert_eq!(ix.arg_fields(0, "fields"), vec!["email", "id"]);
        assert_eq!(ix.arg_str(1, "map").as_deref(), Some("ix"));
        assert_eq!(s.enums[0].values[1].map.as_deref(), Some("b"));
        assert_eq!(s.enums[0].db_name(), "role");
    }
}

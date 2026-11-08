//! `#[derive(Model)]` and `#[derive(OrmEnum)]` for corvis-orm: the Rust
//! equivalents of `model` and `enum` blocks in `schema.prisma`.
//!
//! ```ignore
//! #[derive(Debug, Clone, Serialize, Deserialize, Model)]
//! #[serde(rename_all = "camelCase")]
//! #[model(table = "users")]
//! #[model(unique(fields = ["tenantId", "email"]))]
//! pub struct User {
//!     #[id] #[default(uuid)] #[db("Uuid")] pub id: String,
//!     pub tenant_id: String,
//!     pub email: String,
//!     #[default("MEMBER")] pub role: Role,
//!     pub tags: Vec<String>,
//!     #[relation(has_many)] #[serde(default, skip_serializing_if = "Option::is_none")]
//!     pub posts: Option<Vec<Post>>,
//! }
//!
//! #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, OrmEnum)]
//! pub enum Role { ADMIN, MEMBER }
//! ```
//!
//! `#[derive(Model)]` generates:
//! - `impl corvis_orm::Model for User` (static metadata used by the query engine);
//! - a `UserDelegate` extension trait implemented for `OrmClient` (`db.user()`);
//! - a `user` module with typed helpers (`user::email::contains("@x")`, `user::id::order(..)`, ...).

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Expr, Fields, GenericArgument, Ident, Lit, Meta, PathArguments, Token, Type, UnOp};

#[proc_macro_derive(Model, attributes(model, id, unique, default, relation, updated_at, map, db_type, db, skip))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[derive(OrmEnum)]`: a schema `enum`. Variants serialize with their serde
/// name (the schema value name); `#[map("db_value")]` sets the database value and
/// `#[orm_enum(name = "Role", map = "role", schema = "public")]` the type.
#[proc_macro_derive(OrmEnum, attributes(orm_enum, map))]
pub fn derive_orm_enum(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    match expand_enum(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `sql!("SELECT * FROM \"User\" WHERE id = {id} AND email = {user.email}")`: a typed raw
/// statement (`corvis_orm::RawSql`), the equivalent of a tagged template
/// `` sql`SELECT ... WHERE id = ${id}` ``.
///
/// Every `{expr}` becomes a bound parameter (taken by reference through
/// `corvis_orm::ToSqlArg`); a `RawSql` value is inlined as a fragment. Write `{{` and
/// `}}` for literal braces (`'{{1,2}}'::int[]`).
#[proc_macro]
pub fn sql(input: TokenStream) -> TokenStream {
    let lit = syn::parse_macro_input!(input as syn::LitStr);
    match expand_sql(&lit) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_sql(lit: &syn::LitStr) -> syn::Result<TokenStream2> {
    let text = lit.value();
    let mut strings: Vec<String> = vec![String::new()];
    let mut exprs: Vec<Expr> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                strings.last_mut().unwrap().push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                strings.last_mut().unwrap().push('}');
            }
            '{' => {
                let mut src = String::new();
                let mut depth = 0usize;
                loop {
                    match chars.next() {
                        None => return Err(syn::Error::new(lit.span(), "sql!: unclosed `{` (write `{{` for a literal brace)")),
                        Some('{') => {
                            depth += 1;
                            src.push('{');
                        }
                        Some('}') if depth == 0 => break,
                        Some('}') => {
                            depth -= 1;
                            src.push('}');
                        }
                        Some(other) => src.push(other),
                    }
                }
                let expr: Expr = syn::parse_str(src.trim())
                    .map_err(|e| syn::Error::new(lit.span(), format!("sql!: invalid expression `{}`: {e}", src.trim())))?;
                exprs.push(expr);
                strings.push(String::new());
            }
            '}' => return Err(syn::Error::new(lit.span(), "sql!: unmatched `}` (write `}}` for a literal brace)")),
            other => strings.last_mut().unwrap().push(other),
        }
    }
    Ok(quote! {
        ::corvis_orm::RawSql::new(
            ::std::vec![#(#strings),*],
            ::std::vec![#(::corvis_orm::ToSqlArg::to_sql_arg(&(#exprs))),*],
        )
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Int,
    BigInt,
    Float,
    Boolean,
    String,
    DateTime,
    Json,
    Decimal,
    Bytes,
    /// Resolved through `ScalarType` (enums, custom types).
    Custom,
}

impl Kind {
    fn from_name(s: &str) -> Option<Kind> {
        Some(match s {
            "Int" => Kind::Int,
            "BigInt" => Kind::BigInt,
            "Float" => Kind::Float,
            "Decimal" => Kind::Decimal,
            "Boolean" | "Bool" => Kind::Boolean,
            "String" => Kind::String,
            "DateTime" => Kind::DateTime,
            "Json" => Kind::Json,
            "Bytes" => Kind::Bytes,
            _ => return None,
        })
    }
    fn from_type_ident(s: &str) -> Option<Kind> {
        Some(match s {
            "i8" | "i16" | "i32" | "u8" | "u16" => Kind::Int,
            "u32" | "i64" | "u64" | "isize" | "usize" => Kind::BigInt,
            "f32" | "f64" => Kind::Float,
            "bool" => Kind::Boolean,
            "String" | "Uuid" => Kind::String,
            "DateTime" | "NaiveDateTime" | "NaiveDate" => Kind::DateTime,
            "Value" | "JsonValue" => Kind::Json,
            "Decimal" => Kind::Decimal,
            _ => return None,
        })
    }
    fn tokens(&self, inner: &Type) -> TokenStream2 {
        match self {
            Kind::Int => quote!(::corvis_orm::FieldKind::Int),
            Kind::BigInt => quote!(::corvis_orm::FieldKind::BigInt),
            Kind::Float => quote!(::corvis_orm::FieldKind::Float),
            Kind::Boolean => quote!(::corvis_orm::FieldKind::Boolean),
            Kind::String => quote!(::corvis_orm::FieldKind::String),
            Kind::DateTime => quote!(::corvis_orm::FieldKind::DateTime),
            Kind::Json => quote!(::corvis_orm::FieldKind::Json),
            Kind::Decimal => quote!(::corvis_orm::FieldKind::Decimal),
            Kind::Bytes => quote!(::corvis_orm::FieldKind::Bytes),
            Kind::Custom => quote!(<#inner as ::corvis_orm::ScalarType>::KIND),
        }
    }
    fn numeric(&self) -> bool {
        matches!(self, Kind::Int | Kind::BigInt | Kind::Float | Kind::Decimal)
    }
    fn comparable(&self) -> bool {
        matches!(self, Kind::Int | Kind::BigInt | Kind::Float | Kind::String | Kind::DateTime | Kind::Decimal)
    }
}

enum DefaultSpec {
    Autoincrement,
    Now,
    Uuid,
    UuidV7,
    Cuid,
    Cuid2,
    Nanoid(u8),
    Ulid,
    DbGenerated(String),
    Literal(String),
}

struct RelationSpec {
    kind: String,
    fields: Vec<String>,
    references: Vec<String>,
    on_delete: Option<String>,
    on_update: Option<String>,
    name: Option<String>,
    map: Option<String>,
}

struct FieldInfo {
    ident: Ident,
    api_name: String,
    column: String,
    inner: Type,
    optional: bool,
    list: bool,
    kind: Option<Kind>,
    is_id: bool,
    unique: bool,
    default: Option<DefaultSpec>,
    updated_at: bool,
    relation: Option<RelationSpec>,
    native: Option<String>,
}

fn last_ident(ty: &Type) -> Option<(String, Option<Type>)> {
    if let Type::Path(p) = ty {
        let seg = p.path.segments.last()?;
        let arg = match &seg.arguments {
            PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| match g {
                GenericArgument::Type(t) => Some(t.clone()),
                _ => None,
            }),
            _ => None,
        };
        return Some((seg.ident.to_string(), arg));
    }
    None
}

fn snake_case(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower = i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            let prev_upper = i > 0 && chars[i - 1].is_uppercase();
            if i > 0 && (prev_lower || (prev_upper && next_lower)) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(*c);
        }
    }
    out
}

fn camel_from_snake(s: &str, upper_first: bool) -> String {
    let mut out = String::new();
    let mut upper = upper_first;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn apply_rename_all(rule: &str, name: &str) -> String {
    match rule {
        "camelCase" => camel_from_snake(name, false),
        "PascalCase" => camel_from_snake(name, true),
        "SCREAMING_SNAKE_CASE" => name.to_uppercase(),
        "kebab-case" => name.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => name.replace('_', "-").to_uppercase(),
        "lowercase" => name.to_lowercase(),
        "UPPERCASE" => name.to_uppercase(),
        _ => name.to_string(),
    }
}

fn lit_string(e: &Expr) -> Option<String> {
    match e {
        Expr::Lit(l) => match &l.lit {
            Lit::Str(s) => Some(s.value()),
            _ => None,
        },
        _ => None,
    }
}

fn string_list(e: &Expr) -> syn::Result<Vec<String>> {
    match e {
        Expr::Array(a) => {
            a.elems.iter().map(|x| lit_string(x).ok_or_else(|| syn::Error::new_spanned(x, "expected a string literal"))).collect()
        }
        Expr::Path(p) if p.path.get_ident().is_some() => Ok(vec![p.path.get_ident().unwrap().to_string()]),
        other => {
            let s = lit_string(other).ok_or_else(|| syn::Error::new_spanned(other, "expected \"a, b\" or [\"a\", \"b\"]"))?;
            Ok(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        }
    }
}

fn serde_metas(attrs: &[syn::Attribute]) -> Vec<Meta> {
    let mut out = Vec::new();
    for a in attrs {
        if a.path().is_ident("serde")
            && let Ok(list) = a.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        {
            out.extend(list);
        }
    }
    out
}

fn meta_name_value_str(m: &Meta, key: &str) -> Option<String> {
    if let Meta::NameValue(nv) = m
        && nv.path.is_ident(key)
    {
        return lit_string(&nv.value);
    }
    None
}

fn json_string(v: &str) -> String {
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn lit_json(e: &Expr) -> Option<String> {
    fn lit(l: &Lit, neg: bool) -> Option<String> {
        let sign = if neg { "-" } else { "" };
        Some(match l {
            Lit::Bool(b) => b.value.to_string(),
            Lit::Int(i) => format!("{sign}{}", i.base10_digits()),
            Lit::Float(f) => format!("{sign}{}", f.base10_digits()),
            Lit::Str(s) => json_string(&s.value()),
            _ => return None,
        })
    }
    match e {
        Expr::Lit(l) => lit(&l.lit, false),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => match &*u.expr {
            Expr::Lit(l) => lit(&l.lit, true),
            _ => None,
        },
        Expr::Array(a) => {
            let items: Option<Vec<String>> = a.elems.iter().map(lit_json).collect();
            Some(format!("[{}]", items?.join(",")))
        }
        // Bare identifier: an enum value (`#[default(ADMIN)]`).
        Expr::Path(p) => p.path.get_ident().map(|i| json_string(&i.to_string())),
        _ => None,
    }
}

fn parse_default(attr: &syn::Attribute) -> syn::Result<DefaultSpec> {
    let e: Expr = attr.parse_args()?;
    let (name, args): (Option<String>, Vec<Expr>) = match &e {
        Expr::Path(p) => (p.path.get_ident().map(|i| i.to_string()), vec![]),
        Expr::Call(c) => match &*c.func {
            Expr::Path(p) => (p.path.get_ident().map(|i| i.to_string()), c.args.iter().cloned().collect()),
            _ => (None, vec![]),
        },
        _ => (None, vec![]),
    };
    let int_arg = |args: &[Expr]| -> Option<u64> {
        args.first().and_then(|a| match a {
            Expr::Lit(l) => match &l.lit {
                Lit::Int(i) => i.base10_parse().ok(),
                _ => None,
            },
            _ => None,
        })
    };
    if let Some(id) = &name {
        match id.as_str() {
            "autoincrement" => return Ok(DefaultSpec::Autoincrement),
            "now" => return Ok(DefaultSpec::Now),
            "uuid" => return Ok(if int_arg(&args) == Some(7) { DefaultSpec::UuidV7 } else { DefaultSpec::Uuid }),
            "cuid" => return Ok(if int_arg(&args) == Some(2) { DefaultSpec::Cuid2 } else { DefaultSpec::Cuid }),
            "nanoid" => return Ok(DefaultSpec::Nanoid(int_arg(&args).unwrap_or(21) as u8)),
            "ulid" => return Ok(DefaultSpec::Ulid),
            "dbgenerated" => {
                let s = args.first().and_then(lit_string).unwrap_or_default();
                return Ok(DefaultSpec::DbGenerated(s));
            }
            _ => {}
        }
    }
    lit_json(&e).map(DefaultSpec::Literal).ok_or_else(|| {
        syn::Error::new_spanned(&e, "expected autoincrement, now, uuid, cuid, nanoid, ulid, dbgenerated(\"...\") or a literal")
    })
}

fn parse_relation(attr: &syn::Attribute) -> syn::Result<RelationSpec> {
    let args = attr.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated)?;
    let mut spec =
        RelationSpec { kind: String::new(), fields: vec![], references: vec![], on_delete: None, on_update: None, name: None, map: None };
    for a in args {
        match &a {
            Expr::Path(p) if p.path.get_ident().is_some() => {
                spec.kind = p.path.get_ident().unwrap().to_string();
            }
            Expr::Assign(asg) => {
                let key = match &*asg.left {
                    Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()).unwrap_or_default(),
                    _ => String::new(),
                };
                match key.as_str() {
                    "fields" => spec.fields = string_list(&asg.right)?,
                    "references" => spec.references = string_list(&asg.right)?,
                    "on_delete" | "onDelete" => spec.on_delete = lit_string(&asg.right).or_else(|| string_list(&asg.right).ok()?.pop()),
                    "on_update" | "onUpdate" => spec.on_update = lit_string(&asg.right).or_else(|| string_list(&asg.right).ok()?.pop()),
                    "name" => spec.name = lit_string(&asg.right),
                    "map" => spec.map = lit_string(&asg.right),
                    _ => {
                        return Err(syn::Error::new_spanned(
                            &asg.left,
                            "unknown relation argument (expected fields, references, on_delete, on_update, name, map)",
                        ));
                    }
                }
            }
            Expr::Lit(_) => spec.name = lit_string(&a),
            other => return Err(syn::Error::new_spanned(other, "unexpected relation argument")),
        }
    }
    if !matches!(spec.kind.as_str(), "has_many" | "has_one" | "belongs_to" | "many_to_many") {
        return Err(syn::Error::new_spanned(attr, "relation kind must be has_many, has_one, belongs_to or many_to_many"));
    }
    Ok(spec)
}

fn referential(s: &Option<String>) -> syn::Result<TokenStream2> {
    let Some(s) = s else { return Ok(quote!(::core::option::Option::None)) };
    let v = match s.replace(['_', ' '], "").to_lowercase().as_str() {
        "cascade" => quote!(Cascade),
        "restrict" => quote!(Restrict),
        "noaction" => quote!(NoAction),
        "setnull" => quote!(SetNull),
        "setdefault" => quote!(SetDefault),
        _ => return Err(syn::Error::new(Span::call_site(), format!("unknown referential action `{s}`"))),
    };
    Ok(quote!(::core::option::Option::Some(::corvis_orm::ReferentialAction::#v)))
}

fn opt_str(s: &Option<String>) -> TokenStream2 {
    match s {
        Some(v) => quote!(::core::option::Option::Some(#v)),
        None => quote!(::core::option::Option::None),
    }
}

/// `key(fields = [..], name = "..", map = "..", kind = "..")` inside `#[model(...)]`.
struct BlockSpec {
    fields: Vec<String>,
    name: Option<String>,
    map: Option<String>,
    kind: Option<String>,
    column: Option<String>,
    db: Option<String>,
    optional: bool,
}

fn parse_block(list: &syn::MetaList) -> syn::Result<BlockSpec> {
    let mut b = BlockSpec { fields: vec![], name: None, map: None, kind: None, column: None, db: None, optional: false };
    let items = list.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated)?;
    for it in items {
        match &it {
            Expr::Assign(a) => {
                let key = match &*a.left {
                    Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()).unwrap_or_default(),
                    _ => String::new(),
                };
                match key.as_str() {
                    "fields" => b.fields = string_list(&a.right)?,
                    "name" => b.name = lit_string(&a.right),
                    "map" => b.map = lit_string(&a.right),
                    "kind" | "type" => b.kind = lit_string(&a.right),
                    "column" => b.column = lit_string(&a.right),
                    "db" => b.db = lit_string(&a.right),
                    "optional" => {
                        b.optional = matches!(&*a.right, Expr::Lit(l) if matches!(&l.lit, Lit::Bool(v) if v.value));
                    }
                    _ => return Err(syn::Error::new_spanned(&a.left, "unknown argument")),
                }
            }
            Expr::Path(p) if p.path.is_ident("optional") => b.optional = true,
            Expr::Array(_) | Expr::Lit(_) => b.fields = string_list(&it)?,
            other => return Err(syn::Error::new_spanned(other, "unexpected argument")),
        }
    }
    Ok(b)
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let name_str = name.to_string();
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input, "#[derive(Model)] only supports structs"));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(&input, "#[derive(Model)] requires named fields"));
    };

    let mut table = name_str.clone();
    let mut module_name = snake_case(&name_str);
    let mut schema: Option<String> = None;
    let mut is_view = false;
    let mut pk_fields: Vec<String> = Vec::new();
    let mut pk_name: Option<String> = None;
    let mut pk_map: Option<String> = None;
    let mut uniques: Vec<BlockSpec> = Vec::new();
    let mut indexes: Vec<BlockSpec> = Vec::new();
    let mut unsupported: Vec<BlockSpec> = Vec::new();
    for a in &input.attrs {
        if a.path().is_ident("model") {
            let args = a.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            for m in args {
                if let Some(t) = meta_name_value_str(&m, "table").or_else(|| meta_name_value_str(&m, "map")) {
                    table = t;
                } else if let Some(t) = meta_name_value_str(&m, "module") {
                    module_name = t;
                } else if let Some(t) = meta_name_value_str(&m, "schema") {
                    schema = Some(t);
                } else if m.path().is_ident("view") {
                    is_view = true;
                } else if let Meta::List(l) = &m {
                    let b = parse_block(l)?;
                    if l.path.is_ident("id") {
                        pk_fields = b.fields;
                        pk_name = b.name;
                        pk_map = b.map;
                    } else if l.path.is_ident("unique") {
                        uniques.push(b);
                    } else if l.path.is_ident("index") {
                        indexes.push(b);
                    } else if l.path.is_ident("unsupported") {
                        unsupported.push(b);
                    } else {
                        return Err(syn::Error::new_spanned(l, "unknown #[model] block (expected id, unique, index or unsupported)"));
                    }
                } else {
                    return Err(syn::Error::new_spanned(
                        m,
                        "unknown #[model] argument (expected table, module, schema, view, id(..), unique(..), index(..), unsupported(..))",
                    ));
                }
            }
        }
    }
    let rename_all = serde_metas(&input.attrs).iter().find_map(|m| meta_name_value_str(m, "rename_all"));

    let mut fields = Vec::new();
    for f in &named.named {
        if f.attrs.iter().any(|a| a.path().is_ident("skip")) {
            continue;
        }
        let ident = f.ident.clone().unwrap();
        let rust_name = ident.to_string().trim_start_matches("r#").to_string();
        let serde = serde_metas(&f.attrs);
        if serde.iter().any(|m| m.path().is_ident("skip")) {
            continue;
        }
        let api_name = serde.iter().find_map(|m| meta_name_value_str(m, "rename")).unwrap_or_else(|| match &rename_all {
            Some(rule) => apply_rename_all(rule, &rust_name),
            None => rust_name.clone(),
        });
        let mut info = FieldInfo {
            ident: ident.clone(),
            api_name,
            column: rust_name.clone(),
            inner: f.ty.clone(),
            optional: false,
            list: false,
            kind: None,
            is_id: false,
            unique: false,
            default: None,
            updated_at: false,
            relation: None,
            native: None,
        };
        let mut db_type: Option<Kind> = None;
        for a in &f.attrs {
            let p = a.path();
            if p.is_ident("id") {
                info.is_id = true;
                if let Meta::List(l) = &a.meta {
                    let b = parse_block(l)?;
                    if b.map.is_some() {
                        pk_map = b.map;
                    }
                }
            } else if p.is_ident("unique") {
                info.unique = true;
            } else if p.is_ident("updated_at") {
                info.updated_at = true;
            } else if p.is_ident("default") {
                info.default = Some(parse_default(a)?);
            } else if p.is_ident("map") {
                let e: Expr = a.parse_args()?;
                info.column = lit_string(&e).ok_or_else(|| syn::Error::new_spanned(&e, "expected #[map(\"column\")]"))?;
            } else if p.is_ident("db") {
                let e: Expr = a.parse_args()?;
                info.native = Some(lit_string(&e).ok_or_else(|| syn::Error::new_spanned(&e, "expected #[db(\"Uuid\")]"))?);
            } else if p.is_ident("db_type") {
                let e: Expr = a.parse_args()?;
                let s = lit_string(&e).or_else(|| match &e {
                    Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                    _ => None,
                });
                db_type = Some(s.as_deref().and_then(Kind::from_name).ok_or_else(|| {
                    syn::Error::new_spanned(&e, "expected Int, BigInt, Float, Decimal, Boolean, String, DateTime, Json or Bytes")
                })?);
            } else if p.is_ident("relation") {
                info.relation = Some(parse_relation(a)?);
            }
        }
        // Unwrap Option / Vec / Box.
        let mut ty = f.ty.clone();
        if let Some((id, Some(arg))) = last_ident(&ty)
            && id == "Option"
        {
            info.optional = true;
            ty = arg;
        }
        if info.relation.is_some() {
            loop {
                match last_ident(&ty) {
                    Some((id, Some(arg))) if id == "Vec" => {
                        info.list = true;
                        ty = arg;
                    }
                    Some((id, Some(arg))) if id == "Box" || id == "Arc" || id == "Option" => {
                        ty = arg;
                    }
                    _ => break,
                }
            }
            // A nullable to-one may be wrapped twice (`Option<Option<Box<T>>>`).
        } else {
            // Scalar list `Vec<T>` (but `Vec<u8>` is `Bytes`).
            if let Some((id, Some(arg))) = last_ident(&ty)
                && id == "Vec"
            {
                let elem = last_ident(&arg).map(|(i, _)| i);
                if elem.as_deref() == Some("u8") {
                    info.kind = Some(Kind::Bytes);
                } else {
                    info.list = true;
                    ty = arg;
                }
            }
            if info.kind.is_none() {
                let kind = db_type.or_else(|| last_ident(&ty).and_then(|(id, _)| Kind::from_type_ident(&id)));
                info.kind = Some(kind.unwrap_or(Kind::Custom));
            }
        }
        info.inner = ty;
        fields.push(info);
    }

    if pk_fields.is_empty() {
        pk_fields = fields.iter().filter(|f| f.is_id).map(|f| f.api_name.clone()).collect();
    }
    if pk_fields.is_empty() && uniques.is_empty() && !fields.iter().any(|f| f.unique) && !is_view {
        return Err(syn::Error::new_spanned(name, "a model needs an #[id] field, #[model(id(fields = [...]))] or a unique"));
    }
    // Field lookup by api / rust name, for compound keys.
    let resolve = |n: &str| -> syn::Result<&FieldInfo> {
        fields
            .iter()
            .find(|f| f.api_name == n || f.ident == n || f.column == n)
            .ok_or_else(|| syn::Error::new(Span::call_site(), format!("unknown field `{n}` in #[model(...)] of `{name_str}`")))
    };
    let pk_api: Vec<String> = pk_fields.iter().map(|n| resolve(n).map(|f| f.api_name.clone())).collect::<syn::Result<_>>()?;

    // ----- metadata -----
    let mut field_metas = Vec::new();
    for f in &fields {
        let api = &f.api_name;
        let rust = f.ident.to_string();
        let column = &f.column;
        let optional = f.optional;
        let list = f.list;
        let is_id = f.is_id || (pk_api.len() == 1 && pk_api[0] == f.api_name);
        let unique = f.unique;
        let updated_at = f.updated_at;
        let inner = &f.inner;
        let kind = match &f.kind {
            Some(k) => k.tokens(inner),
            None => quote!(::corvis_orm::FieldKind::Relation),
        };
        let enum_meta = match &f.kind {
            Some(Kind::Custom) => quote!(<#inner as ::corvis_orm::ScalarType>::ENUM),
            _ => quote!(::core::option::Option::None),
        };
        let native = opt_str(&f.native);
        let default = match &f.default {
            None => quote!(::core::option::Option::None),
            Some(d) => {
                let v = match d {
                    DefaultSpec::Autoincrement => quote!(Autoincrement),
                    DefaultSpec::Now => quote!(Now),
                    DefaultSpec::Uuid => quote!(Uuid),
                    DefaultSpec::UuidV7 => quote!(UuidV7),
                    DefaultSpec::Cuid => quote!(Cuid),
                    DefaultSpec::Cuid2 => quote!(Cuid2),
                    DefaultSpec::Nanoid(n) => quote!(Nanoid(#n)),
                    DefaultSpec::Ulid => quote!(Ulid),
                    DefaultSpec::DbGenerated(s) => quote!(DbGenerated(#s)),
                    DefaultSpec::Literal(s) => quote!(Literal(#s)),
                };
                quote!(::core::option::Option::Some(::corvis_orm::DefaultValue::#v))
            }
        };
        let relation = match &f.relation {
            None => quote!(::core::option::Option::None),
            Some(r) => {
                let target = &f.inner;
                let kind = match r.kind.as_str() {
                    "has_many" => quote!(::corvis_orm::RelationKind::HasMany),
                    "has_one" => quote!(::corvis_orm::RelationKind::HasOne),
                    "many_to_many" => quote!(::corvis_orm::RelationKind::ManyToMany),
                    _ => quote!(::corvis_orm::RelationKind::BelongsTo),
                };
                let fs = &r.fields;
                let rs = &r.references;
                let od = referential(&r.on_delete)?;
                let ou = referential(&r.on_update)?;
                let rn = opt_str(&r.name);
                let rm = opt_str(&r.map);
                quote!(::core::option::Option::Some(::corvis_orm::RelationMeta {
                    kind: #kind,
                    target: <#target as ::corvis_orm::Model>::meta,
                    fields: &[#(#fs),*],
                    references: &[#(#rs),*],
                    on_delete: #od,
                    on_update: #ou,
                    name: #rn,
                    map: #rm,
                }))
            }
        };
        field_metas.push(quote! {
            ::corvis_orm::FieldMeta {
                name: #api,
                rust_name: #rust,
                column: #column,
                kind: #kind,
                optional: #optional,
                list: #list,
                is_id: #is_id,
                is_unique: #unique,
                default: #default,
                updated_at: #updated_at,
                relation: #relation,
                native_type: #native,
                enum_meta: #enum_meta,
            }
        });
    }

    let mut unique_metas = Vec::new();
    for u in &uniques {
        let names: Vec<String> = u.fields.iter().map(|n| resolve(n).map(|f| f.api_name.clone())).collect::<syn::Result<_>>()?;
        let input_name = u.name.clone().unwrap_or_else(|| names.join("_"));
        let map = opt_str(&u.map);
        unique_metas.push(quote!(::corvis_orm::UniqueMeta { name: #input_name, fields: &[#(#names),*], map: #map }));
    }
    let mut index_metas = Vec::new();
    for i in &indexes {
        let names: Vec<String> = i.fields.iter().map(|n| resolve(n).map(|f| f.api_name.clone())).collect::<syn::Result<_>>()?;
        let map = opt_str(&i.map);
        let kind = opt_str(&i.kind);
        index_metas.push(quote!(::corvis_orm::IndexMeta { fields: &[#(#names),*], map: #map, kind: #kind }));
    }
    let mut unsupported_metas = Vec::new();
    for u in &unsupported {
        let n = u.name.clone().unwrap_or_default();
        let col = u.column.clone().unwrap_or_else(|| n.clone());
        let db = u.db.clone().unwrap_or_default();
        let opt = u.optional;
        unsupported_metas.push(quote!(::corvis_orm::UnsupportedMeta { name: #n, column: #col, db_type: #db, optional: #opt }));
    }
    let schema_t = opt_str(&schema);
    let pk_name_t = opt_str(&pk_name);
    let pk_map_t = opt_str(&pk_map);

    let client_name = {
        let mut c = name_str.chars();
        match c.next() {
            Some(first) => first.to_lowercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    let mod_ident = Ident::new(&module_name, Span::call_site());
    let delegate_trait = format_ident!("{}Delegate", name);
    let method = Ident::new(&snake_case(&name_str), Span::call_site());

    // ----- typed helpers -----
    let mut field_mods = Vec::new();
    for f in &fields {
        let fmod = &f.ident;
        let api = &f.api_name;
        let inner = &f.inner;
        let mut fns = Vec::new();
        if let Some(rel) = &f.relation {
            let target = inner;
            if rel.kind == "has_many" || rel.kind == "many_to_many" {
                fns.push(quote! {
                    /// `{ some: { ... } }`
                    pub fn some(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "some", ::corvis_orm::where_object(params))
                    }
                    /// `{ every: { ... } }`
                    pub fn every(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "every", ::corvis_orm::where_object(params))
                    }
                    /// `{ none: { ... } }`
                    pub fn none(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "none", ::corvis_orm::where_object(params))
                    }
                    /// `include: { posts: { where: { ... } } }`
                    pub fn include_where(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::IncludeParam<#name> {
                        ::corvis_orm::IncludeParam::new(#api, ::corvis_orm::serde_json::json!({ "where": ::corvis_orm::where_object(params) }))
                    }
                    /// Nested write: `posts: { create: [...] }`.
                    pub fn create(items: ::std::vec::Vec<::std::vec::Vec<::corvis_orm::SetParam<#target>>>) -> ::corvis_orm::SetParam<#name> {
                        let items: ::std::vec::Vec<::corvis_orm::serde_json::Value> = items.into_iter().map(::corvis_orm::data_object).collect();
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "create": items }))
                    }
                    /// Nested write: `posts: { connect: [...] }`.
                    pub fn connect(items: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::SetParam<#name> {
                        let items: ::std::vec::Vec<::corvis_orm::serde_json::Value> = items.into_iter().map(|w| ::corvis_orm::where_object(::std::vec![w])).collect();
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "connect": items }))
                    }
                });
            } else {
                fns.push(quote! {
                    /// `{ is: { ... } }`
                    pub fn is(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "is", ::corvis_orm::where_object(params))
                    }
                    /// `{ isNot: { ... } }`
                    pub fn is_not(params: ::std::vec::Vec<::corvis_orm::WhereParam<#target>>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "isNot", ::corvis_orm::where_object(params))
                    }
                    /// Nested write: `author: { connect: { ... } }`.
                    pub fn connect(param: ::corvis_orm::WhereParam<#target>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "connect": ::corvis_orm::where_object(::std::vec![param]) }))
                    }
                    /// Nested write: `author: { create: { ... } }`.
                    pub fn create(data: ::std::vec::Vec<::corvis_orm::SetParam<#target>>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "create": ::corvis_orm::data_object(data) }))
                    }
                });
            }
            fns.push(quote! {
                /// `include: { relation: true }`
                pub fn include() -> ::corvis_orm::IncludeParam<#name> {
                    ::corvis_orm::IncludeParam::new(#api, ::corvis_orm::serde_json::Value::Bool(true))
                }
                /// `include: { relation: { ...args } }` with raw JSON arguments.
                pub fn include_args(args: ::corvis_orm::serde_json::Value) -> ::corvis_orm::IncludeParam<#name> {
                    ::corvis_orm::IncludeParam::new(#api, args)
                }
                /// `select: { relation: true }`
                pub fn select() -> ::corvis_orm::SelectParam<#name> {
                    ::corvis_orm::SelectParam::new(#api, ::corvis_orm::serde_json::Value::Bool(true))
                }
                /// The field name as seen by the query engine.
                pub const NAME: &str = #api;
            });
        } else if f.list {
            fns.push(quote! {
                /// `{ field: { has: value } }`
                pub fn has(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "has", ::corvis_orm::to_json(&v.into()))
                }
                /// `{ field: { hasSome: [...] } }`
                pub fn has_some(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "hasSome", ::corvis_orm::to_json(&v))
                }
                /// `{ field: { hasEvery: [...] } }`
                pub fn has_every(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "hasEvery", ::corvis_orm::to_json(&v))
                }
                /// `{ field: { isEmpty: bool } }`
                pub fn is_empty(v: bool) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "isEmpty", ::corvis_orm::serde_json::Value::Bool(v))
                }
                /// `{ field: { equals: [...] } }`
                pub fn equals(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "equals", ::corvis_orm::to_json(&v))
                }
                /// `data: { field: [...] }`
                pub fn set(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::SetParam<#name> {
                    ::corvis_orm::SetParam::new(#api, ::corvis_orm::to_json(&v))
                }
                /// `data: { field: { push: value | [...] } }`
                pub fn push(v: impl ::corvis_orm::serde::Serialize) -> ::corvis_orm::SetParam<#name> {
                    ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "push": ::corvis_orm::to_json(&v) }))
                }
                /// `select: { field: true }`
                pub fn select() -> ::corvis_orm::SelectParam<#name> {
                    ::corvis_orm::SelectParam::new(#api, ::corvis_orm::serde_json::Value::Bool(true))
                }
                /// The field name as seen by the query engine.
                pub const NAME: &str = #api;
            });
        } else {
            let kind = f.kind.unwrap();
            // The bare value (`{ field: value }`) means `equals` in a filter and is also what a
            // `WhereUniqueInput` accepts for a unique field; Json filters need `{ equals }`.
            let equals = if kind == Kind::Json {
                quote! { ::corvis_orm::WhereParam::field(#api, "equals", ::corvis_orm::to_json(&v.into())) }
            } else {
                quote! { ::corvis_orm::WhereParam::new(#api, ::corvis_orm::to_json(&v.into())) }
            };
            fns.push(quote! {
                /// `{ field: value }` (`{ field: { equals: value } }` for Json fields)
                pub fn equals(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    #equals
                }
                /// `{ field: { not: value } }`
                pub fn not(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "not", ::corvis_orm::to_json(&v.into()))
                }
                /// `{ field: { in: [...] } }`
                pub fn in_vec(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "in", ::corvis_orm::to_json(&v))
                }
                /// `{ field: { notIn: [...] } }`
                pub fn not_in_vec(v: ::std::vec::Vec<#inner>) -> ::corvis_orm::WhereParam<#name> {
                    ::corvis_orm::WhereParam::field(#api, "notIn", ::corvis_orm::to_json(&v))
                }
                /// `data: { field: value }` (`{ set: value }`).
                pub fn set(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::SetParam<#name> {
                    ::corvis_orm::SetParam::new(#api, ::corvis_orm::to_json(&v.into()))
                }
                /// `orderBy: { field: "asc" | "desc" }`
                pub fn order(order: ::corvis_orm::SortOrder) -> ::corvis_orm::OrderByParam<#name> {
                    ::corvis_orm::OrderByParam::new(#api, ::corvis_orm::serde_json::Value::from(order.as_str()))
                }
                /// `select: { field: true }`
                pub fn select() -> ::corvis_orm::SelectParam<#name> {
                    ::corvis_orm::SelectParam::new(#api, ::corvis_orm::serde_json::Value::Bool(true))
                }
                /// The field name as seen by the query engine.
                pub const NAME: &str = #api;
            });
            if f.optional {
                fns.push(quote! {
                    /// `{ field: null }`
                    pub fn is_null() -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::new(#api, ::corvis_orm::serde_json::Value::Null)
                    }
                    /// `{ field: { not: null } }`
                    pub fn is_not_null() -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "not", ::corvis_orm::serde_json::Value::Null)
                    }
                    /// `data: { field: null }`
                    pub fn set_null() -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::Value::Null)
                    }
                });
            }
            if kind.comparable() {
                fns.push(quote! {
                    pub fn lt(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "lt", ::corvis_orm::to_json(&v.into()))
                    }
                    pub fn lte(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "lte", ::corvis_orm::to_json(&v.into()))
                    }
                    pub fn gt(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "gt", ::corvis_orm::to_json(&v.into()))
                    }
                    pub fn gte(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "gte", ::corvis_orm::to_json(&v.into()))
                    }
                });
            }
            if kind == Kind::String {
                fns.push(quote! {
                    pub fn contains(v: impl ::core::convert::Into<::std::string::String>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "contains", ::corvis_orm::serde_json::Value::String(v.into()))
                    }
                    pub fn starts_with(v: impl ::core::convert::Into<::std::string::String>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "startsWith", ::corvis_orm::serde_json::Value::String(v.into()))
                    }
                    pub fn ends_with(v: impl ::core::convert::Into<::std::string::String>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::field(#api, "endsWith", ::corvis_orm::serde_json::Value::String(v.into()))
                    }
                    /// `{ field: { contains: v, mode: "insensitive" } }`
                    pub fn contains_insensitive(v: impl ::core::convert::Into<::std::string::String>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::new(#api, ::corvis_orm::serde_json::json!({ "contains": v.into(), "mode": "insensitive" }))
                    }
                    /// `{ field: { equals: v, mode: "insensitive" } }`
                    pub fn equals_insensitive(v: impl ::core::convert::Into<::std::string::String>) -> ::corvis_orm::WhereParam<#name> {
                        ::corvis_orm::WhereParam::new(#api, ::corvis_orm::serde_json::json!({ "equals": v.into(), "mode": "insensitive" }))
                    }
                });
            }
            if kind.numeric() {
                fns.push(quote! {
                    /// `data: { field: { increment: n } }`
                    pub fn increment(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "increment": ::corvis_orm::to_json(&v.into()) }))
                    }
                    pub fn decrement(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "decrement": ::corvis_orm::to_json(&v.into()) }))
                    }
                    pub fn multiply(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "multiply": ::corvis_orm::to_json(&v.into()) }))
                    }
                    pub fn divide(v: impl ::core::convert::Into<#inner>) -> ::corvis_orm::SetParam<#name> {
                        ::corvis_orm::SetParam::new(#api, ::corvis_orm::serde_json::json!({ "divide": ::corvis_orm::to_json(&v.into()) }))
                    }
                });
            }
        }
        field_mods.push(quote! {
            #[allow(dead_code, unused_imports, non_snake_case, clippy::all)]
            pub mod #fmod {
                use super::super::*;
                #(#fns)*
            }
        });
    }

    // Compound unique helpers: `wallet::merchant_id_currency(a, b)`.
    let mut compound_fns = Vec::new();
    let mut compounds: Vec<(String, Vec<String>)> = Vec::new();
    if pk_api.len() > 1 {
        compounds.push((pk_name.clone().unwrap_or_else(|| pk_api.join("_")), pk_api.clone()));
    }
    for u in &uniques {
        let names: Vec<String> = u.fields.iter().map(|n| resolve(n).map(|f| f.api_name.clone())).collect::<syn::Result<_>>()?;
        compounds.push((u.name.clone().unwrap_or_else(|| names.join("_")), names));
    }
    for (input_name, names) in &compounds {
        let fn_ident = Ident::new(&snake_case(input_name).replace("__", "_"), Span::call_site());
        let mut params = Vec::new();
        let mut inserts = Vec::new();
        for n in names {
            let fi = resolve(n)?;
            let pi = &fi.ident;
            let ty = &fi.inner;
            params.push(quote!(#pi: impl ::core::convert::Into<#ty>));
            inserts.push(quote!(m.insert(#n.to_string(), ::corvis_orm::to_json(&#pi.into()));));
        }
        compound_fns.push(quote! {
            /// Compound unique input `{ #input_name: { ... } }`.
            #[allow(non_snake_case, clippy::too_many_arguments)]
            pub fn #fn_ident(#(#params),*) -> ::corvis_orm::WhereParam<#name> {
                let mut m = ::corvis_orm::serde_json::Map::new();
                #(#inserts)*
                ::corvis_orm::WhereParam::new(#input_name, ::corvis_orm::serde_json::Value::Object(m))
            }
        });
    }

    Ok(quote! {
        impl ::corvis_orm::Model for #name {
            fn meta() -> &'static ::corvis_orm::ModelMeta {
                static META: ::corvis_orm::ModelMeta = ::corvis_orm::ModelMeta {
                    name: #name_str,
                    table: #table,
                    client_name: #client_name,
                    fields: &[#(#field_metas),*],
                    schema: #schema_t,
                    primary_key: &[#(#pk_api),*],
                    primary_key_name: #pk_name_t,
                    primary_key_map: #pk_map_t,
                    uniques: &[#(#unique_metas),*],
                    indexes: &[#(#index_metas),*],
                    unsupported: &[#(#unsupported_metas),*],
                    is_view: #is_view,
                };
                &META
            }
        }

        /// `db.<model>()` – generated by `#[derive(Model)]`.
        pub trait #delegate_trait {
            fn #method(&self) -> ::corvis_orm::Delegate<#name>;
        }

        impl #delegate_trait for ::corvis_orm::OrmClient {
            fn #method(&self) -> ::corvis_orm::Delegate<#name> {
                self.model::<#name>()
            }
        }

        /// Typed query helpers for the model.
        #[allow(dead_code, unused_imports, non_snake_case, clippy::all)]
        pub mod #mod_ident {
            use super::*;
            pub type WhereParam = ::corvis_orm::WhereParam<#name>;
            pub type SetParam = ::corvis_orm::SetParam<#name>;
            pub type OrderByParam = ::corvis_orm::OrderByParam<#name>;
            pub type IncludeParam = ::corvis_orm::IncludeParam<#name>;
            pub type SelectParam = ::corvis_orm::SelectParam<#name>;
            #(#compound_fns)*
            #(#field_mods)*
        }
    })
}

fn expand_enum(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input, "#[derive(OrmEnum)] only supports enums"));
    };
    let mut enum_name = name.to_string();
    let mut db_name: Option<String> = None;
    let mut schema: Option<String> = None;
    for a in &input.attrs {
        if a.path().is_ident("orm_enum") {
            let args = a.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            for m in args {
                if let Some(v) = meta_name_value_str(&m, "name") {
                    enum_name = v;
                } else if let Some(v) = meta_name_value_str(&m, "map") {
                    db_name = Some(v);
                } else if let Some(v) = meta_name_value_str(&m, "schema") {
                    schema = Some(v);
                } else {
                    return Err(syn::Error::new_spanned(m, "unknown #[orm_enum] argument (expected name, map, schema)"));
                }
            }
        }
    }
    let db_name = db_name.unwrap_or_else(|| enum_name.clone());
    let rename_all = serde_metas(&input.attrs).iter().find_map(|m| meta_name_value_str(m, "rename_all"));
    let mut values = Vec::new();
    let mut idents = Vec::new();
    let mut names = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(v, "OrmEnum variants must be unit variants"));
        }
        let ident = &v.ident;
        let raw = ident.to_string().trim_start_matches("r#").to_string();
        let serde = serde_metas(&v.attrs);
        let api = serde.iter().find_map(|m| meta_name_value_str(m, "rename")).unwrap_or_else(|| match &rename_all {
            Some(rule) => apply_rename_all(rule, &raw),
            None => raw.clone(),
        });
        let mut db = api.clone();
        for a in &v.attrs {
            if a.path().is_ident("map") {
                let e: Expr = a.parse_args()?;
                db = lit_string(&e).ok_or_else(|| syn::Error::new_spanned(&e, "expected #[map(\"value\")]"))?;
            }
        }
        values.push(quote!(::corvis_orm::EnumValueMeta { name: #api, db_name: #db }));
        idents.push(ident.clone());
        names.push(api);
    }
    let schema_t = opt_str(&schema);
    Ok(quote! {
        impl ::corvis_orm::OrmEnum for #name {
            fn enum_meta() -> &'static ::corvis_orm::EnumMeta {
                static META: ::corvis_orm::EnumMeta = ::corvis_orm::EnumMeta {
                    name: #enum_name,
                    db_name: #db_name,
                    schema: #schema_t,
                    values: &[#(#values),*],
                };
                &META
            }
            fn as_str(&self) -> &'static str {
                match self {
                    #(Self::#idents => #names,)*
                }
            }
            fn values() -> &'static [Self] {
                &[#(Self::#idents),*]
            }
        }

        impl ::corvis_orm::ScalarType for #name {
            const KIND: ::corvis_orm::FieldKind = ::corvis_orm::FieldKind::Enum;
            const ENUM: ::core::option::Option<fn() -> &'static ::corvis_orm::EnumMeta> =
                ::core::option::Option::Some(<#name as ::corvis_orm::OrmEnum>::enum_meta);
        }

        impl ::core::fmt::Display for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(<#name as ::corvis_orm::OrmEnum>::as_str(self))
            }
        }

        impl ::core::str::FromStr for #name {
            type Err = ::std::string::String;
            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                <#name as ::corvis_orm::OrmEnum>::from_name(s)
                    .ok_or_else(|| ::std::format!("Invalid value for enum {}: {}", #enum_name, s))
            }
        }

        impl ::core::convert::From<#name> for ::std::string::String {
            fn from(v: #name) -> ::std::string::String {
                <#name as ::corvis_orm::OrmEnum>::as_str(&v).to_string()
            }
        }
    })
}

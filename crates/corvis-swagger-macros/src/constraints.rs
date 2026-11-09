//! Validation constraints of a field (`#[derive(Validate)]` attributes) as OpenAPI keywords.
//!
//! The mapping and its order follow `createValidationPropertyAssignments` of the original OpenAPI compiler plugin
//! (`classValidatorShim`): the statements are successive inserts into one options map, so a later key replaces an
//! earlier one. The rule names are the ones of `RULES` in `corvis-macros/src/validate.rs`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, GenericArgument, PathArguments, Token, Type};

/// Rules that add keywords, in the order the plugin applies them (`is_in` and `is_enum` come first, `length` and
/// `matches` last).
const ORDER: &[&str] = &[
    "is_enum",
    "is_in",
    "min",
    "max",
    "min_length",
    "max_length",
    "array_not_empty",
    "is_positive",
    "is_negative",
    "array_unique",
    "is_email",
    "is_url",
    "is_uuid",
    "is_hex_color",
    "array_min_size",
    "array_max_size",
    "contains",
    "length",
    "matches",
];

/// Verbatim from the plugin's `decorators-properties.js`.
const HEX_COLOR_PATTERN: &str = "^#?([0-9A-F]{3}|[0-9A-F]{4}|[0-9A-F]{6}|[0-9A-F]{8})$";

struct Rule {
    positional: Vec<Expr>,
    each: bool,
}

fn rule_of(attr: &Attribute) -> syn::Result<Rule> {
    let exprs: Vec<Expr> = match &attr.meta {
        syn::Meta::Path(_) => Vec::new(),
        syn::Meta::List(l) if l.tokens.is_empty() => Vec::new(),
        _ => attr.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated)?.into_iter().collect(),
    };
    let mut rule = Rule { positional: Vec::new(), each: false };
    for e in exprs {
        match e {
            Expr::Assign(a) => {
                let key = match &*a.left {
                    Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
                    _ => String::new(),
                };
                if key == "each" {
                    rule.each = true;
                }
            }
            Expr::Path(p) if p.path.is_ident("each") => rule.each = true,
            other => rule.positional.push(other),
        }
    }
    Ok(rule)
}

fn generic_inner<'a>(ty: &'a Type, names: &[&str]) -> Option<&'a Type> {
    let Type::Path(tp) = ty else { return None };
    let last = tp.path.segments.last()?;
    if !names.iter().any(|n| last.ident == n) {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
    args.args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

/// The type a rule looks at: the field type without `Option` / `Box` and, with `each`, without the collection.
fn value_type(ty: &Type, each: bool) -> &Type {
    let mut t = ty;
    while let Some(inner) = generic_inner(t, &["Option", "Box", "Arc", "Rc"]) {
        t = inner;
    }
    if each {
        let elem = match t {
            Type::Array(a) => Some(&*a.elem),
            Type::Slice(s) => Some(&*s.elem),
            _ => generic_inner(t, &["Vec", "VecDeque", "HashSet", "BTreeSet", "IndexSet", "LinkedList"]),
        };
        if let Some(e) = elem {
            return value_type(e, false);
        }
    }
    t
}

/// `true` when the numbers of the field are floating point (`f32` / `f64`, also inside `Option` or `Vec`).
fn is_float(ty: &Type) -> bool {
    let mut t = ty;
    loop {
        if let Some(inner) = generic_inner(t, &["Option", "Box", "Arc", "Rc", "Vec", "VecDeque", "HashSet", "BTreeSet", "IndexSet"]) {
            t = inner;
            continue;
        }
        return match t {
            Type::Path(tp) => tp.path.segments.last().is_some_and(|s| s.ident == "f32" || s.ident == "f64"),
            Type::Array(a) => is_float(&a.elem),
            Type::Slice(s) => is_float(&s.elem),
            Type::Reference(r) => is_float(&r.elem),
            Type::Paren(p) => is_float(&p.elem),
            _ => false,
        };
    }
}

fn insert(key: &str, value: TokenStream) -> TokenStream {
    quote!(__o.insert(::std::string::String::from(#key), #value);)
}

fn json(e: &Expr) -> TokenStream {
    quote!(::corvis_swagger::__private::to_value(&(#e)))
}

fn string(s: &str) -> TokenStream {
    quote!(::corvis_swagger::__private::Value::String(::std::string::String::from(#s)))
}

fn number(n: i64) -> TokenStream {
    quote!(::corvis_swagger::__private::Value::from(#n))
}

fn flatten(args: &[Expr]) -> Vec<&Expr> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Expr::Array(arr) => out.extend(arr.elems.iter()),
            other => out.push(other),
        }
    }
    out
}

fn statements(name: &str, rule: &Rule, ty: &Type) -> syn::Result<Vec<TokenStream>> {
    let arg = |i: usize| -> syn::Result<&Expr> {
        rule.positional.get(i).ok_or_else(|| syn::Error::new(proc_macro2::Span::call_site(), format!("`#[{name}]` needs an argument")))
    };
    let mut out = Vec::new();
    match name {
        "is_enum" => {
            let t = match rule.positional.first() {
                Some(Expr::Path(p)) => {
                    let p = &p.path;
                    quote!(#p)
                }
                _ => {
                    let t = value_type(ty, rule.each);
                    quote!(#t)
                }
            };
            out.push(insert("enum", quote!(::corvis_swagger::__private::to_value(::corvis::validation::rules::enum_variants::<#t>()))));
            if rule.each {
                out.push(insert("isArray", quote!(::corvis_swagger::__private::Value::Bool(true))));
            }
        }
        "is_in" => {
            let values = flatten(&rule.positional).into_iter().map(json);
            out.push(insert("enum", quote!(::corvis_swagger::__private::Value::Array(::std::vec![#(#values),*]))));
            if rule.each {
                out.push(insert("isArray", quote!(::corvis_swagger::__private::Value::Bool(true))));
            }
        }
        "min" => out.push(insert("minimum", json(arg(0)?))),
        "max" => out.push(insert("maximum", json(arg(0)?))),
        "min_length" => out.push(insert("minLength", json(arg(0)?))),
        "max_length" => out.push(insert("maxLength", json(arg(0)?))),
        "array_not_empty" => out.push(insert("minItems", number(1))),
        "is_positive" if is_float(ty) => out.push(insert("exclusiveMinimum", number(0))),
        "is_positive" => out.push(insert("minimum", number(1))),
        "is_negative" if is_float(ty) => out.push(insert("exclusiveMaximum", number(0))),
        "is_negative" => out.push(insert("maximum", number(-1))),
        "array_unique" => out.push(insert("uniqueItems", quote!(::corvis_swagger::__private::Value::Bool(true)))),
        "is_email" => out.push(insert("format", string("email"))),
        "is_url" => out.push(insert("format", string("uri"))),
        "is_uuid" => out.push(insert("format", string("uuid"))),
        "is_hex_color" => out.push(insert("pattern", string(HEX_COLOR_PATTERN))),
        "array_min_size" => out.push(insert("minItems", json(arg(0)?))),
        "array_max_size" => out.push(insert("maxItems", json(arg(0)?))),
        "contains" => {
            let e = arg(0)?;
            out.push(insert(
                "pattern",
                quote!(::corvis_swagger::__private::Value::String(::corvis_swagger::__private::escape_pattern(::std::convert::AsRef::<str>::as_ref(&(#e))))),
            ));
        }
        "length" => {
            out.push(insert("minLength", json(arg(0)?)));
            if let Some(max) = rule.positional.get(1) {
                out.push(insert("maxLength", json(max)));
            }
        }
        "matches" => out.push(insert("pattern", json(arg(0)?))),
        _ => {}
    }
    Ok(out)
}

/// `__o.insert(...)` statements for the validation attributes of one field, in the order of the plugin. For a rule
/// written twice only the upper attribute counts.
pub fn constraint_tokens(attrs: &[Attribute], ty: &Type) -> syn::Result<Vec<TokenStream>> {
    let mut found: Vec<(&'static str, Rule)> = Vec::new();
    for a in attrs {
        let Some(name) = ORDER.iter().find(|n| a.path().is_ident(n)) else { continue };
        if found.iter().any(|(n, _)| n == name) {
            continue;
        }
        found.push((name, rule_of(a)?));
    }
    let mut out = Vec::new();
    for name in ORDER {
        if let Some((_, rule)) = found.iter().find(|(n, _)| n == name) {
            out.extend(statements(name, rule, ty)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(field: &str) -> Vec<String> {
        let f: syn::Field = syn::parse::Parser::parse_str(syn::Field::parse_named, field).unwrap();
        constraint_tokens(&f.attrs, &f.ty).unwrap().iter().map(|t| t.to_string().replace(' ', "")).collect()
    }

    #[test]
    fn length_with_one_and_two_arguments() {
        let one = tokens("#[length(2)] s: String");
        assert_eq!(one.len(), 1);
        assert!(one[0].contains("\"minLength\""));
        let two = tokens("#[length(2, 20)] s: String");
        assert_eq!(two.len(), 2);
        assert!(two[0].contains("\"minLength\"") && two[1].contains("\"maxLength\""));
    }

    #[test]
    fn is_in_flattens_arrays_and_marks_each() {
        let t = tokens("#[is_in([\"a\", \"b\"], each)] s: Vec<String>");
        assert_eq!(t.len(), 2);
        assert!(t[0].contains("\"enum\"") && t[0].contains("\"a\"") && t[0].contains("\"b\""));
        assert!(t[1].contains("\"isArray\""));
    }

    #[test]
    fn later_keywords_follow_the_plugin_order() {
        let t = tokens("#[is_positive] #[min(5)] n: i32");
        assert_eq!(t.len(), 2);
        assert!(t[0].contains("\"minimum\"") && t[0].contains("5"));
        assert!(t[1].contains("\"minimum\"") && t[1].contains("1i64"));
    }

    #[test]
    fn float_sign_uses_exclusive_bounds() {
        let t = tokens("#[is_positive] n: f64");
        assert!(t[0].contains("\"exclusiveMinimum\""));
        let t = tokens("#[is_negative] n: Option<f32>");
        assert!(t[0].contains("\"exclusiveMaximum\""));
    }

    #[test]
    fn repeated_rule_counts_once_and_other_rules_add_nothing() {
        let t = tokens("#[max(10)] #[max(20)] #[is_int] #[is_not_empty] n: i32");
        assert_eq!(t.len(), 1);
        assert!(t[0].contains("10") && !t[0].contains("20"));
    }
}

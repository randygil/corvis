//! Argument parsing for the `api_*!` macros: `key = value` pairs and positional expressions.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Token, Type};

pub enum NamedValue {
    Expr(Expr),
    Type(Type),
}

pub enum Arg {
    Named {
        key: String,
        #[allow(dead_code)]
        ident: syn::Ident,
        value: NamedValue,
    },
    Positional(Expr),
}

pub struct Args {
    pub items: Vec<Arg>,
}

/// Keys whose value is a type (`type = UserDto`, `type = [UserDto]`, `type = "string"`).
pub fn is_type_key(key: &str) -> bool {
    key == "type"
}

/// `operation_id` -> `operationId` (already camelCase keys are kept).
pub fn camel_case(s: &str) -> String {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for c in s.chars() {
        if c == '_' {
            upper = !out.is_empty();
            continue;
        }
        if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut items = Vec::new();
        while !input.is_empty() {
            let named = input.peek(syn::Ident::peek_any) && input.peek2(Token![=]) && !input.peek2(Token![==]);
            if named {
                let ident = syn::Ident::parse_any(input)?;
                input.parse::<Token![=]>()?;
                let raw = ident.to_string();
                let key = camel_case(&raw);
                let value = if is_type_key(&key) && !input.peek(syn::LitStr) {
                    NamedValue::Type(input.parse()?)
                } else {
                    NamedValue::Expr(input.parse()?)
                };
                items.push(Arg::Named { key, ident, value });
            } else {
                items.push(Arg::Positional(input.parse()?));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Args { items })
    }
}

/// `A, B, C` (types).
pub struct TypeList(pub Vec<Type>);

impl Parse for TypeList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let p = syn::punctuated::Punctuated::<Type, Token![,]>::parse_terminated(input)?;
        Ok(TypeList(p.into_iter().collect()))
    }
}

/// The JSON value of a `type = ...` argument: `[T]` becomes `[marker]` (an array type, like
/// `type: [UserDto]` in TypeScript); any other type its marker.
pub fn type_value_tokens(t: &Type) -> TokenStream {
    match t {
        Type::Slice(s) => {
            let inner = type_value_tokens(&s.elem);
            quote!(::corvis_swagger::__private::Value::Array(::std::vec![#inner]))
        }
        Type::Array(a) => {
            let inner = type_value_tokens(&a.elem);
            quote!(::corvis_swagger::__private::Value::Array(::std::vec![#inner]))
        }
        Type::Paren(p) => type_value_tokens(&p.elem),
        Type::Group(g) => type_value_tokens(&g.elem),
        other => type_marker_tokens(other),
    }
}

/// Registers the type and returns its marker (`{"$model": ...}` / `{"$ctor": ...}`).
pub fn type_marker_tokens(t: &Type) -> TokenStream {
    quote!({
        #[allow(unused_imports)]
        use ::corvis_swagger::__private::probe::{ModelProbe as _, ZodProbe as _, TypeProbe as _};
        (&&&::corvis_swagger::__private::probe::Probe::<#t>::new()).__swagger_type_marker()
    })
}

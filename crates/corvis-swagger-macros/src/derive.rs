//! `#[derive(ApiSchema)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericArgument, PathArguments, Type};

use crate::args::{self, Arg, Args, NamedValue};

fn serde_meta(attrs: &[syn::Attribute]) -> syn::Result<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();
    for a in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        a.parse_nested_meta(|m| {
            let key = m.path.get_ident().map(|i| i.to_string()).unwrap_or_default();
            if m.input.peek(syn::Token![=]) {
                let v = m.value()?;
                let lit: syn::Expr = v.parse()?;
                let s = match lit {
                    syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
                    _ => None,
                };
                out.push((key, s));
            } else if m.input.peek(syn::token::Paren) {
                // e.g. rename(serialize = "..")
                let content;
                syn::parenthesized!(content in m.input);
                let _: TokenStream = content.parse()?;
                out.push((key, None));
            } else {
                out.push((key, None));
            }
            Ok(())
        })?;
    }
    Ok(out)
}

fn rename_all(rule: &str, field: &str) -> String {
    let words: Vec<String> = field.split('_').filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    let cap = |w: &String| {
        let mut c = w.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    match rule {
        "lowercase" => field.to_lowercase(),
        "UPPERCASE" => field.to_uppercase(),
        "PascalCase" => words.iter().map(cap).collect(),
        "camelCase" => {
            let mut s = String::new();
            for (i, w) in words.iter().enumerate() {
                if i == 0 { s.push_str(w) } else { s.push_str(&cap(w)) }
            }
            s
        }
        "snake_case" => field.to_string(),
        "SCREAMING_SNAKE_CASE" => field.to_uppercase(),
        "kebab-case" => field.replace('_', "-"),
        "SCREAMING-KEBAB-CASE" => field.replace('_', "-").to_uppercase(),
        _ => field.to_string(),
    }
}

fn last_segment(ty: &Type) -> Option<(&syn::Ident, Option<&Type>)> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None;
    }
    let seg = p.path.segments.last()?;
    let inner = match &seg.arguments {
        PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| match g {
            GenericArgument::Type(t) => Some(t),
            _ => None,
        }),
        _ => None,
    };
    Some((&seg.ident, inner))
}

/// Field type -> (design type tokens, optional).
fn design_type(ty: &Type) -> (TokenStream, bool) {
    if let Some((id, Some(inner))) = last_segment(ty) {
        let n = id.to_string();
        if n == "Option" {
            let (t, _) = design_type(inner);
            return (t, true);
        }
        if matches!(n.as_str(), "Box" | "Arc" | "Rc") {
            return design_type(inner);
        }
        if matches!(n.as_str(), "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "IndexSet" | "LinkedList") {
            let (t, _) = design_type(inner);
            return (quote!(::corvis_swagger::__private::Value::Array(::std::vec![#t])), false);
        }
    }
    match ty {
        Type::Array(a) => {
            let (t, _) = design_type(&a.elem);
            (quote!(::corvis_swagger::__private::Value::Array(::std::vec![#t])), false)
        }
        Type::Slice(s) => {
            let (t, _) = design_type(&s.elem);
            (quote!(::corvis_swagger::__private::Value::Array(::std::vec![#t])), false)
        }
        Type::Reference(r) => design_type(&r.elem),
        Type::Paren(p) => design_type(&p.elem),
        other => (args::type_marker_tokens(other), false),
    }
}

fn options_tokens(args: &Args) -> syn::Result<Vec<TokenStream>> {
    let mut inserts = Vec::new();
    for a in &args.items {
        match a {
            Arg::Named { key, value, .. } => {
                let v = match value {
                    NamedValue::Expr(e) => quote!(::corvis_swagger::__private::to_value(&(#e))),
                    NamedValue::Type(t) => args::type_value_tokens(t),
                };
                inserts.push(quote!(__o.insert(::std::string::String::from(#key), #v);));
            }
            Arg::Positional(e) => return Err(syn::Error::new_spanned(e, "expected `key = value`")),
        }
    }
    Ok(inserts)
}

fn parse_args(attr: &syn::Attribute) -> syn::Result<Args> {
    match &attr.meta {
        syn::Meta::Path(_) => Ok(Args { items: Vec::new() }),
        syn::Meta::List(l) if l.tokens.is_empty() => Ok(Args { items: Vec::new() }),
        _ => attr.parse_args::<Args>(),
    }
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input.ident, "#[derive(ApiSchema)] supports structs with named fields"));
    };
    let fields = match &data.fields {
        Fields::Named(n) => n.named.iter().collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(&input.ident, "#[derive(ApiSchema)] supports structs with named fields"));
        }
    };

    let container_serde = serde_meta(&input.attrs)?;
    let rule = container_serde.iter().find(|(k, _)| k == "rename_all").and_then(|(_, v)| v.clone());

    let mut schema_name = quote!(::corvis_swagger::__private::short_type_name(::std::any::type_name::<Self>()));
    let mut schema_props = Vec::new();
    for a in input.attrs.iter().filter(|a| a.path().is_ident("api_schema")) {
        for item in parse_args(a)?.items {
            match item {
                Arg::Named { key, value: NamedValue::Expr(e), .. } if key == "name" => {
                    schema_name = quote!(::std::string::ToString::to_string(&(#e)));
                }
                Arg::Named { key, value, .. } => {
                    let v = match value {
                        NamedValue::Expr(e) => quote!(::corvis_swagger::__private::to_value(&(#e))),
                        NamedValue::Type(t) => args::type_value_tokens(&t),
                    };
                    schema_props.push(quote!(__sp.insert(::std::string::String::from(#key), #v);));
                }
                Arg::Positional(e) => return Err(syn::Error::new_spanned(e, "expected `key = value`")),
            }
        }
    }

    let mut props = Vec::new();
    for f in fields {
        let Some(fid) = &f.ident else { continue };
        let fname = fid.to_string();
        let fname = fname.strip_prefix("r#").unwrap_or(&fname).to_string();
        let serde = serde_meta(&f.attrs)?;
        if serde.iter().any(|(k, _)| k == "skip" || k == "skip_serializing") {
            continue;
        }
        if f.attrs.iter().any(|a| a.path().is_ident("api_hide_property")) {
            continue;
        }
        let json_name = serde
            .iter()
            .find(|(k, v)| k == "rename" && v.is_some())
            .and_then(|(_, v)| v.clone())
            .unwrap_or_else(|| match &rule {
                Some(r) => rename_all(r, &fname),
                None => fname.clone(),
            });

        let mut prefix = Vec::new();
        let mut inserts = Vec::new();
        let mut suffix = Vec::new();
        for a in &f.attrs {
            let p = a.path();
            if p.is_ident("api_property") {
                inserts.extend(options_tokens(&parse_args(a)?)?);
            } else if p.is_ident("api_property_optional") {
                inserts.extend(options_tokens(&parse_args(a)?)?);
                suffix.push(quote!(__o.insert(::std::string::String::from("required"), ::corvis_swagger::__private::Value::Bool(false));));
            } else if p.is_ident("api_response_property") {
                prefix.push(quote!(__o.insert(::std::string::String::from("readOnly"), ::corvis_swagger::__private::Value::Bool(true));));
                inserts.extend(options_tokens(&parse_args(a)?)?);
            }
        }
        let (design, optional) = design_type(&f.ty);
        props.push(quote!({
            #[allow(unused_mut)]
            let mut __o = ::corvis_swagger::__private::Map::new();
            #(#prefix)*
            #(#inserts)*
            #(#suffix)*
            __props.push((
                ::std::string::String::from(#json_name),
                ::corvis_swagger::__private::property_metadata(__o, #design, #optional),
            ));
        }));
    }

    let (impl_g, ty_g, where_c) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_g ::corvis::openapi::ApiSchema for #ident #ty_g #where_c {
            fn api_schema_def() -> ::corvis::openapi::ApiSchemaDef {
                #[allow(unused_mut)]
                let mut __props: ::std::vec::Vec<(::std::string::String, ::corvis_swagger::__private::Value)> = ::std::vec::Vec::new();
                #(#props)*
                #[allow(unused_mut)]
                let mut __sp = ::corvis_swagger::__private::Map::new();
                #(#schema_props)*
                ::corvis::openapi::ApiSchemaDef {
                    name: #schema_name,
                    schema_properties: __sp,
                    properties: __props,
                    ..::std::default::Default::default()
                }
            }
        }
    })
}

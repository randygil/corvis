use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, ExprArray, GenericArgument, Lit, PathArguments, Token, Type};

/// Last path segment of an attribute (`#[corvis::get]` -> `get`).
pub fn attr_name(attr: &Attribute) -> String {
    attr.path().segments.last().map(|s| s.ident.to_string()).unwrap_or_default()
}

/// Parses `#[attr(a, b = c, ...)]` arguments as expressions (empty for `#[attr]`).
pub fn attr_args(attr: &Attribute) -> syn::Result<Vec<Expr>> {
    match &attr.meta {
        syn::Meta::Path(_) => Ok(Vec::new()),
        syn::Meta::List(list) if list.tokens.is_empty() => Ok(Vec::new()),
        _ => {
            let p = attr.parse_args_with(Punctuated::<Expr, Token![,]>::parse_terminated)?;
            Ok(p.into_iter().collect())
        }
    }
}

/// `true` for paths that look like constants/values (`CONFIG_OPTIONS`, `VERSION_NEUTRAL`).
pub fn is_const_like(path: &syn::Path) -> bool {
    let Some(last) = path.segments.last() else { return false };
    let s = last.ident.to_string();
    s.chars().any(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Returns the path if the expression is a plain type-like path (`CatsService`, `auth::AuthGuard`).
pub fn type_path(expr: &Expr) -> Option<&syn::Path> {
    match expr {
        Expr::Path(p) if p.qself.is_none() && !is_const_like(&p.path) => Some(&p.path),
        _ => None,
    }
}

/// Flattens `[a, b]` into `a, b`; otherwise returns the expression itself.
pub fn flatten_array(exprs: Vec<Expr>) -> Vec<Expr> {
    let mut out = Vec::new();
    for e in exprs {
        match e {
            Expr::Array(ExprArray { elems, .. }) => out.extend(elems),
            other => out.push(other),
        }
    }
    out
}

pub fn lit_str(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(l) => match &l.lit {
            Lit::Str(s) => Some(s.value()),
            _ => None,
        },
        _ => None,
    }
}

/// `Arc<T>` -> Some(T)
pub fn generic_inner<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    let Type::Path(tp) = ty else { return None };
    let last = tp.path.segments.last()?;
    if last.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
    args.args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}

pub fn last_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => last_ident(&r.elem),
        _ => None,
    }
}

/// Compact display of a type: `Arc < CatsService >` -> `CatsService`.
pub fn type_display(ty: &Type) -> String {
    let inner = generic_inner(ty, "Option").unwrap_or(ty);
    let inner = generic_inner(inner, "Arc").unwrap_or(inner);
    let s = inner.to_token_stream().to_string().replace(' ', "");
    let s = s.strip_prefix("dyn").map(|r| format!("dyn {r}")).unwrap_or(s);
    s.rsplit("::").next().map(|x| x.to_string()).unwrap_or(s)
}

pub fn err(span: impl ToTokens, msg: &str) -> TokenStream {
    syn::Error::new_spanned(span, msg).to_compile_error()
}

pub fn enhancer_tokens(kind: &str, exprs: Vec<Expr>) -> Vec<TokenStream> {
    let (of, inst) = match kind {
        "guard" => (quote!(::corvis::guard_of), quote!(::corvis::guard_instance)),
        "interceptor" => (quote!(::corvis::interceptor_of), quote!(::corvis::interceptor_instance)),
        "pipe" => (quote!(::corvis::pipe_of), quote!(::corvis::pipe_instance)),
        _ => (quote!(::corvis::filter_of), quote!(::corvis::filter_instance)),
    };
    flatten_array(exprs)
        .into_iter()
        .map(|e| match type_path(&e) {
            Some(p) => quote!(#of::<#p>()),
            None => quote!(#inst(#e)),
        })
        .collect()
}

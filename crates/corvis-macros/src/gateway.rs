//! `#[websocket_gateway(path = "/chat")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{Expr, ItemStruct};

use crate::injectable::{InjectableOptions, export_impl, injectable_impl};
use crate::utils::{attr_args, attr_name, enhancer_tokens, lit_str};

/// The most bytes a Socket.IO namespace name may have (the rule of `ws/socket-io`).
const MAX_NAMESPACE_LEN: usize = 256;

/// What `cors = ...` declared.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cors {
    Any,
    Origins(Vec<String>),
}

/// The options of `#[websocket_gateway(...)]`.
#[derive(Debug, Default, PartialEq, Eq)]
struct GatewayOptions {
    /// The path the `ws` adapter serves: `path`, `namespace` or the literal, the last one given, normalized.
    ws_path: String,
    /// `namespace = "..."` or the literal, as written.
    namespace: Option<String>,
    /// `path = "..."` only, as written.
    path: Option<String>,
    cors: Option<Cors>,
}

const ACCEPTED: &str = "`path`, `namespace` and `cors`";

/// A literal passed through `macro_rules!` (`$x:literal`, `$x:expr`) arrives wrapped in an invisible group.
fn ungroup(mut expr: &Expr) -> &Expr {
    while let Expr::Group(group) = expr {
        expr = &group.expr;
    }
    expr
}

fn string_literal(value: &Expr, key: &str) -> syn::Result<String> {
    lit_str(ungroup(value))
        .ok_or_else(|| syn::Error::new(value.span(), format!("`{key}` of #[websocket_gateway] expects a string literal")))
}

fn check_namespace(namespace: &str, span: proc_macro2::Span) -> syn::Result<()> {
    let name = if namespace.starts_with('/') { namespace.to_string() } else { format!("/{namespace}") };
    if name.len() > MAX_NAMESPACE_LEN {
        return Err(syn::Error::new(span, format!("the namespace name is {} bytes long; the most is {MAX_NAMESPACE_LEN}", name.len())));
    }
    if name.contains(',') {
        return Err(syn::Error::new(span, "the namespace name must not contain a comma, the delimiter of the packet header"));
    }
    if name.chars().any(char::is_control) {
        return Err(syn::Error::new(span, "the namespace name must not contain a control character"));
    }
    Ok(())
}

fn parse_cors(value: &Expr) -> syn::Result<Option<Cors>> {
    let forms = "`cors` of #[websocket_gateway] takes `true`, `false`, an origin string or an array of origin strings";
    match ungroup(value) {
        Expr::Lit(l) => match &l.lit {
            syn::Lit::Bool(b) => Ok(if b.value { Some(Cors::Any) } else { None }),
            syn::Lit::Str(s) => Ok(Some(Cors::Origins(vec![s.value()]))),
            _ => Err(syn::Error::new(value.span(), forms)),
        },
        Expr::Array(a) => {
            let mut origins = Vec::new();
            for element in &a.elems {
                origins.push(lit_str(ungroup(element)).ok_or_else(|| syn::Error::new(element.span(), forms))?);
            }
            if origins.is_empty() {
                return Err(syn::Error::new(value.span(), "`cors` of #[websocket_gateway] needs at least one origin in the array"));
            }
            Ok(Some(Cors::Origins(origins)))
        }
        _ => Err(syn::Error::new(value.span(), forms)),
    }
}

/// An option given twice (the positional literal is the `namespace`) is an error: the second would silently win.
fn once(seen: &mut Vec<&'static str>, key: &'static str, span: proc_macro2::Span) -> syn::Result<()> {
    if seen.contains(&key) {
        return Err(syn::Error::new(
            span,
            format!("`{key}` is given more than once in #[websocket_gateway] (the positional literal counts as `namespace`)"),
        ));
    }
    seen.push(key);
    Ok(())
}

fn parse_options(exprs: &[Expr]) -> syn::Result<GatewayOptions> {
    let mut seen: Vec<&'static str> = Vec::new();
    let mut options = GatewayOptions::default();
    let mut ws_path = "/".to_string();
    for e in exprs {
        match ungroup(e) {
            Expr::Assign(a) => {
                let key = match &*a.left {
                    Expr::Path(p) => p.path.get_ident().map(|i| i.to_string()),
                    _ => None,
                };
                match key.as_deref() {
                    Some("path") => {
                        once(&mut seen, "path", a.left.span())?;
                        let value = string_literal(&a.right, "path")?;
                        ws_path = value.clone();
                        options.path = Some(value);
                    }
                    Some("namespace") => {
                        once(&mut seen, "namespace", a.left.span())?;
                        let value = string_literal(&a.right, "namespace")?;
                        check_namespace(&value, a.right.span())?;
                        ws_path = value.clone();
                        options.namespace = Some(value);
                    }
                    Some("cors") => {
                        once(&mut seen, "cors", a.left.span())?;
                        options.cors = parse_cors(&a.right)?;
                    }
                    Some(other) => {
                        return Err(syn::Error::new(
                            a.left.span(),
                            format!("unknown option `{other}` of #[websocket_gateway]; the accepted options are {ACCEPTED}"),
                        ));
                    }
                    None => {
                        return Err(syn::Error::new(
                            a.left.span(),
                            format!("unknown option of #[websocket_gateway]; the accepted options are {ACCEPTED}"),
                        ));
                    }
                }
            }
            Expr::Lit(_) => {
                once(&mut seen, "namespace", e.span())?;
                let value = string_literal(e, "the namespace")?;
                check_namespace(&value, e.span())?;
                ws_path = value.clone();
                options.namespace = Some(value);
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    format!("unexpected argument of #[websocket_gateway]; use a namespace literal or the options {ACCEPTED}"),
                ));
            }
        }
    }
    if !ws_path.starts_with('/') {
        ws_path = format!("/{ws_path}");
    }
    if ws_path.len() > 1 {
        ws_path = ws_path.trim_end_matches('/').to_string();
    }
    options.ws_path = ws_path;
    Ok(options)
}

fn option_tokens(value: &Option<String>) -> TokenStream {
    match value {
        Some(v) => quote!(::std::option::Option::Some(::std::string::String::from(#v))),
        None => quote!(::std::option::Option::None),
    }
}

fn cors_tokens(cors: &Option<Cors>) -> TokenStream {
    match cors {
        None => quote!(::std::option::Option::None),
        Some(Cors::Any) => quote!(::std::option::Option::Some(::corvis::discovery::CorsDecl::Any)),
        Some(Cors::Origins(list)) => {
            quote!(::std::option::Option::Some(::corvis::discovery::CorsDecl::Origins(
                ::std::vec![#(::std::string::String::from(#list)),*]
            )))
        }
    }
}

pub fn expand(args: TokenStream, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let exprs: Vec<Expr> = syn::parse::Parser::parse2(parser, args)?.into_iter().collect();
    let options = parse_options(&exprs)?;
    let path = options.ws_path.clone();
    let (io_namespace, io_path, io_cors) = (option_tokens(&options.namespace), option_tokens(&options.path), cors_tokens(&options.cors));

    // Class-level enhancers below `#[websocket_gateway]`.
    let mut enhancers = Vec::<TokenStream>::new();
    let mut keep = Vec::new();
    for attr in item.attrs.drain(..) {
        let n = attr_name(&attr);
        let builder = match n.as_str() {
            "use_guards" => "guard",
            "use_interceptors" => "interceptor",
            "use_pipes" => "pipe",
            "set_metadata" => {
                let args = attr_args(&attr)?;
                if args.len() != 2 {
                    return Err(syn::Error::new_spanned(&attr, "#[set_metadata(key, value)] expects two arguments"));
                }
                let (k, v) = (&args[0], &args[1]);
                enhancers.push(quote!(.set_metadata(#k, #v)));
                continue;
            }
            "use_filters" => "filter",
            "public" => {
                crate::controller::no_public_args(&attr)?;
                enhancers.push(quote!(.set_metadata(::corvis::PUBLIC_METADATA, true)));
                continue;
            }
            _ => {
                keep.push(attr);
                continue;
            }
        };
        let m = syn::Ident::new(builder, proc_macro2::Span::call_site());
        let tokens = if builder == "filter" {
            crate::utils::filter_tokens_in_try_order(attr_args(&attr)?)
        } else {
            enhancer_tokens(builder, attr_args(&attr)?)
        };
        for e in tokens {
            enhancers.push(quote!(.#m(#e)));
        }
    }
    item.attrs = keep;

    // Each `#[websocket_server]` field gets the gateway's own server handle (one per application): its type decides
    // which adapter it belongs to (`WsServer` or `IoGatewayServer`).
    let mut server_field: Option<(TokenStream, syn::Type)> = None;
    for (index, field) in item.fields.iter_mut().enumerate() {
        let mut is_server = false;
        field.attrs.retain(|a| {
            if attr_name(a) == "websocket_server" {
                is_server = true;
                false
            } else {
                true
            }
        });
        if is_server {
            let ty = &field.ty;
            let attr: syn::Attribute = syn::parse_quote!(#[init(<#ty as ::corvis::websockets::GatewayServerField>::__corvis_unbound())]);
            field.attrs.push(attr);
            if server_field.is_none() {
                let access = match &field.ident {
                    Some(id) => quote!(#id),
                    None => {
                        let i = syn::Index::from(index);
                        quote!(#i)
                    }
                };
                server_field = Some((access, field.ty.clone()));
            }
        }
    }
    let (server_tokens, server_kind_tokens) = match &server_field {
        Some((f, ty)) => (
            quote! {
                ::std::option::Option::Some(::std::sync::Arc::new(__this.#f.clone()) as ::std::sync::Arc<dyn ::std::any::Any + Send + Sync>)
            },
            quote!(::std::option::Option::Some(<#ty as ::corvis::websockets::GatewayServerField>::KIND)),
        ),
        None => (quote!(::std::option::Option::None), quote!(::std::option::Option::None)),
    };

    let injectable = injectable_impl(&mut item, &InjectableOptions::default())?;
    let exportable = export_impl(&item, false);
    let name = &item.ident;
    let (impl_g, ty_g, where_c) = item.generics.split_for_impl();
    Ok(quote! {
        #item
        #injectable
        #exportable
        impl #impl_g ::corvis::websockets::Gateway for #name #ty_g #where_c {
            fn __corvis_gateway(__this: ::std::sync::Arc<Self>) -> ::corvis::discovery::GatewayInfo {
                #[allow(unused_imports)]
                use ::corvis::websockets::hooks::*;
                let __p = ::corvis::__private::Probe::<Self>::new();
                ::corvis::discovery::GatewayInfo {
                    path: ::std::string::String::from(#path),
                    io: ::corvis::discovery::IoGatewayDecl { namespace: #io_namespace, path: #io_path, cors: #io_cors },
                    on_init: (&__p).__corvis_gw_init(__this.clone()),
                    on_connection: (&__p).__corvis_gw_connection(__this.clone()),
                    on_disconnect: (&__p).__corvis_gw_disconnect(__this.clone()),
                    io_on_init: (&__p).__corvis_gw_io_init(__this.clone()),
                    io_on_connection: (&__p).__corvis_gw_io_connection(__this.clone()),
                    io_on_disconnect: (&__p).__corvis_gw_io_disconnect(__this.clone()),
                    server: #server_tokens,
                    server_kind: #server_kind_tokens,
                    enhancers: ::corvis::discovery::WsEnhancers::default() #(#enhancers)*,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(tokens: TokenStream) -> syn::Result<GatewayOptions> {
        let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
        let exprs: Vec<Expr> = syn::parse::Parser::parse2(parser, tokens)?.into_iter().collect();
        parse_options(&exprs)
    }

    fn error(tokens: TokenStream) -> String {
        options(tokens).unwrap_err().to_string()
    }

    #[test]
    fn no_arguments_serve_the_root_path_and_declare_nothing_for_socket_io() {
        let o = options(quote!()).unwrap();
        assert_eq!(o, GatewayOptions { ws_path: "/".into(), ..GatewayOptions::default() });
    }

    #[test]
    fn the_literal_and_namespace_are_the_ws_path_and_the_socket_io_namespace() {
        for tokens in [quote!("/chat"), quote!(namespace = "/chat")] {
            let o = options(tokens).unwrap();
            assert_eq!(o.ws_path, "/chat");
            assert_eq!(o.namespace.as_deref(), Some("/chat"));
            assert_eq!(o.path, None);
        }
    }

    #[test]
    fn path_is_the_ws_path_and_the_engine_path_not_the_namespace() {
        let o = options(quote!(path = "/realtime")).unwrap();
        assert_eq!(o.ws_path, "/realtime");
        assert_eq!(o.path.as_deref(), Some("/realtime"));
        assert_eq!(o.namespace, None);
    }

    #[test]
    fn path_and_namespace_together_keep_both_and_the_last_one_is_the_ws_path() {
        let o = options(quote!(path = "/realtime", namespace = "/chat")).unwrap();
        assert_eq!((o.path.as_deref(), o.namespace.as_deref(), o.ws_path.as_str()), (Some("/realtime"), Some("/chat"), "/chat"));
        let o = options(quote!(namespace = "/chat", path = "/realtime")).unwrap();
        assert_eq!(o.ws_path, "/realtime");
    }

    #[test]
    fn the_ws_path_is_normalized_as_before() {
        assert_eq!(options(quote!("chat/")).unwrap().ws_path, "/chat");
        assert_eq!(options(quote!(path = "/a/b//")).unwrap().ws_path, "/a/b");
        assert_eq!(options(quote!("/")).unwrap().ws_path, "/");
    }

    #[test]
    fn cors_takes_true_false_a_string_or_an_array() {
        assert_eq!(options(quote!(cors = true)).unwrap().cors, Some(Cors::Any));
        assert_eq!(options(quote!(cors = false)).unwrap().cors, None);
        assert_eq!(options(quote!(cors = "https://a")).unwrap().cors, Some(Cors::Origins(vec!["https://a".into()])));
        assert_eq!(
            options(quote!(cors = ["https://a", "https://b"])).unwrap().cors,
            Some(Cors::Origins(vec!["https://a".into(), "https://b".into()]))
        );
    }

    #[test]
    fn a_cors_value_of_another_form_names_the_accepted_ones() {
        for tokens in [quote!(cors = 3), quote!(cors = [1]), quote!(cors = some_variable), quote!(cors = 'c')] {
            let message = error(tokens);
            assert!(message.contains("`true`, `false`, an origin string or an array of origin strings"), "{message}");
        }
        assert!(error(quote!(cors = [])).contains("at least one origin"));
    }

    #[test]
    fn an_unknown_option_names_the_accepted_ones() {
        let message = error(quote!(namspace = "/chat"));
        assert!(message.contains("namspace"), "{message}");
        assert!(message.contains("`path`, `namespace` and `cors`"), "{message}");
        assert!(error(quote!(ping_interval = 300)).contains("`path`, `namespace` and `cors`"));
    }

    #[test]
    fn a_bad_namespace_is_a_compile_error() {
        assert!(error(quote!(namespace = "/a,b")).contains("namespace name"));
        assert!(error(quote!("/a,b")).contains("comma"));
        assert!(error(quote!(namespace = "/a\nb")).contains("control character"));
        let long = format!("/{}", "a".repeat(256));
        assert!(error(quote!(namespace = #long)).contains("257 bytes"));
        let ok = format!("/{}", "a".repeat(255));
        assert!(options(quote!(namespace = #ok)).is_ok());
    }

    #[test]
    fn literals_that_came_through_macro_rules_are_accepted() {
        let namespace = proc_macro2::Group::new(proc_macro2::Delimiter::None, quote!("/chat"));
        let path = proc_macro2::Group::new(proc_macro2::Delimiter::None, quote!("/realtime"));
        let cors = proc_macro2::Group::new(proc_macro2::Delimiter::None, quote!(true));
        let o = options(quote!(namespace = #namespace, path = #path, cors = #cors)).unwrap();
        assert_eq!((o.namespace.as_deref(), o.path.as_deref(), o.cors), (Some("/chat"), Some("/realtime"), Some(Cors::Any)));
    }

    #[test]
    fn an_option_given_twice_is_an_error() {
        for tokens in [
            quote!(namespace = "/a", namespace = "/b"),
            quote!("/a", namespace = "/b"),
            quote!("/a", "/b"),
            quote!(path = "/a", path = "/b"),
            quote!(cors = true, cors = "https://a"),
        ] {
            assert!(error(tokens).contains("more than once"));
        }
        assert!(options(quote!("/a", path = "/b", cors = true)).is_ok());
    }

    #[test]
    fn options_that_are_not_literals_are_refused() {
        assert!(error(quote!(path = SOME_CONST)).contains("string literal"));
        assert!(error(quote!(namespace = 3)).contains("string literal"));
        assert!(error(quote!(5)).contains("string literal"));
        assert!(error(quote!(foo(1))).contains("unexpected argument"));
    }
}

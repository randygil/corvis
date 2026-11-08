//! `#[websocket_gateway(path = "/chat")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, ItemStruct};

use crate::injectable::{InjectableOptions, export_impl, injectable_impl};
use crate::utils::{attr_args, attr_name, enhancer_tokens, lit_str};

pub fn expand(args: TokenStream, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let exprs: Vec<Expr> = syn::parse::Parser::parse2(parser, args)?.into_iter().collect();
    let mut path = "/".to_string();
    for e in exprs {
        match &e {
            Expr::Assign(a) => {
                let key = match &*a.left {
                    Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
                    _ => String::new(),
                };
                if (key == "path" || key == "namespace")
                    && let Some(s) = lit_str(&a.right)
                {
                    path = s;
                }
            }
            Expr::Lit(_) => {
                if let Some(s) = lit_str(&e) {
                    path = s;
                }
            }
            _ => {}
        }
    }
    if !path.starts_with('/') {
        path = format!("/{path}");
    }
    if path.len() > 1 {
        path = path.trim_end_matches('/').to_string();
    }

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
        for e in enhancer_tokens(builder, attr_args(&attr)?) {
            enhancers.push(quote!(.#m(#e)));
        }
    }
    item.attrs = keep;

    // Each `#[websocket_server]` field gets the gateway's own `WsServer` (one per application).
    let mut server_field: Option<TokenStream> = None;
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
            let attr: syn::Attribute = syn::parse_quote!(#[init(::corvis::websockets::WsServer::new())]);
            field.attrs.push(attr);
            if server_field.is_none() {
                server_field = Some(match &field.ident {
                    Some(id) => quote!(#id),
                    None => {
                        let i = syn::Index::from(index);
                        quote!(#i)
                    }
                });
            }
        }
    }
    let server_tokens = match &server_field {
        Some(f) => quote! {
            ::std::option::Option::Some(::std::sync::Arc::new(__this.#f.clone()) as ::std::sync::Arc<dyn ::std::any::Any + Send + Sync>)
        },
        None => quote!(::std::option::Option::None),
    };

    let injectable = injectable_impl(&mut item, &InjectableOptions { transient: false })?;
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
                    on_init: (&__p).__corvis_gw_init(__this.clone()),
                    on_connection: (&__p).__corvis_gw_connection(__this.clone()),
                    on_disconnect: (&__p).__corvis_gw_disconnect(__this.clone()),
                    server: #server_tokens,
                    enhancers: ::corvis::discovery::WsEnhancers::default() #(#enhancers)*,
                }
            }
        }
    })
}

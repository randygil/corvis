//! `#[controller("cats")]` on a struct and `#[routes]` on its impl block.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, FnArg, ImplItem, ItemImpl, ItemStruct, ReturnType, Type};

use crate::injectable::{InjectableOptions, export_impl, injectable_impl};
use crate::utils::{attr_args, attr_name, enhancer_tokens, flatten_array, last_ident, lit_str};

fn version_tokens(exprs: Vec<Expr>) -> TokenStream {
    let exprs = flatten_array(exprs);
    if exprs.len() == 1 {
        if let Some(s) = lit_str(&exprs[0]) {
            return quote!(::corvis::VersionValue::single(#s));
        }
        let e = &exprs[0];
        return quote!(::corvis::VersionValue::from(#e));
    }
    let strs: Vec<TokenStream> = exprs.iter().map(|e| quote!(::std::string::String::from(#e))).collect();
    quote!(::corvis::VersionValue::Versions(::std::vec![#(#strs),*]))
}

fn metadata_tokens(args: &[Expr]) -> syn::Result<(TokenStream, TokenStream)> {
    if args.len() != 2 {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), "#[set_metadata(key, value)] expects two arguments"));
    }
    let k = &args[0];
    let v = &args[1];
    Ok((quote!(#k), quote!(#v)))
}

/// Custom decorators: `#[apply_decorators(a(), b())]` and any attribute whose name starts with
/// `api_` (`#[api_tags("cats")]` -> `.decorate(api_tags!("cats"))`, implemented by corvis-swagger).
fn decorator_tokens(attr: &syn::Attribute) -> syn::Result<Option<Vec<TokenStream>>> {
    let name = attr_name(attr);
    if name == "apply_decorators" {
        return Ok(Some(attr_args(attr)?.into_iter().map(|e| quote!(#e)).collect()));
    }
    if !name.starts_with("api_") {
        return Ok(None);
    }
    let path = attr.path();
    let tokens = match &attr.meta {
        syn::Meta::Path(_) => TokenStream::new(),
        syn::Meta::List(list) => list.tokens.clone(),
        syn::Meta::NameValue(nv) => {
            return Err(syn::Error::new_spanned(nv, "decorator attributes take arguments in parentheses: #[api_x(...)]"));
        }
    };
    Ok(Some(vec![quote!(#path!(#tokens))]))
}

/// `#[controller]`, `#[controller("cats")]`, `#[controller(["a", "b"])]`,
/// `#[controller(path = "cats", version = "1", host = "admin.example.com")]`.
pub fn expand_controller(args: TokenStream, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let exprs: Vec<Expr> = syn::parse::Parser::parse2(parser, args)?.into_iter().collect();

    let mut paths: Vec<Expr> = Vec::new();
    let mut builder = Vec::<TokenStream>::new();
    for e in exprs {
        match e {
            Expr::Assign(a) => {
                let key = quote!(#a.left).to_string();
                let key = match &*a.left {
                    Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or(key),
                    _ => key,
                };
                let right = *a.right;
                match key.as_str() {
                    "path" => paths.extend(flatten_array(vec![right])),
                    "version" => {
                        let v = version_tokens(vec![right]);
                        builder.push(quote!(.version(#v)));
                    }
                    "host" => builder.push(quote!(.host(#right))),
                    _ => return Err(syn::Error::new_spanned(right, "unknown #[controller] option (expected path, version or host)")),
                }
            }
            other => paths.extend(flatten_array(vec![other])),
        }
    }

    let mut keep = Vec::new();
    for attr in item.attrs.drain(..) {
        match attr_name(&attr).as_str() {
            "use_guards" => {
                for g in enhancer_tokens("guard", attr_args(&attr)?) {
                    builder.push(quote!(.guard(#g)));
                }
            }
            "use_interceptors" => {
                for i in enhancer_tokens("interceptor", attr_args(&attr)?) {
                    builder.push(quote!(.interceptor(#i)));
                }
            }
            "use_pipes" => {
                for p in enhancer_tokens("pipe", attr_args(&attr)?) {
                    builder.push(quote!(.pipe(#p)));
                }
            }
            "use_filters" => {
                for f in enhancer_tokens("filter", attr_args(&attr)?) {
                    builder.push(quote!(.filter(#f)));
                }
            }
            "set_metadata" => {
                let (k, v) = metadata_tokens(&attr_args(&attr)?)?;
                builder.push(quote!(.set_metadata(#k, #v)));
            }
            "version" => {
                let v = version_tokens(attr_args(&attr)?);
                builder.push(quote!(.version(#v)));
            }
            _ => match decorator_tokens(&attr)? {
                Some(ds) => builder.extend(ds.into_iter().map(|d| quote!(.decorate(#d)))),
                None => keep.push(attr),
            },
        }
    }
    item.attrs = keep;

    let path_strs: Vec<TokenStream> = paths.iter().map(|p| quote!(#p)).collect();
    let paths_tokens = if path_strs.is_empty() {
        quote!(::corvis::ControllerMetadata::new(""))
    } else {
        quote!(::corvis::ControllerMetadata::new("").paths(::std::vec![#(#path_strs),*]))
    };

    let injectable = injectable_impl(&mut item, &InjectableOptions { transient: false })?;
    let exportable = export_impl(&item, false);
    let name = &item.ident;
    let (impl_g, ty_g, where_c) = item.generics.split_for_impl();
    Ok(quote! {
        #item
        #injectable
        #exportable
        impl #impl_g ::corvis::ControllerBase for #name #ty_g #where_c {
            fn controller_metadata() -> ::corvis::ControllerMetadata {
                #paths_tokens #(#builder)*
            }
        }
    })
}

const METHODS: &[(&str, &str)] = &[
    ("get", "Get"),
    ("post", "Post"),
    ("put", "Put"),
    ("delete", "Delete"),
    ("patch", "Patch"),
    ("options", "Options"),
    ("head", "Head"),
    ("all", "All"),
    ("search", "Search"),
    ("sse", "Get"),
];

const PARAM_ATTRS: &[(&str, &str)] = &[
    ("body", "Body"),
    ("param", "Param"),
    ("query", "Query"),
    ("headers", "Headers"),
    ("header", "Headers"),
    ("ip", "Ip"),
    ("req", "Custom"),
    ("request", "Custom"),
    ("ctx", "Custom"),
    ("uploaded_file", "File"),
    ("uploaded_files", "Files"),
];

/// `#[routes] impl CatsController { #[get(":id")] async fn find_one(...) {} }`.
pub fn expand_routes(mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = item.self_ty.clone();
    let mut routes = Vec::new();

    for impl_item in item.items.iter_mut() {
        let ImplItem::Fn(method) = impl_item else { continue };

        let mut route: Option<(String, Vec<Expr>, bool)> = None;
        let mut builder = Vec::<TokenStream>::new();
        let mut keep = Vec::new();
        for attr in method.attrs.drain(..) {
            let name = attr_name(&attr);
            if let Some((_, variant)) = METHODS.iter().find(|(m, _)| *m == name) {
                let args = flatten_array(attr_args(&attr)?);
                route = Some((variant.to_string(), args, name == "sse"));
                continue;
            }
            match name.as_str() {
                "http_code" => {
                    let a = attr_args(&attr)?;
                    let c = &a[0];
                    builder.push(quote!(.http_code((#c) as u16)));
                }
                "header" => {
                    let a = attr_args(&attr)?;
                    if a.len() != 2 {
                        return Err(syn::Error::new_spanned(&attr, "#[header(name, value)] expects two arguments"));
                    }
                    let (k, v) = (&a[0], &a[1]);
                    builder.push(quote!(.header(#k, #v)));
                }
                "redirect" => {
                    let a = attr_args(&attr)?;
                    let url = &a[0];
                    let code = a.get(1).map(|c| quote!((#c) as u16)).unwrap_or(quote!(302u16));
                    builder.push(quote!(.redirect(#url, #code)));
                }
                "use_guards" => {
                    for g in enhancer_tokens("guard", attr_args(&attr)?) {
                        builder.push(quote!(.guard(#g)));
                    }
                }
                "use_interceptors" => {
                    for i in enhancer_tokens("interceptor", attr_args(&attr)?) {
                        builder.push(quote!(.interceptor(#i)));
                    }
                }
                "use_pipes" => {
                    for p in enhancer_tokens("pipe", attr_args(&attr)?) {
                        builder.push(quote!(.pipe(#p)));
                    }
                }
                "use_filters" => {
                    for f in enhancer_tokens("filter", attr_args(&attr)?) {
                        builder.push(quote!(.filter(#f)));
                    }
                }
                "set_metadata" => {
                    let (k, v) = metadata_tokens(&attr_args(&attr)?)?;
                    builder.push(quote!(.set_metadata(#k, #v)));
                }
                "version" => {
                    let v = version_tokens(attr_args(&attr)?);
                    builder.push(quote!(.version(#v)));
                }
                _ => match decorator_tokens(&attr)? {
                    Some(ds) => builder.extend(ds.into_iter().map(|d| quote!(.decorate(#d)))),
                    None => keep.push(attr),
                },
            }
        }
        method.attrs = keep;
        let Some((variant, paths, is_sse)) = route else { continue };
        if is_sse {
            builder.push(quote!(.sse()));
        }

        let fname = method.sig.ident.clone();
        let fname_str = fname.to_string();
        let is_async = method.sig.asyncness.is_some();
        let has_self = matches!(method.sig.inputs.first(), Some(FnArg::Receiver(_)));
        if !has_self {
            return Err(syn::Error::new_spanned(&method.sig, "route handlers must take `&self`"));
        }

        let mut param_defs = Vec::new();
        let mut arg_lets = Vec::new();
        let mut arg_names = Vec::new();
        for (i, input) in method.sig.inputs.iter_mut().enumerate().skip(1) {
            let FnArg::Typed(pt) = input else { continue };
            let ty: Type = (*pt.ty).clone();
            let arg_ident = format_ident!("__arg{}", i);
            let mut kind = "Custom".to_string();
            let mut data: Option<String> = None;
            let mut pipes: Vec<Expr> = Vec::new();
            let mut keep = Vec::new();
            for attr in pt.attrs.drain(..) {
                let n = attr_name(&attr);
                if n == "res" || n == "response" {
                    // `@Res()` / `@Res({ passthrough: true })`: `#[res]`, `#[res(passthrough)]`,
                    // `#[res(passthrough = true)]`.
                    let mut passthrough = false;
                    if !matches!(attr.meta, syn::Meta::Path(_)) {
                        for a in attr_args(&attr)? {
                            match &a {
                                Expr::Path(p) if p.path.is_ident("passthrough") => passthrough = true,
                                Expr::Assign(asg) if matches!(&*asg.left, Expr::Path(p) if p.path.is_ident("passthrough")) => {
                                    passthrough = match &*asg.right {
                                        Expr::Lit(syn::ExprLit { lit: syn::Lit::Bool(b), .. }) => b.value,
                                        other => return Err(syn::Error::new_spanned(other, "expected `true` or `false`")),
                                    };
                                }
                                other => {
                                    return Err(syn::Error::new_spanned(other, "expected #[res] or #[res(passthrough = true)]"));
                                }
                            }
                        }
                    }
                    let mode = if passthrough { quote!(Passthrough) } else { quote!(Handled) };
                    builder.push(quote!(.response_mode(::corvis::ResMode::#mode)));
                    kind = "Custom".to_string();
                    continue;
                }
                if let Some((_, k)) = PARAM_ATTRS.iter().find(|(a, _)| *a == n) {
                    kind = k.to_string();
                    let args = attr_args(&attr)?;
                    let mut it = args.into_iter().peekable();
                    if let Some(first) = it.peek() {
                        if let Some(s) = lit_str(first) {
                            data = Some(s);
                            it.next();
                        }
                    }
                    pipes.extend(it);
                } else {
                    keep.push(attr);
                }
            }
            pt.attrs = keep;
            if kind == "Headers" {
                data = data.map(|d| d.to_ascii_lowercase());
            }

            let kind_ident = format_ident!("{}", kind);
            let data_tokens = match &data {
                Some(d) => quote!(::std::option::Option::Some(#d)),
                None => quote!(::std::option::Option::None),
            };
            let ty_str = quote!(#ty).to_string().replace(' ', "");
            let pipe_tokens = enhancer_tokens("pipe", pipes);
            param_defs.push(quote! {
                .param(
                    ::corvis::ParamDef::new(
                        ::corvis::ParamKind::#kind_ident,
                        #data_tokens,
                        ::corvis::Metatype {
                            name: #ty_str,
                            validator: {
                                #[allow(unused_imports)]
                                use ::corvis::__private::{HasValidator as _, NoValidator as _};
                                (&::corvis::__private::Probe::<#ty>::new()).__corvis_validator()
                            },
                            zod: {
                                #[allow(unused_imports)]
                                use ::corvis::__private::{HasZod as _, NoZod as _};
                                (&::corvis::__private::Probe::<#ty>::new()).__corvis_zod()
                            },
                        },
                    )
                    .api_schema({
                        #[allow(unused_imports)]
                        use ::corvis::__private::{HasApiSchema as _, NoApiSchema as _};
                        (&::corvis::__private::Probe::<#ty>::new()).__corvis_api_schema()
                    })
                    #(.pipe(#pipe_tokens))*
                )
            });
            let name_tokens = match &data {
                Some(d) => quote!(::std::option::Option::Some(#d)),
                None => quote!(::std::option::Option::None),
            };
            if kind == "Custom" {
                arg_lets.push(quote! {
                    let #arg_ident: #ty = <#ty as ::corvis::FromContext>::from_context(&__ctx).await?;
                });
            } else {
                let idx = i - 1;
                arg_lets.push(quote! {
                    let #arg_ident: #ty = ::corvis::__private::extract_arg::<#ty>(&mut __args, #idx, ::corvis::ParamKind::#kind_ident, #name_tokens)?;
                });
            }
            arg_names.push(arg_ident);
        }

        let call = if is_async {
            quote!(__this.#fname(#(#arg_names),*).await)
        } else {
            quote!(__this.#fname(#(#arg_names),*))
        };

        let is_result = match &method.sig.output {
            ReturnType::Default => false,
            ReturnType::Type(_, ty) => last_ident(ty).map(|i| i.ends_with("Result")).unwrap_or(false),
        };
        let convert = if is_result {
            quote! {
                match __ret {
                    ::std::result::Result::Ok(__v) => (&__v).__corvis_reply_kind().convert(__v),
                    ::std::result::Result::Err(__e) => ::std::result::Result::Err((&__e).__corvis_exc_kind().convert(__e)),
                }
            }
        } else {
            quote!((&__ret).__corvis_reply_kind().convert(__ret))
        };

        let variant_ident = format_ident!("{}", variant);
        let first_path = paths.first().map(|p| quote!(#p)).unwrap_or(quote!(""));
        let extra_paths = if paths.len() > 1 {
            let ps: Vec<TokenStream> = paths.iter().map(|p| quote!(#p)).collect();
            quote!(.paths(::std::vec![#(#ps),*]))
        } else {
            quote!()
        };

        routes.push(quote! {
            ::corvis::RouteDef::new(
                ::corvis::RequestMethod::#variant_ident,
                #first_path,
                #fname_str,
                ::std::sync::Arc::new(|__this: ::std::sync::Arc<Self>, __ctx: ::corvis::ExecutionContext, mut __args: ::std::vec::Vec<::corvis::__private::serde_json::Value>| -> ::corvis::__private::BoxFuture<'static, ::corvis::HandlerResult> {
                    ::std::boxed::Box::pin(async move {
                        #[allow(unused_imports)]
                        use ::corvis::__private::{ReplyKind as _, SerializeKind as _, ExcKind as _, AnyhowKind as _};
                        let _ = &__ctx;
                        let _ = &mut __args;
                        #(#arg_lets)*
                        let __ret = #call;
                        #convert
                    })
                }),
            )
            #extra_paths
            #(#param_defs)*
            #(#builder)*
        });
    }

    let (impl_g, _, where_c) = item.generics.split_for_impl();
    Ok(quote! {
        #item

        impl #impl_g ::corvis::Controller for #self_ty #where_c {
            fn routes() -> ::std::vec::Vec<::corvis::RouteDef<Self>> {
                ::std::vec![#(#routes),*]
            }
        }
    })
}

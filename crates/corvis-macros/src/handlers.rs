//! `#[handlers]` on a provider impl: `#[on_event]`, `#[cron]`, `#[interval]`, `#[timeout]`,
//! `#[subscribe_message]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, FnArg, ImplItem, ItemImpl, ReturnType, Type};

use crate::utils::{attr_args, attr_name, enhancer_tokens, flatten_array, last_ident, lit_str};

fn name_and_value(args: Vec<Expr>) -> (Option<TokenStream>, Option<Expr>) {
    let mut name: Option<Expr> = None;
    let mut positional = Vec::new();
    for a in args {
        match &a {
            Expr::Assign(assign) if matches!(&*assign.left, Expr::Path(p) if p.path.is_ident("name")) => {
                name = Some((*assign.right).clone());
            }
            _ => positional.push(a),
        }
    }
    // `#[interval("name", 1000)]` (name first) vs `#[cron(EXPR, "name")]` / `#[interval(1000)]`.
    let value = if positional.len() >= 2 && lit_str(&positional[0]).is_some() && lit_str(&positional[1]).is_none() {
        name = name.or(Some(positional[0].clone()));
        Some(positional[1].clone())
    } else {
        if positional.len() >= 2 {
            name = name.or(Some(positional[1].clone()));
        }
        positional.first().cloned()
    };
    (name.map(|n| quote!(::std::string::String::from(#n))), value)
}

/// `#[cron(EXPR)]`, `#[cron(EXPR, "name")]`, `#[cron(EXPR, OPTIONS_EXPR)]` and named options
/// (`name`, `time_zone`, `utc_offset`, `disabled`, `wait_for_completion`, `threshold`, `initial_delay`,
/// `unref_timeout`) -> (cron time, `CronOptions` expression).
fn cron_options(sig: &syn::Signature, args: Vec<Expr>) -> syn::Result<(Expr, TokenStream)> {
    let mut positional = Vec::new();
    let mut setters = Vec::new();
    const KEYS: [&str; 8] =
        ["name", "time_zone", "utc_offset", "disabled", "wait_for_completion", "threshold", "initial_delay", "unref_timeout"];
    for a in args {
        if let Expr::Assign(assign) = &a
            && let Expr::Path(p) = &*assign.left
            && let Some(id) = p.path.get_ident()
        {
            let key = id.to_string();
            if !KEYS.contains(&key.as_str()) {
                return Err(syn::Error::new_spanned(id, format!("unknown #[cron] option `{key}` (expected one of: {})", KEYS.join(", "))));
            }
            let v = &assign.right;
            let m = format_ident!("{}", key);
            setters.push(quote!(.#m(#v)));
            continue;
        }
        positional.push(a);
    }
    let mut it = positional.into_iter();
    let cron_time = it.next().ok_or_else(|| syn::Error::new_spanned(sig, "#[cron] requires a cron expression"))?;
    let base = match it.next() {
        None => quote!(::corvis::schedule::CronOptions::new()),
        Some(e) if lit_str(&e).is_some() => quote!(::corvis::schedule::CronOptions::new().name(#e)),
        Some(e) => quote!(::corvis::schedule::CronOptions::clone(&(#e))),
    };
    if let Some(extra) = it.next() {
        return Err(syn::Error::new_spanned(extra, "#[cron] takes at most two positional arguments"));
    }
    Ok((cron_time, quote!(#base #(#setters)*)))
}

fn is_result(output: &ReturnType) -> bool {
    match output {
        ReturnType::Default => false,
        ReturnType::Type(_, ty) => last_ident(ty).map(|i| i.ends_with("Result")).unwrap_or(false),
    }
}

pub fn expand(mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = item.self_ty.clone();
    let class_name = last_ident(&self_ty).unwrap_or_default();
    let mut entries = Vec::new();
    // Errors for enhancers on methods without a handler attribute; reported together after the loop.
    let mut discarded: Option<syn::Error> = None;

    for impl_item in item.items.iter_mut() {
        let ImplItem::Fn(method) = impl_item else { continue };
        let mut kind: Option<(String, Vec<Expr>)> = None;
        let mut keep = Vec::new();
        // `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]` (WebSocket handlers).
        let mut ws_enhancers = Vec::<TokenStream>::new();
        let mut enhancer_attr: Option<syn::Attribute> = None;
        for attr in method.attrs.drain(..) {
            let n = attr_name(&attr);
            let builder = match n.as_str() {
                "use_guards" => Some("guard"),
                "use_interceptors" => Some("interceptor"),
                "use_pipes" => Some("pipe"),
                "use_filters" => Some("filter"),
                _ => None,
            };
            if ["on_event", "cron", "interval", "timeout", "subscribe_message"].contains(&n.as_str()) {
                kind = Some((n, attr_args(&attr)?));
            } else if let Some(b) = builder {
                let m = format_ident!("{}", b);
                let tokens = if b == "filter" {
                    crate::utils::filter_tokens_in_try_order(attr_args(&attr)?)
                } else {
                    enhancer_tokens(b, attr_args(&attr)?)
                };
                for e in tokens {
                    ws_enhancers.push(quote!(.#m(#e)));
                }
                enhancer_attr.get_or_insert(attr);
            } else if n == "public" {
                crate::controller::no_public_args(&attr)?;
                ws_enhancers.push(quote!(.set_metadata(::corvis::PUBLIC_METADATA, true)));
                enhancer_attr.get_or_insert(attr);
            } else if n == "set_metadata" {
                let a = attr_args(&attr)?;
                if a.len() != 2 {
                    return Err(syn::Error::new_spanned(&attr, "#[set_metadata(key, value)] expects two arguments"));
                }
                let (k, v) = (&a[0], &a[1]);
                ws_enhancers.push(quote!(.set_metadata(#k, #v)));
                enhancer_attr.get_or_insert(attr);
            } else {
                keep.push(attr);
            }
        }
        method.attrs = keep;
        let Some((kind, args)) = kind else {
            if let Some(attr) = enhancer_attr {
                let message = format!(
                    "`{}` has #[{}] but no #[subscribe_message] attribute; enhancers and metadata in `#[handlers]` only apply to message handlers",
                    method.sig.ident,
                    attr_name(&attr)
                );
                let error = syn::Error::new_spanned(&attr, message);
                match &mut discarded {
                    Some(all) => all.combine(error),
                    None => discarded = Some(error),
                }
            }
            continue;
        };
        if let Some(attr) = &enhancer_attr
            && kind != "subscribe_message"
        {
            return Err(syn::Error::new_spanned(attr, "enhancers in `#[handlers]` are only supported on #[subscribe_message] methods"));
        }
        let fname = method.sig.ident.clone();
        let fname_str = fname.to_string();
        let awaited = if method.sig.asyncness.is_some() { quote!(.await) } else { quote!() };
        let result = is_result(&method.sig.output);

        // Typed parameters (excluding self), with their attributes stripped.
        // `#[message_body("prop", Pipe, ...)]`: the property of the payload and the pipes.
        let mut params: Vec<(Type, String, Option<String>, Vec<Expr>)> = Vec::new();
        for input in method.sig.inputs.iter_mut() {
            if let FnArg::Typed(pt) = input {
                let mut role = String::from("body");
                let mut data: Option<String> = None;
                let mut pipes: Vec<Expr> = Vec::new();
                let mut keep = Vec::new();
                for a in pt.attrs.drain(..) {
                    let n = attr_name(&a);
                    if n == "connected_socket" {
                        role = "socket".into();
                    } else if n == "message_body" || n == "payload" {
                        role = "body".into();
                        let mut it = attr_args(&a)?.into_iter().peekable();
                        if let Some(s) = it.peek().and_then(lit_str) {
                            data = Some(s);
                            it.next();
                        }
                        pipes.extend(it);
                    } else {
                        keep.push(a);
                    }
                }
                pt.attrs = keep;
                params.push(((*pt.ty).clone(), role, data, pipes));
            }
        }

        let task_conv = if result { quote!(__ret.map_err(|__e| ::corvis::anyhow::anyhow!("{}", __e))?;) } else { quote!(let _ = __ret;) };

        let entry = match kind.as_str() {
            "on_event" => {
                let events: Vec<TokenStream> = flatten_array(args).iter().map(|e| quote!(::std::string::String::from(#e))).collect();
                let (bind, call_args) = match params.first() {
                    Some((ty, ..)) => (
                        quote! {
                            let ::std::option::Option::Some(__a) = ::corvis::modules::event_emitter::payload_as::<#ty>(&*__payload) else {
                                return ::std::result::Result::Ok(());
                            };
                        },
                        quote!(__a),
                    ),
                    None => (quote!(let _ = &__payload;), quote!()),
                };
                quote! {
                    ::corvis::discovery::HandlerKind::Event {
                        events: ::std::vec![#(#events),*],
                        call: {
                            let __this = __this.clone();
                            ::std::sync::Arc::new(move |__payload: ::corvis::discovery::Payload| {
                                let __this = __this.clone();
                                ::std::boxed::Box::pin(async move {
                                    #bind
                                    let __ret = __this.#fname(#call_args) #awaited;
                                    #task_conv
                                    ::std::result::Result::Ok(())
                                })
                            })
                        },
                    }
                }
            }
            "cron" | "interval" | "timeout" => {
                let call = quote! {
                    {
                        let __this = __this.clone();
                        ::std::sync::Arc::new(move || {
                            let __this = __this.clone();
                            ::std::boxed::Box::pin(async move {
                                let __ret = __this.#fname() #awaited;
                                #task_conv
                                ::std::result::Result::Ok(())
                            })
                        })
                    }
                };
                if kind == "cron" {
                    let options = cron_options(&method.sig, args)?;
                    let (cron_time, options) = options;
                    quote! {
                        ::corvis::discovery::HandlerKind::Cron {
                            cron_time: ::corvis::schedule::CronSource::from(#cron_time),
                            options: #options,
                            call: #call,
                        }
                    }
                } else {
                    let (name, value) = name_and_value(args);
                    let value = value.ok_or_else(|| syn::Error::new_spanned(&method.sig, format!("#[{kind}] requires a value")))?;
                    let name = name.map(|n| quote!(::std::option::Option::Some(#n))).unwrap_or(quote!(::std::option::Option::None));
                    if kind == "interval" {
                        quote! {
                            ::corvis::discovery::HandlerKind::Interval { millis: (#value) as u64, name: #name, call: #call }
                        }
                    } else {
                        quote! {
                            ::corvis::discovery::HandlerKind::Timeout { millis: (#value) as u64, name: #name, call: #call }
                        }
                    }
                }
            }
            _ => {
                // subscribe_message
                let event = args
                    .first()
                    .cloned()
                    .ok_or_else(|| syn::Error::new_spanned(&method.sig, "#[subscribe_message(\"event\")] requires an event name"))?;
                let mut lets = Vec::new();
                let mut names = Vec::new();
                let mut body_index = 0usize;
                for (i, (ty, role, data, pipes)) in params.iter().enumerate() {
                    let id = format_ident!("__a{}", i);
                    if role == "socket" {
                        lets.push(quote!(let #id: #ty = ::corvis::websockets::client_from_any(&__client)?;));
                    } else {
                        let data_tokens = match data {
                            Some(d) => quote!(::std::option::Option::Some(#d)),
                            None => quote!(::std::option::Option::None),
                        };
                        let ty_str = quote!(#ty).to_string().replace(' ', "");
                        let pipe_tokens = enhancer_tokens("pipe", pipes.clone());
                        ws_enhancers.push(quote! {
                            .param(
                                ::corvis::ParamDef::new(
                                    ::corvis::ParamKind::Body,
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
                                #(.pipe(#pipe_tokens))*
                            )
                        });
                        lets.push(quote! {
                            let #id: #ty = ::corvis::__private::extract_arg::<#ty>(&mut __args, #body_index, ::corvis::ParamKind::Body, #data_tokens)?;
                        });
                        body_index += 1;
                    }
                    names.push(id);
                }
                let conv = if result {
                    quote! {
                        match __ret {
                            ::std::result::Result::Ok(__v) => ::std::result::Result::Ok((&__v).__corvis_ws_kind().convert(__v)),
                            ::std::result::Result::Err(__e) => ::std::result::Result::Err((&__e).__corvis_exc_kind().convert(__e)),
                        }
                    }
                } else {
                    quote!(::std::result::Result::Ok((&__ret).__corvis_ws_kind().convert(__ret)))
                };
                quote! {
                    ::corvis::discovery::HandlerKind::WsMessage {
                        event: ::std::string::String::from(#event),
                        call: {
                            let __this = __this.clone();
                            ::std::sync::Arc::new(move |__client: ::std::sync::Arc<dyn ::std::any::Any + Send + Sync>, mut __args: ::std::vec::Vec<::corvis::__private::serde_json::Value>| {
                                let __this = __this.clone();
                                ::std::boxed::Box::pin(async move {
                                    #[allow(unused_imports)]
                                    use ::corvis::websockets::{WsReplyKind as _, WsSerializeKind as _};
                                    #[allow(unused_imports)]
                                    use ::corvis::__private::{ExcKind as _, AnyhowKind as _};
                                    let _ = (&__client, &mut __args);
                                    #(#lets)*
                                    let __ret = __this.#fname(#(#names),*) #awaited;
                                    #conv
                                })
                            })
                        },
                        enhancers: ::corvis::discovery::WsEnhancers::default() #(#ws_enhancers)*,
                    }
                }
            }
        };
        entries.push(quote! {
            __out.push(::corvis::discovery::DiscoveredHandler {
                class_name: #class_name,
                method_name: #fname_str,
                kind: #entry,
            });
        });
    }

    if let Some(error) = discarded {
        return Err(error);
    }

    let (impl_g, _, where_c) = item.generics.split_for_impl();
    Ok(quote! {
        #item

        impl #impl_g ::corvis::discovery::HandlersProvider for #self_ty #where_c {
            fn __corvis_handlers(__this: ::std::sync::Arc<Self>) -> ::std::vec::Vec<::corvis::discovery::DiscoveredHandler> {
                let mut __out = ::std::vec::Vec::new();
                #(#entries)*
                let _ = &__this;
                __out
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::expand;

    fn messages(item: syn::ItemImpl) -> Vec<String> {
        match expand(item) {
            Ok(_) => Vec::new(),
            Err(e) => e.into_iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn handler_filters_are_emitted_last_listed_first_and_stacked_attributes_in_source_order() {
        let code = expand(syn::parse_quote! {
            impl Gw {
                #[subscribe_message("x")]
                #[use_filters(FilterA, FilterB)]
                #[use_filters(FilterC)]
                fn x(&self) {}
            }
        })
        .unwrap()
        .to_string();
        let at = |n: &str| code.find(&format!("filter_of :: < {n} >")).unwrap_or_else(|| panic!("{n} missing in {code}"));
        assert!(at("FilterB") < at("FilterA") && at("FilterA") < at("FilterC"), "{code}");
    }

    #[test]
    fn guard_on_helper_is_rejected() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[use_guards(WsAuthGuard)] fn kick_idle(&self) {} }
        });
        assert_eq!(
            m,
            [
                "`kick_idle` has #[use_guards] but no #[subscribe_message] attribute; enhancers and metadata in `#[handlers]` only apply to message handlers"
            ]
        );
    }

    #[test]
    fn metadata_on_helper_is_rejected() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[set_metadata(K, 1)] fn helper(&self) {} }
        });
        assert_eq!(m.len(), 1);
        assert!(m[0].starts_with("`helper` has #[set_metadata] but no #[subscribe_message] attribute"));
    }

    #[test]
    fn path_attribute_reads_as_last_segment() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[corvis::use_pipes(P)] fn helper(&self) {} }
        });
        assert!(m[0].starts_with("`helper` has #[use_pipes] but"));
    }

    #[test]
    fn plain_helpers_are_accepted() {
        assert!(
            messages(syn::parse_quote! {
                impl Gw { fn plain(&self) {} #[allow(dead_code)] fn other(&self) {} }
            })
            .is_empty()
        );
    }

    #[test]
    fn filter_on_an_event_handler_is_rejected() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[use_filters(F)] #[on_event("x")] fn on_x(&self) {} }
        });
        assert_eq!(m, ["enhancers in `#[handlers]` are only supported on #[subscribe_message] methods"]);
    }

    #[test]
    fn filter_on_a_subscribe_message_is_accepted() {
        assert!(
            messages(syn::parse_quote! {
                impl Gw { #[use_filters(F)] #[subscribe_message("x")] async fn on_x(&self) {} }
            })
            .is_empty()
        );
    }

    #[test]
    fn filter_on_a_helper_is_rejected() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[use_filters(F)] fn helper(&self) {} }
        });
        assert!(m[0].starts_with("`helper` has #[use_filters] but no #[subscribe_message] attribute"));
    }

    #[test]
    fn enhancer_on_another_handler_kind_keeps_its_message() {
        let m = messages(syn::parse_quote! {
            impl Gw { #[use_guards(G)] #[on_event("x")] fn on_x(&self) {} }
        });
        assert_eq!(m, ["enhancers in `#[handlers]` are only supported on #[subscribe_message] methods"]);
    }

    #[test]
    fn subscribe_message_with_enhancer_is_accepted() {
        assert!(
            messages(syn::parse_quote! {
                impl Gw { #[use_guards(G)] #[subscribe_message("kick")] fn kick(&self) {} }
            })
            .is_empty()
        );
    }

    #[test]
    fn all_offending_methods_are_reported() {
        let m = messages(syn::parse_quote! {
            impl Gw {
                #[use_guards(A)] fn a(&self) {}
                #[use_pipes(P)] fn b(&self) {}
            }
        });
        assert_eq!(m.len(), 2);
        assert!(m[0].starts_with("`a` has"));
        assert!(m[1].starts_with("`b` has"));
    }
}

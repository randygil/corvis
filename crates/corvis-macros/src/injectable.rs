//! Code generation for `#[injectable]` (also used by `#[controller]` and `#[module]`).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, Fields, ItemStruct, Type};

use crate::utils::{attr_args, attr_name, generic_inner, lit_str, type_display};

pub struct InjectableOptions {
    pub transient: bool,
}

/// Removes `#[inject]`, `#[init]`, `#[default]` from fields and returns the `impl Injectable` block.
/// `#[optional]` is rejected with a compile error: optionality comes from the `Option<Arc<T>>` field type.
pub fn injectable_impl(item: &mut ItemStruct, opts: &InjectableOptions) -> syn::Result<TokenStream> {
    let name = &item.ident;
    let name_str = name.to_string();
    let (impl_g, ty_g, where_c) = item.generics.split_for_impl();

    let mut inits = Vec::new();
    let mut deps = Vec::new();
    let mut dep_items: Vec<TokenStream> = Vec::new();
    let is_named = matches!(item.fields, Fields::Named(_));
    let is_unit = matches!(item.fields, Fields::Unit);

    for (i, field) in item.fields.iter_mut().enumerate() {
        let mut token: Option<Expr> = None;
        let mut init: Option<TokenStream> = None;
        let mut keep = Vec::new();
        for attr in field.attrs.drain(..) {
            match attr_name(&attr).as_str() {
                "inject" => {
                    let args = attr_args(&attr)?;
                    token = args.into_iter().next();
                }
                "optional" => {
                    return Err(syn::Error::new_spanned(
                        &attr,
                        "#[optional] has no effect: declare the field as Option<Arc<T>> to make the dependency optional",
                    ));
                }
                "init" => {
                    let args = attr_args(&attr)?;
                    let e = args.into_iter().next().ok_or_else(|| syn::Error::new_spanned(&attr, "#[init(expr)] expects an expression"))?;
                    init = Some(quote!(#e));
                }
                "default" => init = Some(quote!(::std::default::Default::default())),
                _ => keep.push(attr),
            }
        }
        field.attrs = keep;
        let ty = &field.ty;
        let index = i;
        deps.push(match (&token, &init) {
            (_, Some(_)) => "-".to_string(),
            (Some(t), _) => lit_str(t).map(|s| format!("\"{s}\"")).unwrap_or_else(|| quote!(#t).to_string()),
            _ => type_display(ty),
        });
        // The static description of the field for the startup validation of transient providers: it dispatches
        // exactly like the construction below, so the two cannot disagree.
        let injectable_shape =
            generic_inner(ty, "Arc").is_some() || generic_inner(ty, "Option").and_then(|t| generic_inner(t, "Arc")).is_some();
        match (&token, &init) {
            (_, Some(_)) => {}
            (Some(tok), None) if injectable_shape => dep_items.push(quote! {
                (&::corvis::__private::Probe::<#ty>::new())
                    .__corvis_inject_kind()
                    .dependency::<#ty>(::std::option::Option::Some(::corvis::Token::from(#tok)), #index)
            }),
            (Some(tok), None) => dep_items.push(quote! {
                ::std::option::Option::Some(::corvis::di::Dependency { token: ::corvis::Token::from(#tok), index: #index, optional: false })
            }),
            (None, None) => dep_items.push(quote! {
                (&::corvis::__private::Probe::<#ty>::new())
                    .__corvis_inject_kind()
                    .dependency::<#ty>(::std::option::Option::None, #index)
            }),
        }
        let value = if let Some(init) = init {
            init
        } else if let Some(tok) = token {
            if injectable_shape {
                quote! {
                    (&::corvis::__private::Probe::<#ty>::new())
                        .__corvis_inject_kind()
                        .resolve::<#ty>(__ctx.clone(), ::std::option::Option::Some(::corvis::Token::from(#tok)), #index)
                        .await?
                }
            } else {
                quote! {
                    ::corvis::__private::inject_cloned::<#ty>(__ctx.clone(), ::corvis::Token::from(#tok), #index).await?
                }
            }
        } else {
            quote! {
                (&::corvis::__private::Probe::<#ty>::new())
                    .__corvis_inject_kind()
                    .resolve::<#ty>(__ctx.clone(), ::std::option::Option::None, #index)
                    .await?
            }
        };
        if is_named {
            let ident = field.ident.clone().unwrap();
            inits.push(quote!(#ident: #value));
        } else {
            let _ = format_ident!("_{}", i);
            inits.push(value);
        }
    }

    let construct = if is_unit {
        quote!(Self)
    } else if is_named {
        quote!(Self { #(#inits),* })
    } else {
        quote!(Self( #(#inits),* ))
    };

    let scope = if opts.transient {
        quote! { fn scope() -> ::corvis::Scope { ::corvis::Scope::Transient } }
    } else {
        quote! {}
    };

    let deps_lits: Vec<TokenStream> = deps.iter().map(|d| quote!(#d)).collect();
    let _: Option<&Type> = None;

    Ok(quote! {
        impl #impl_g ::corvis::Injectable for #name #ty_g #where_c {
            fn construct(__ctx: ::corvis::ResolveCtx) -> ::corvis::__private::BoxFuture<'static, ::std::result::Result<Self, ::corvis::DiError>> {
                ::std::boxed::Box::pin(async move {
                    #[allow(unused_imports)]
                    use ::corvis::__private::{InjectKind as _, DefaultKind as _};
                    #[allow(unused_variables)]
                    let __ctx = __ctx.with_dependencies(#name_str, &[#(#deps_lits),*]);
                    ::std::result::Result::Ok(#construct)
                })
            }

            fn dependencies() -> ::std::option::Option<::corvis::di::Dependencies> {
                #[allow(unused_imports)]
                use ::corvis::__private::{InjectKind as _, DefaultKind as _};
                #[allow(unused_mut)]
                let mut items = ::std::vec::Vec::new();
                #(items.extend(#dep_items);)*
                ::std::option::Option::Some(::corvis::di::Dependencies { requester: #name_str, names: &[#(#deps_lits),*], items })
            }

            fn hooks() -> ::corvis::Hooks<Self> {
                #[allow(unused_imports)]
                use ::corvis::__private::hooks::*;
                let __p = ::corvis::__private::Probe::<Self>::new();
                ::corvis::Hooks {
                    on_module_init: (&__p).__corvis_on_module_init(),
                    on_application_bootstrap: (&__p).__corvis_on_application_bootstrap(),
                    on_module_destroy: (&__p).__corvis_on_module_destroy(),
                    before_application_shutdown: (&__p).__corvis_before_application_shutdown(),
                    on_application_shutdown: (&__p).__corvis_on_application_shutdown(),
                    handlers: (&__p).__corvis_handlers_fn(),
                    gateway: (&__p).__corvis_gateway_fn(),
                }
            }

            #scope
        }

        impl #impl_g ::corvis::__private::ProviderItem for #name #ty_g #where_c {
            fn __corvis_provider() -> ::corvis::Provider {
                ::corvis::Provider::class::<Self>()
            }
        }
    })
}

pub fn export_impl(item: &ItemStruct, module: bool) -> TokenStream {
    let name = &item.ident;
    let name_str = name.to_string();
    let (impl_g, ty_g, where_c) = item.generics.split_for_impl();
    let body = if module {
        quote!(::corvis::__private::module_export::<Self>(#name_str))
    } else {
        quote!(::corvis::__private::token_export::<Self>())
    };
    quote! {
        impl #impl_g ::corvis::__private::Exportable for #name #ty_g #where_c {
            fn __corvis_export() -> ::corvis::ExportItem { #body }
        }
    }
}

pub fn parse_injectable_args(args: TokenStream) -> syn::Result<InjectableOptions> {
    let mut opts = InjectableOptions { transient: false };
    if args.is_empty() {
        return Ok(opts);
    }
    let parser = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated;
    let exprs = syn::parse::Parser::parse2(parser, args)?;
    for e in exprs {
        let s = quote!(#e).to_string().replace(' ', "").to_lowercase();
        if s.contains("transient") {
            opts.transient = true;
        } else if s.contains("request") {
            return Err(syn::Error::new_spanned(e, "request-scoped providers are not supported; use transient or default scope"));
        }
    }
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mut item: ItemStruct) -> syn::Result<TokenStream> {
        injectable_impl(&mut item, &InjectableOptions { transient: false })
    }

    #[test]
    fn optional_attribute_is_rejected() {
        let item: ItemStruct = syn::parse_quote! {
            struct NeedsCats {
                #[optional]
                cats: Arc<CatsService>,
            }
        };
        let err = run(item).unwrap_err().to_string();
        assert!(err.contains("Option<Arc<T>>"), "{err}");
    }

    #[test]
    fn optional_by_type_is_accepted() {
        let item: ItemStruct = syn::parse_quote! {
            struct NeedsCats {
                cats: Option<Arc<CatsService>>,
            }
        };
        assert!(run(item).is_ok());
    }
}

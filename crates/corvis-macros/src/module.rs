//! `#[module(imports = [...], controllers = [...], providers = [...], exports = [...], global)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, ItemStruct, Lit, Token};

use crate::injectable::{InjectableOptions, export_impl, injectable_impl};
use crate::utils::{attr_name, flatten_array, type_path};

#[derive(Default)]
pub struct ModuleArgs {
    imports: Vec<Expr>,
    controllers: Vec<Expr>,
    providers: Vec<Expr>,
    exports: Vec<Expr>,
    global: bool,
}

impl Parse for ModuleArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ModuleArgs::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let key_s = key.to_string();
            if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                let value: Expr = input.parse()?;
                let list = match value {
                    Expr::Array(a) => a.elems.into_iter().collect::<Vec<_>>(),
                    other => vec![other],
                };
                match key_s.as_str() {
                    "imports" => args.imports.extend(list),
                    "controllers" => args.controllers.extend(list),
                    "providers" => args.providers.extend(list),
                    "exports" => args.exports.extend(list),
                    "global" => {
                        args.global = matches!(list.first(), Some(Expr::Lit(l)) if matches!(&l.lit, Lit::Bool(b) if b.value));
                    }
                    _ => return Err(syn::Error::new_spanned(key, "unknown #[module] key; expected imports, controllers, providers, exports or global")),
                }
            } else if key_s == "global" {
                args.global = true;
            } else {
                return Err(syn::Error::new_spanned(key, "expected `key = [...]`"));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

pub fn expand(args: TokenStream, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let mut args: ModuleArgs = syn::parse2(args)?;
    // `#[global]` attribute (like `@Global()`).
    item.attrs.retain(|a| {
        if attr_name(a) == "global" {
            args.global = true;
            false
        } else {
            true
        }
    });

    let injectable = injectable_impl(&mut item, &InjectableOptions { transient: false })?;
    let name = &item.ident;
    let name_str = name.to_string();
    let exportable = export_impl(&item, true);

    let imports: Vec<TokenStream> = flatten_array(args.imports)
        .into_iter()
        .map(|e| match type_path(&e) {
            Some(p) => quote!(::corvis::ModuleImport::of::<#p>()),
            None => quote!(::corvis::ModuleImport::from(#e)),
        })
        .collect();
    let providers: Vec<TokenStream> = flatten_array(args.providers)
        .into_iter()
        .map(|e| match type_path(&e) {
            Some(p) => quote!(<#p as ::corvis::__private::ProviderItem>::__corvis_provider()),
            None => quote!(::std::convert::Into::<::corvis::Provider>::into(#e)),
        })
        .collect();
    let controllers: Vec<TokenStream> = flatten_array(args.controllers)
        .into_iter()
        .map(|e| match type_path(&e) {
            Some(p) => quote!(::corvis::ControllerDef::of::<#p>()),
            None => quote!(#e),
        })
        .collect();
    let exports: Vec<TokenStream> = flatten_array(args.exports)
        .into_iter()
        .map(|e| match type_path(&e) {
            Some(p) => quote!(<#p as ::corvis::__private::Exportable>::__corvis_export()),
            None => quote!(::corvis::ExportItem::from(#e)),
        })
        .collect();
    let global = args.global;
    let _ = Punctuated::<Expr, Token![,]>::new();

    let (impl_g, ty_g, where_c) = item.generics.split_for_impl();
    Ok(quote! {
        #item
        #injectable
        #exportable

        impl #impl_g ::corvis::Module for #name #ty_g #where_c {
            fn definition() -> ::corvis::ModuleDefinition {
                #[allow(unused_imports)]
                use ::corvis::__private::hooks::{HasConfigure as _, NoConfigure as _};
                ::corvis::ModuleDefinition::new::<Self>(#name_str)
                    .imports(::std::vec![#(#imports),*])
                    .providers(::std::vec![#(#providers),*])
                    .controllers(::std::vec![#(#controllers),*])
                    .exports(::std::vec![#(#exports),*])
                    .global(#global)
                    .module_provider(::corvis::Provider::class::<Self>())
                    .configure((&::corvis::__private::Probe::<Self>::new()).__corvis_configure())
            }
        }
    })
}

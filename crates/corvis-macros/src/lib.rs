//! Procedural macros ("decorators") for corvis.

#![forbid(unsafe_code)]

mod controller;
mod gateway;
mod handlers;
mod injectable;
mod module;
mod utils;
mod validate;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, ItemImpl, ItemStruct, parse_macro_input};

/// `@Injectable()` – registers a struct as a provider that can be injected.
///
/// Fields of type `Arc<T>` / `Option<Arc<T>>` / `ModuleRef` are resolved from the
/// container; other fields use `Default` (or `#[init(expr)]`).
///
/// * `#[inject("TOKEN")]` – inject by custom token (`@Inject('TOKEN')`).
/// * `#[init(Logger::new("CatsService"))]` – field initializer.
/// * `#[injectable(scope = "transient")]` – `Scope.TRANSIENT`.
#[proc_macro_attribute]
pub fn injectable(args: TokenStream, input: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(input as ItemStruct);
    let opts = match injectable::parse_injectable_args(args.into()) {
        Ok(o) => o,
        Err(e) => return e.to_compile_error().into(),
    };
    let imp = match injectable::injectable_impl(&mut item, &opts) {
        Ok(t) => t,
        Err(e) => return e.to_compile_error().into(),
    };
    let export = injectable::export_impl(&item, false);
    quote!(#item #imp #export).into()
}

/// `@Module({ imports, controllers, providers, exports })`.
///
/// ```ignore
/// #[module(
///     imports = [ConfigModule::for_root(Default::default()), UsersModule],
///     controllers = [CatsController],
///     providers = [CatsService, Provider::use_value("API_KEY", "secret".to_string())],
///     exports = [CatsService],
/// )]
/// pub struct CatsModule;
/// ```
#[proc_macro_attribute]
pub fn module(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    match module::expand(args.into(), item) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `@Controller('cats')`. Put class-level decorators (`#[use_guards(...)]`,
/// `#[use_interceptors(...)]`, `#[set_metadata(...)]`, ...) *below* this attribute.
#[proc_macro_attribute]
pub fn controller(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    match controller::expand_controller(args.into(), item) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Collects the route handlers of a controller impl block:
/// `#[get]`, `#[post(":id")]`, `#[put]`, `#[patch]`, `#[delete]`, `#[all]`, `#[options]`,
/// `#[head]`, `#[sse]`, plus `#[http_code]`, `#[header]`, `#[redirect]`, `#[use_guards]`,
/// `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]`, `#[version]`.
///
/// Parameters: `#[body]`, `#[body("name")]`, `#[param("id", ParseIntPipe)]`, `#[query]`,
/// `#[headers("authorization")]`, `#[ip]`, `#[req]`; undecorated parameters use `FromContext`.
#[proc_macro_attribute]
pub fn routes(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemImpl);
    match controller::expand_routes(item) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// Collects method handlers of a provider: `#[on_event("order.created")]`, `#[cron(expr)]`,
/// `#[interval(ms)]`, `#[timeout(ms)]` and `#[subscribe_message("event")]` (gateways).
#[proc_macro_attribute]
pub fn handlers(_args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemImpl);
    match handlers::expand(item) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `@WebSocketGateway()` – a gateway provider. Mark the server field with `#[websocket_server]`
/// and declare message handlers in a `#[handlers] impl` with `#[subscribe_message("event")]`.
#[proc_macro_attribute]
pub fn websocket_gateway(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    match gateway::expand(args.into(), item) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// class-validator for Rust: derives `corvis::Validate`.
#[proc_macro_derive(
    Validate,
    attributes(
        is_string,
        is_int,
        is_number,
        is_boolean,
        is_array,
        is_object,
        is_defined,
        is_optional,
        is_not_empty,
        is_empty,
        is_email,
        is_url,
        is_uuid,
        is_alpha,
        is_alphanumeric,
        is_number_string,
        is_numeric_string,
        is_date_string,
        is_iso8601,
        is_hex_color,
        is_jwt,
        is_mongo_id,
        is_ip,
        is_lowercase,
        is_uppercase,
        is_positive,
        is_negative,
        min_length,
        max_length,
        length,
        min,
        max,
        matches,
        contains,
        not_contains,
        is_in,
        is_not_in,
        equals,
        not_equals,
        array_min_size,
        array_max_size,
        array_not_empty,
        array_unique,
        is_enum,
        validate_nested,
        validate_with,
        validate_if
    )
)]
pub fn derive_validate(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match validate::expand(input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[corvis::main]` – runs the async `main` on a multi-threaded Tokio runtime.
#[proc_macro_attribute]
pub fn main(_args: TokenStream, input: TokenStream) -> TokenStream {
    let f = parse_macro_input!(input as ItemFn);
    let attrs = &f.attrs;
    let vis = &f.vis;
    let output = &f.sig.output;
    let body = &f.block;
    if f.sig.asyncness.is_none() {
        return utils::err(&f.sig, "#[corvis::main] requires an async fn").into();
    }
    quote! {
        #(#attrs)*
        #vis fn main() #output {
            ::corvis::tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("Failed building the Tokio runtime")
                .block_on(async move #body)
        }
    }
    .into()
}

/// `#[corvis::test]` – an async test running on a Tokio runtime.
#[proc_macro_attribute]
pub fn test(_args: TokenStream, input: TokenStream) -> TokenStream {
    let f = parse_macro_input!(input as ItemFn);
    let attrs = &f.attrs;
    let name = &f.sig.ident;
    let output = &f.sig.output;
    let body = &f.block;
    quote! {
        #[::core::prelude::v1::test]
        #(#attrs)*
        fn #name() #output {
            ::corvis::tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed building the Tokio runtime")
                .block_on(async move #body)
        }
    }
    .into()
}

macro_rules! misplaced {
    ($($name:ident => $msg:literal),* $(,)?) => {
        $(
            #[doc = $msg]
            #[proc_macro_attribute]
            pub fn $name(_args: TokenStream, input: TokenStream) -> TokenStream {
                let input2: proc_macro2::TokenStream = input.into();
                let msg = $msg;
                quote!(::core::compile_error!(#msg); #input2).into()
            }
        )*
    };
}

misplaced! {
    get => "#[get] must be used on a method inside a `#[routes] impl` block",
    post => "#[post] must be used on a method inside a `#[routes] impl` block",
    put => "#[put] must be used on a method inside a `#[routes] impl` block",
    patch => "#[patch] must be used on a method inside a `#[routes] impl` block",
    delete => "#[delete] must be used on a method inside a `#[routes] impl` block",
    use_guards => "#[use_guards] must be placed below #[controller] or on a method inside `#[routes]`",
    use_interceptors => "#[use_interceptors] must be placed below #[controller] or on a method inside `#[routes]`",
    use_pipes => "#[use_pipes] must be placed below #[controller] or on a method inside `#[routes]`",
    use_filters => "#[use_filters] must be placed below #[controller] or on a method inside `#[routes]`",
}

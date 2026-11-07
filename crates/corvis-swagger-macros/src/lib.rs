//! Procedural macros for `corvis-swagger`.
//!
//! * The `api_*!` function-like macros back the `#[api_*(...)]` attributes: `#[controller]`
//!   and `#[routes]` (corvis) forward every attribute whose name starts with `api_` to the
//!   macro with the same name, which evaluates to a `corvis::CustomDecorator`.
//! * `#[derive(ApiSchema)]` describes a struct like a TypeScript class with `@ApiProperty()`.

mod args;
mod derive;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use args::{Arg, Args, NamedValue};

fn krate() -> TokenStream2 {
    quote!(::corvis_swagger)
}

fn expand(input: TokenStream, f: impl FnOnce(Args) -> syn::Result<TokenStream2>) -> TokenStream {
    let parsed = match syn::parse::<Args>(input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error().into(),
    };
    match f(parsed) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `{ key: value, ... }` from the named arguments (keys converted to camelCase).
fn options_map(args: &Args) -> syn::Result<TokenStream2> {
    let k = krate();
    let mut inserts = Vec::new();
    for a in &args.items {
        match a {
            Arg::Named { key, value, .. } => {
                let v = value_tokens(value);
                inserts.push(quote!(__m.insert(::std::string::String::from(#key), #v);));
            }
            Arg::Positional(e) => {
                return Err(syn::Error::new_spanned(e, "expected `key = value` arguments"));
            }
        }
    }
    Ok(quote!({
        #[allow(unused_mut)]
        let mut __m = #k::__private::Map::new();
        #(#inserts)*
        #k::__private::Value::Object(__m)
    }))
}

fn value_tokens(v: &NamedValue) -> TokenStream2 {
    let k = krate();
    match v {
        NamedValue::Expr(e) => quote!(#k::__private::to_value(&(#e))),
        NamedValue::Type(t) => args::type_value_tokens(t),
    }
}

fn positional(args: &Args) -> Vec<&syn::Expr> {
    args.items
        .iter()
        .filter_map(|a| match a {
            Arg::Positional(e) => Some(e),
            _ => None,
        })
        .collect()
}

fn values_vec(exprs: &[&syn::Expr]) -> TokenStream2 {
    let k = krate();
    quote!(::std::vec![#(#k::__private::to_value(&(#exprs))),*])
}

fn options_macro(input: TokenStream, func: &str) -> TokenStream {
    let k = krate();
    let f = syn::Ident::new(func, proc_macro2::Span::call_site());
    expand(input, |a| {
        let m = options_map(&a)?;
        Ok(quote!(#k::decorators::#f(#m)))
    })
}

fn status_macro(input: TokenStream, status: u16) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let m = options_map(&a)?;
        Ok(quote!(#k::decorators::api_status_response(#status, #m)))
    })
}

/// `@ApiOperation({ summary, description, operationId, deprecated, tags, externalDocs })`.
#[proc_macro]
pub fn api_operation(input: TokenStream) -> TokenStream {
    options_macro(input, "api_operation")
}

/// `@ApiResponse({ status, description, type, isArray, schema, headers, links, content, example, examples })`.
#[proc_macro]
pub fn api_response(input: TokenStream) -> TokenStream {
    options_macro(input, "api_response")
}

/// `@ApiDefaultResponse(options)`.
#[proc_macro]
pub fn api_default_response(input: TokenStream) -> TokenStream {
    options_macro(input, "api_default_response")
}

/// `@ApiParam({ name, required, description, type, enum, schema, example, ... })`.
#[proc_macro]
pub fn api_param(input: TokenStream) -> TokenStream {
    options_macro(input, "api_param")
}

/// `@ApiQuery({ name, required, description, type, isArray, enum, schema, example, ... })`.
#[proc_macro]
pub fn api_query(input: TokenStream) -> TokenStream {
    options_macro(input, "api_query")
}

/// `@ApiHeader({ name, description, required, schema, example, enum, ... })`.
#[proc_macro]
pub fn api_header(input: TokenStream) -> TokenStream {
    options_macro(input, "api_header")
}

/// `@ApiBody({ type, description, required, isArray, schema, examples, ... })`.
#[proc_macro]
pub fn api_body(input: TokenStream) -> TokenStream {
    options_macro(input, "api_body")
}

/// `@ApiHeaders([...])`: `#[api_headers(json!([{ "name": "X-A" }, { "name": "X-B" }]))]`.
#[proc_macro]
pub fn api_headers(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let p = positional(&a);
        let [e] = p.as_slice() else {
            return Err(syn::Error::new(proc_macro2::Span::call_site(), "#[api_headers(expr)] expects one array expression"));
        };
        Ok(quote!(#k::decorators::api_headers(#k::__private::to_value(&(#e)))))
    })
}

/// `@ApiTags('a', 'b')`.
#[proc_macro]
pub fn api_tags(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let v = values_vec(&positional(&a));
        Ok(quote!(#k::decorators::api_tags(#v)))
    })
}

/// `@ApiProduces('application/pdf', ...)`.
#[proc_macro]
pub fn api_produces(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let v = values_vec(&positional(&a));
        Ok(quote!(#k::decorators::api_produces(#v)))
    })
}

/// `@ApiConsumes('multipart/form-data', ...)`.
#[proc_macro]
pub fn api_consumes(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let v = values_vec(&positional(&a));
        Ok(quote!(#k::decorators::api_consumes(#v)))
    })
}

/// `@ApiSecurity(name, requirements?)` or `@ApiSecurity({ name: [...] })`.
#[proc_macro]
pub fn api_security(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let p = positional(&a);
        match p.as_slice() {
            [name] => Ok(quote!(#k::decorators::api_security(#k::__private::to_value(&(#name)), #k::__private::Value::Array(::std::vec![])))),
            [name, req] => Ok(quote!(#k::decorators::api_security(#k::__private::to_value(&(#name)), #k::__private::to_value(&(#req))))),
            _ => Err(syn::Error::new(proc_macro2::Span::call_site(), "#[api_security(name, requirements?)]")),
        }
    })
}

fn optional_name_macro(input: TokenStream, func: &str) -> TokenStream {
    let k = krate();
    let f = syn::Ident::new(func, proc_macro2::Span::call_site());
    expand(input, |a| {
        let p = positional(&a);
        match p.as_slice() {
            [] => Ok(quote!(#k::decorators::#f(::std::option::Option::None))),
            [name] => Ok(quote!(#k::decorators::#f(::std::option::Option::Some(::std::string::ToString::to_string(&(#name)))))),
            _ => Err(syn::Error::new(proc_macro2::Span::call_site(), "expected an optional security name")),
        }
    })
}

/// `@ApiBearerAuth(name = 'bearer')`.
#[proc_macro]
pub fn api_bearer_auth(input: TokenStream) -> TokenStream {
    optional_name_macro(input, "api_bearer_auth")
}

/// `@ApiBasicAuth(name = 'basic')`.
#[proc_macro]
pub fn api_basic_auth(input: TokenStream) -> TokenStream {
    optional_name_macro(input, "api_basic_auth")
}

/// `@ApiCookieAuth(name = 'cookie')`.
#[proc_macro]
pub fn api_cookie_auth(input: TokenStream) -> TokenStream {
    optional_name_macro(input, "api_cookie_auth")
}

/// `@ApiOAuth2(scopes, name = 'oauth2')`.
#[proc_macro]
pub fn api_oauth2(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let p = positional(&a);
        match p.as_slice() {
            [scopes] => Ok(quote!(#k::decorators::api_oauth2(#k::__private::to_value(&(#scopes)), ::std::option::Option::None))),
            [scopes, name] => Ok(quote!(#k::decorators::api_oauth2(
                #k::__private::to_value(&(#scopes)),
                ::std::option::Option::Some(::std::string::ToString::to_string(&(#name)))
            ))),
            _ => Err(syn::Error::new(proc_macro2::Span::call_site(), "#[api_oauth2(scopes, name?)]")),
        }
    })
}

fn bool_macro(input: TokenStream, func: &str) -> TokenStream {
    let k = krate();
    let f = syn::Ident::new(func, proc_macro2::Span::call_site());
    expand(input, |a| {
        let p = positional(&a);
        match p.as_slice() {
            [] => Ok(quote!(#k::decorators::#f(true))),
            [v] => Ok(quote!(#k::decorators::#f(#v))),
            _ => Err(syn::Error::new(proc_macro2::Span::call_site(), "expected an optional boolean")),
        }
    })
}

/// `@ApiExcludeEndpoint(disable = true)`.
#[proc_macro]
pub fn api_exclude_endpoint(input: TokenStream) -> TokenStream {
    bool_macro(input, "api_exclude_endpoint")
}

/// `@ApiExcludeController(disable = true)`.
#[proc_macro]
pub fn api_exclude_controller(input: TokenStream) -> TokenStream {
    bool_macro(input, "api_exclude_controller")
}

/// `@ApiIncludeEndpoint()` (with `onlyIncludeDecoratedEndpoints`).
#[proc_macro]
pub fn api_include_endpoint(input: TokenStream) -> TokenStream {
    bool_macro(input, "api_include_endpoint")
}

/// `@ApiExtension('x-key', value)`.
#[proc_macro]
pub fn api_extension(input: TokenStream) -> TokenStream {
    let k = krate();
    expand(input, |a| {
        let p = positional(&a);
        let [key, value] = p.as_slice() else {
            return Err(syn::Error::new(proc_macro2::Span::call_site(), "#[api_extension(\"x-key\", value)]"));
        };
        Ok(quote!(#k::decorators::api_extension(::std::string::ToString::to_string(&(#key)), #k::__private::to_value(&(#value)))))
    })
}

/// `@ApiExtraModels(A, B)`.
#[proc_macro]
pub fn api_extra_models(input: TokenStream) -> TokenStream {
    let k = krate();
    let types = match syn::parse::<args::TypeList>(input) {
        Ok(t) => t.0,
        Err(e) => return e.to_compile_error().into(),
    };
    let markers: Vec<TokenStream2> = types.iter().map(args::type_marker_tokens).collect();
    quote!(#k::decorators::api_extra_models(::std::vec![#(#markers),*])).into()
}

macro_rules! status_macros {
    ($($name:ident = $code:literal),* $(,)?) => {
        $(
            #[doc = concat!("`ApiResponse({ ...options, status: ", stringify!($code), " })`.")]
            #[proc_macro]
            pub fn $name(input: TokenStream) -> TokenStream {
                status_macro(input, $code)
            }
        )*
    };
}

status_macros! {
    api_continue_response = 100,
    api_switching_protocols_response = 101,
    api_processing_response = 102,
    api_earlyhints_response = 103,
    api_ok_response = 200,
    api_created_response = 201,
    api_accepted_response = 202,
    api_non_authoritative_information_response = 203,
    api_no_content_response = 204,
    api_reset_content_response = 205,
    api_partial_content_response = 206,
    api_multi_status_response = 207,
    api_already_reported_response = 208,
    api_content_different_response = 210,
    api_ambiguous_response = 300,
    api_moved_permanently_response = 301,
    api_found_response = 302,
    api_see_other_response = 303,
    api_not_modified_response = 304,
    api_temporary_redirect_response = 307,
    api_permanent_redirect_response = 308,
    api_bad_request_response = 400,
    api_unauthorized_response = 401,
    api_payment_required_response = 402,
    api_forbidden_response = 403,
    api_not_found_response = 404,
    api_method_not_allowed_response = 405,
    api_not_acceptable_response = 406,
    api_proxy_authentication_required_response = 407,
    api_request_timeout_response = 408,
    api_conflict_response = 409,
    api_gone_response = 410,
    api_length_required_response = 411,
    api_precondition_failed_response = 412,
    api_payload_too_large_response = 413,
    api_uri_too_long_response = 414,
    api_unsupported_media_type_response = 415,
    api_requested_range_not_satisfiable_response = 416,
    api_expectation_failed_response = 417,
    api_i_am_a_teapot_response = 418,
    api_misdirected_response = 421,
    api_unprocessable_entity_response = 422,
    api_locked_response = 423,
    api_failed_dependency_response = 424,
    api_precondition_required_response = 428,
    api_too_many_requests_response = 429,
    api_unrecoverable_error_response = 456,
    api_internal_server_error_response = 500,
    api_not_implemented_response = 501,
    api_bad_gateway_response = 502,
    api_service_unavailable_response = 503,
    api_gateway_timeout_response = 504,
    api_http_version_not_supported_response = 505,
    api_insufficient_storage_response = 507,
    api_loop_detected_response = 508,
}

/// `#[derive(ApiSchema)]`: describes a struct like a TypeScript class decorated with `@ApiProperty()`.
///
/// Every named field becomes a property (like the OpenAPI compiler plugins of the Node.js ecosystem), honoring
/// `#[serde(rename)]`, `#[serde(rename_all)]` and `#[serde(skip)]`. Field attributes:
/// `#[api_property(...)]`, `#[api_property_optional(...)]`, `#[api_response_property(...)]`,
/// `#[api_hide_property]`. Container attribute: `#[api_schema(name = "...", description = "...")]`.
#[proc_macro_derive(
    ApiSchema,
    attributes(api_property, api_property_optional, api_response_property, api_hide_property, api_schema)
)]
pub fn derive_api_schema(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match derive::expand(input) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

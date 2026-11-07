//! `#[derive(Validate)]` with class-validator-like field attributes.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, Fields, Type};

use crate::utils::{attr_args, attr_name, generic_inner, last_ident, lit_str};

pub const RULES: &[&str] = &[
    "is_string", "is_int", "is_number", "is_boolean", "is_array", "is_object", "is_defined", "is_optional",
    "is_not_empty", "is_empty", "is_email", "is_url", "is_uuid", "is_alpha", "is_alphanumeric", "is_number_string",
    "is_numeric_string", "is_date_string", "is_iso8601", "is_hex_color", "is_jwt", "is_mongo_id", "is_ip",
    "is_lowercase", "is_uppercase", "is_positive", "is_negative", "min_length", "max_length", "length", "min", "max",
    "matches", "contains", "not_contains", "is_in", "is_not_in", "equals", "not_equals", "array_min_size",
    "array_max_size", "array_not_empty", "array_unique", "is_enum", "validate_nested", "validate_with", "validate_if",
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Str,
    Int,
    Float,
    Bool,
    Array,
    Map,
    Other,
}

fn kind_of(ty: &Type) -> Kind {
    match last_ident(ty).as_deref() {
        Some("String" | "str" | "char" | "Cow") => Kind::Str,
        Some("i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize") => Kind::Int,
        Some("f32" | "f64") => Kind::Float,
        Some("bool") => Kind::Bool,
        Some("Vec" | "HashSet" | "BTreeSet" | "VecDeque") => Kind::Array,
        Some("HashMap" | "BTreeMap" | "Map") => Kind::Map,
        _ => Kind::Other,
    }
}

fn rename(name: &str, rule: &str) -> String {
    let words: Vec<String> = name.split('_').filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
    let cap = |w: &str| {
        let mut c = w.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    };
    match rule {
        "camelCase" => {
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                if i == 0 { out.push_str(w) } else { out.push_str(&cap(w)) }
            }
            out
        }
        "PascalCase" => words.iter().map(|w| cap(w)).collect(),
        "kebab-case" => words.join("-"),
        "SCREAMING_SNAKE_CASE" => name.to_uppercase(),
        "SCREAMING-KEBAB-CASE" => words.join("-").to_uppercase(),
        "lowercase" => name.replace('_', "").to_lowercase(),
        "UPPERCASE" => name.replace('_', "").to_uppercase(),
        _ => name.to_string(),
    }
}

struct SerdeInfo {
    rename: Option<String>,
    skip: bool,
    default: bool,
    flatten: bool,
}

fn serde_field(attrs: &[syn::Attribute]) -> SerdeInfo {
    let mut info = SerdeInfo { rename: None, skip: false, default: false, flatten: false };
    for a in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                if let Ok(v) = meta.value() {
                    if let Ok(s) = v.parse::<syn::LitStr>() {
                        info.rename = Some(s.value());
                    }
                } else {
                    let _ = meta.parse_nested_meta(|inner| {
                        if inner.path.is_ident("deserialize") {
                            if let Ok(s) = inner.value()?.parse::<syn::LitStr>() {
                                info.rename = Some(s.value());
                            }
                        } else if let Ok(v) = inner.value() {
                            let _ = v.parse::<syn::LitStr>();
                        }
                        Ok(())
                    });
                }
            } else if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                info.skip = true;
            } else if meta.path.is_ident("default") {
                info.default = true;
                if let Ok(v) = meta.value() {
                    let _ = v.parse::<syn::LitStr>();
                }
            } else if meta.path.is_ident("flatten") {
                info.flatten = true;
            } else if let Ok(v) = meta.value() {
                let _ = v.parse::<proc_macro2::TokenStream>();
            } else {
                let _ = meta.parse_nested_meta(|m| {
                    if let Ok(v) = m.value() {
                        let _ = v.parse::<proc_macro2::TokenStream>();
                    }
                    Ok(())
                });
            }
            Ok(())
        });
    }
    info
}

fn serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    let mut out = None;
    for a in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                if let Ok(v) = meta.value() {
                    if let Ok(s) = v.parse::<syn::LitStr>() {
                        out = Some(s.value());
                    }
                } else {
                    let _ = meta.parse_nested_meta(|inner| {
                        if inner.path.is_ident("deserialize") {
                            if let Ok(s) = inner.value()?.parse::<syn::LitStr>() {
                                out = Some(s.value());
                            }
                        } else if let Ok(v) = inner.value() {
                            let _ = v.parse::<syn::LitStr>();
                        }
                        Ok(())
                    });
                }
            } else if let Ok(v) = meta.value() {
                let _ = v.parse::<proc_macro2::TokenStream>();
            } else {
                let _ = meta.parse_nested_meta(|m| {
                    if let Ok(v) = m.value() {
                        let _ = v.parse::<proc_macro2::TokenStream>();
                    }
                    Ok(())
                });
            }
            Ok(())
        });
    }
    out
}

struct RuleArgs {
    positional: Vec<Expr>,
    message: Option<Expr>,
    each: bool,
}

fn rule_args(attr: &syn::Attribute) -> syn::Result<RuleArgs> {
    let mut out = RuleArgs { positional: Vec::new(), message: None, each: false };
    for e in attr_args(attr)? {
        match e {
            Expr::Assign(a) => {
                let key = match &*a.left {
                    Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default(),
                    _ => String::new(),
                };
                match key.as_str() {
                    "message" => out.message = Some(*a.right),
                    "each" => out.each = true,
                    _ => return Err(syn::Error::new_spanned(a, "unknown option (expected `message = \"...\"` or `each`)")),
                }
            }
            Expr::Path(p) if p.path.is_ident("each") => out.each = true,
            other => out.positional.push(other),
        }
    }
    Ok(out)
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let (impl_g, ty_g, where_c) = input.generics.split_for_impl();
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(&input, "#[derive(Validate)] only supports structs with named fields"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(&input, "#[derive(Validate)] only supports structs with named fields"));
    };
    let rename_all = serde_rename_all(&input.attrs);

    let mut props = Vec::new();
    let mut blocks = Vec::new();

    for field in &fields.named {
        let serde = serde_field(&field.attrs);
        if serde.skip || serde.flatten {
            continue;
        }
        let ident = field.ident.as_ref().unwrap();
        let raw = ident.to_string();
        let raw = raw.strip_prefix("r#").unwrap_or(&raw).to_string();
        let json_name = serde.rename.clone().unwrap_or_else(|| match &rename_all {
            Some(r) => rename(&raw, r),
            None => raw.clone(),
        });
        props.push(json_name.clone());

        let is_option = generic_inner(&field.ty, "Option").is_some();
        let inner_ty = generic_inner(&field.ty, "Option").unwrap_or(&field.ty).clone();
        let kind = kind_of(&inner_ty);
        let elem_ty = generic_inner(&inner_ty, "Vec").cloned();

        let mut optional = is_option || serde.default;
        let mut checks = Vec::new();
        let mut nested: Option<Type> = None;
        let mut has_rules = false;
        let mut condition: Option<Expr> = None;

        for attr in &field.attrs {
            let rule = attr_name(attr);
            if !RULES.contains(&rule.as_str()) || attr.path().is_ident("serde") {
                continue;
            }
            let args = rule_args(attr)?;
            let custom = match &args.message {
                Some(m) => quote!(::std::option::Option::Some(#m)),
                None => quote!(::std::option::Option::None),
            };
            let p0 = args.positional.first();
            let p1 = args.positional.get(1);
            let simple = |f: &str, c: &str, m: &str| -> (TokenStream, String, TokenStream) {
                let f = syn::Ident::new(f, proc_macro2::Span::call_site());
                (quote!(__r::#f(__x)), c.to_string(), quote!(::std::format!(#m, __prop)))
            };
            let need = |e: Option<&Expr>| -> syn::Result<Expr> {
                e.cloned().ok_or_else(|| syn::Error::new_spanned(attr, format!("#[{rule}] requires an argument")))
            };
            let (check, cname, msg): (TokenStream, String, TokenStream) = match rule.as_str() {
                "is_optional" => {
                    optional = true;
                    continue;
                }
                "validate_if" => {
                    condition = Some(need(p0)?);
                    continue;
                }
                "validate_nested" => {
                    let t = elem_ty.clone().unwrap_or_else(|| inner_ty.clone());
                    nested = Some(t);
                    has_rules = true;
                    continue;
                }
                "is_string" => simple("is_string", "isString", "{} must be a string"),
                "is_int" => simple("is_int", "isInt", "{} must be an integer number"),
                "is_number" => simple("is_number", "isNumber", "{} must be a number conforming to the specified constraints"),
                "is_boolean" => simple("is_boolean", "isBoolean", "{} must be a boolean value"),
                "is_array" => simple("is_array", "isArray", "{} must be an array"),
                "is_object" => simple("is_object", "isObject", "{} must be an object"),
                "is_defined" => simple("is_defined", "isDefined", "{} should not be null or undefined"),
                "is_not_empty" => simple("is_not_empty", "isNotEmpty", "{} should not be empty"),
                "is_empty" => simple("is_empty", "isEmpty", "{} must be empty"),
                "is_email" => simple("is_email", "isEmail", "{} must be an email"),
                "is_url" => simple("is_url", "isUrl", "{} must be a URL address"),
                "is_uuid" => simple("is_uuid_value", "isUuid", "{} must be a UUID"),
                "is_alpha" => simple("is_alpha", "isAlpha", "{} must contain only letters (a-zA-Z)"),
                "is_alphanumeric" => simple("is_alphanumeric", "isAlphanumeric", "{} must contain only letters and numbers"),
                "is_number_string" | "is_numeric_string" => simple("is_number_string", "isNumberString", "{} must be a number string"),
                "is_date_string" | "is_iso8601" => simple("is_date_string", "isDateString", "{} must be a valid ISO 8601 date string"),
                "is_hex_color" => simple("is_hex_color", "isHexColor", "{} must be a hexadecimal color"),
                "is_jwt" => simple("is_jwt", "isJwt", "{} must be a jwt string"),
                "is_mongo_id" => simple("is_mongo_id", "isMongoId", "{} must be a mongodb id"),
                "is_ip" => simple("is_ip", "isIp", "{} must be an ip address"),
                "is_lowercase" => simple("is_lowercase", "isLowercase", "{} must be a lowercase string"),
                "is_uppercase" => simple("is_uppercase", "isUppercase", "{} must be uppercase"),
                "is_positive" => simple("is_positive", "isPositive", "{} must be a positive number"),
                "is_negative" => simple("is_negative", "isNegative", "{} must be a negative number"),
                "array_not_empty" => simple("array_not_empty", "arrayNotEmpty", "{} should not be empty"),
                "array_unique" => simple("array_unique", "arrayUnique", "All {}'s elements must be unique"),
                "min_length" => {
                    let n = need(p0)?;
                    (quote!(__r::min_length(__x, (#n) as usize)), "minLength".into(),
                     quote!(::std::format!("{} must be longer than or equal to {} characters", __prop, #n)))
                }
                "max_length" => {
                    let n = need(p0)?;
                    (quote!(__r::max_length(__x, (#n) as usize)), "maxLength".into(),
                     quote!(::std::format!("{} must be shorter than or equal to {} characters", __prop, #n)))
                }
                "length" => {
                    let min = need(p0)?;
                    let max = p1.cloned();
                    let max_check = match &max {
                        Some(m) => quote!(__r::max_length(__x, (#m) as usize)),
                        None => quote!(true),
                    };
                    let max_msg = match &max {
                        Some(m) => quote!(::std::format!("{} must be shorter than or equal to {} characters", __prop, #m)),
                        None => quote!(::std::string::String::new()),
                    };
                    (quote!(__r::min_length(__x, (#min) as usize) && #max_check), "isLength".into(),
                     quote!(if !__r::min_length(__val, (#min) as usize) {
                         ::std::format!("{} must be longer than or equal to {} characters", __prop, #min)
                     } else { #max_msg }))
                }
                "min" => {
                    let n = need(p0)?;
                    (quote!(__r::min(__x, (#n) as f64)), "min".into(), quote!(::std::format!("{} must not be less than {}", __prop, #n)))
                }
                "max" => {
                    let n = need(p0)?;
                    (quote!(__r::max(__x, (#n) as f64)), "max".into(), quote!(::std::format!("{} must not be greater than {}", __prop, #n)))
                }
                "matches" => {
                    let re = need(p0)?;
                    let re_s = lit_str(&re).unwrap_or_default();
                    (quote!({
                        static __RE: ::corvis::__private::LazyLock<::corvis::__private::regex::Regex> =
                            ::corvis::__private::LazyLock::new(|| ::corvis::__private::regex::Regex::new(#re).expect("invalid regex"));
                        __r::matches(__x, &__RE)
                    }), "matches".into(), quote!(::std::format!("{} must match /{}/ regular expression", __prop, #re_s)))
                }
                "contains" => {
                    let s = need(p0)?;
                    (quote!(__r::contains(__x, #s)), "contains".into(), quote!(::std::format!("{} must contain a {} string", __prop, #s)))
                }
                "not_contains" => {
                    let s = need(p0)?;
                    (quote!(__r::not_contains(__x, #s)), "notContains".into(), quote!(::std::format!("{} should not contain a {} string", __prop, #s)))
                }
                "is_in" | "is_not_in" => {
                    let vals = crate::utils::flatten_array(args.positional.clone());
                    let vals: Vec<TokenStream> = vals.iter().map(|v| quote!(::corvis::__private::serde_json::json!(#v))).collect();
                    let list = quote!(&[#(#vals),*]);
                    if rule == "is_in" {
                        (quote!(__r::is_in(__x, #list)), "isIn".into(),
                         quote!(::std::format!("{} must be one of the following values: {}", __prop, __r::join_values(#list))))
                    } else {
                        (quote!(__r::is_not_in(__x, #list)), "isNotIn".into(),
                         quote!(::std::format!("{} should not be one of the following values: {}", __prop, __r::join_values(#list))))
                    }
                }
                "equals" | "not_equals" => {
                    let v = need(p0)?;
                    let val = quote!(&::corvis::__private::serde_json::json!(#v));
                    if rule == "equals" {
                        (quote!(__r::equals(__x, #val)), "equals".into(), quote!(::std::format!("{} must be equal to {}", __prop, #v)))
                    } else {
                        (quote!(__r::not_equals(__x, #val)), "notEquals".into(), quote!(::std::format!("{} should not be equal to {}", __prop, #v)))
                    }
                }
                "array_min_size" => {
                    let n = need(p0)?;
                    (quote!(__r::array_min_size(__x, (#n) as usize)), "arrayMinSize".into(),
                     quote!(::std::format!("{} must contain at least {} elements", __prop, #n)))
                }
                "array_max_size" => {
                    let n = need(p0)?;
                    (quote!(__r::array_max_size(__x, (#n) as usize)), "arrayMaxSize".into(),
                     quote!(::std::format!("{} must contain no more than {} elements", __prop, #n)))
                }
                "is_enum" => {
                    let enum_ty: TokenStream = match p0 {
                        Some(t) => quote!(#t),
                        None => {
                            let t = if args.each { elem_ty.clone().unwrap_or_else(|| inner_ty.clone()) } else { inner_ty.clone() };
                            quote!(#t)
                        }
                    };
                    (quote!(__r::is_enum::<#enum_ty>(__x)), "isEnum".into(),
                     quote!(::std::format!("{} must be one of the following values: {}", __prop, __r::enum_variants::<#enum_ty>().join(", "))))
                }
                "validate_with" => {
                    let f = need(p0)?;
                    (quote!((#f)(__x)), "customValidation".into(), quote!(::std::format!("{} is invalid", __prop)))
                }
                _ => continue,
            };
            has_rules = true;
            let (check, msg) = if args.each {
                (quote!(__r::each(__val, |__x| #check)), quote!(::std::format!("each value in {}", #msg)))
            } else {
                (quote!({ let __x = __val; #check }), msg)
            };
            checks.push(quote! {
                if !(__opts.stop_at_first_error && !__c.is_empty()) && !(#check) {
                    __c.push((#cname.to_string(), __r::message(#custom, #msg, __prop, __val)));
                }
            });
        }

        // Implicit type checks derived from the Rust type when the field has no rules.
        if !has_rules {
            let implicit = match kind {
                Kind::Str => Some(("is_string", "isString", "{} must be a string")),
                Kind::Int => Some(("is_int", "isInt", "{} must be an integer number")),
                Kind::Float => Some(("is_number", "isNumber", "{} must be a number conforming to the specified constraints")),
                Kind::Bool => Some(("is_boolean", "isBoolean", "{} must be a boolean value")),
                Kind::Array => Some(("is_array", "isArray", "{} must be an array")),
                Kind::Map => Some(("is_object", "isObject", "{} must be an object")),
                Kind::Other => {
                    if last_ident(&inner_ty).as_deref() == Some("Value") {
                        None
                    } else {
                        Some(("is_defined", "isDefined", "{} should not be null or undefined"))
                    }
                }
            };
            if let Some((f, c, m)) = implicit {
                let f = syn::Ident::new(f, proc_macro2::Span::call_site());
                checks.push(quote! {
                    if !__r::#f(__val) {
                        __c.push((#c.to_string(), ::std::format!(#m, __prop)));
                    }
                });
            }
        }

        let prim = match kind {
            Kind::Int => Some(quote!(__r::Primitive::Int)),
            Kind::Float => Some(quote!(__r::Primitive::Float)),
            Kind::Bool => Some(quote!(__r::Primitive::Bool)),
            _ => None,
        };
        let coerce = prim.map(|p| quote! {
            if __opts.coerce_primitives { __r::coerce(__obj, __prop, #p); }
        });
        let nested_block = nested.map(|t| quote! {
            __children = __r::nested::<#t>(__obj.get_mut(__prop), __prop, __opts, &mut __c);
        });
        let cond = condition.map(|c| quote!(&& (#c)(&*__obj))).unwrap_or_default();

        blocks.push(quote! {
            {
                let __prop: &str = #json_name;
                #coerce
                let __missing = __r::is_missing(__obj.get(__prop));
                let __skip = (#optional && __missing) || (__opts.skip_missing_properties && __missing);
                if !__skip #cond {
                    let mut __c: ::std::vec::Vec<(::std::string::String, ::std::string::String)> = ::std::vec::Vec::new();
                    #[allow(unused_mut)]
                    let mut __children: ::std::vec::Vec<::corvis::ValidationError> = ::std::vec::Vec::new();
                    {
                        let __val: ::std::option::Option<&::corvis::__private::serde_json::Value> = __obj.get(__prop);
                        #(#checks)*
                    }
                    #nested_block
                    if !__c.is_empty() || !__children.is_empty() {
                        let mut __e = ::corvis::ValidationError::new(__prop);
                        __e.value = __obj.get(__prop).cloned();
                        __e.constraints = __c;
                        __e.children = __children;
                        __errors.push(__e);
                    }
                }
            }
        });
    }

    Ok(quote! {
        impl #impl_g ::corvis::Validate for #name #ty_g #where_c {
            fn validate_value(
                __value: &mut ::corvis::__private::serde_json::Value,
                __opts: &::corvis::ValidatorOptions,
            ) -> ::std::vec::Vec<::corvis::ValidationError> {
                #[allow(unused_imports)]
                use ::corvis::validation::rules as __r;
                let mut __errors: ::std::vec::Vec<::corvis::ValidationError> = ::std::vec::Vec::new();
                if __value.is_null() {
                    *__value = ::corvis::__private::serde_json::Value::Object(::std::default::Default::default());
                }
                let ::std::option::Option::Some(__obj) = __value.as_object_mut() else {
                    let mut __e = ::corvis::ValidationError::new("");
                    __e.constraints.push(("unknownValue".to_string(), "an unknown value was passed to the validate function".to_string()));
                    return ::std::vec![__e];
                };
                if __opts.whitelist {
                    __r::whitelist(__obj, <Self as ::corvis::Validate>::properties(), __opts.forbid_non_whitelisted, &mut __errors);
                }
                #(#blocks)*
                __errors
            }

            fn properties() -> &'static [&'static str] {
                &[#(#props),*]
            }
        }
    })
}

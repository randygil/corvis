//! `z.locales`: error messages in other languages.
//!
//! ```ignore
//! z::configure(|c| c.locale_error = Some(z::locales::es()));   // z.config(z.locales.es())
//! ```
//!
//! `en` is the default. Other languages can be supplied the same way with an
//! [`ErrorMap`] (`z::error_fn(|iss| ...)`).

use std::sync::Arc;

use serde_json::Value;

use super::ZodIssueCode;
use super::core::{ErrorMap, IssueInfo, en_message, join_values, stringify_primitive_json};
use super::js;

/// `z.locales.en()` (the default messages).
pub fn en() -> ErrorMap {
    Arc::new(|iss: &IssueInfo| Some(en_message(iss)))
}

/// `z.locales.es()`: Spanish messages of `zod/v4/locales/es.js`.
pub fn es() -> ErrorMap {
    Arc::new(|iss: &IssueInfo| Some(es_message(iss)))
}

/// `${x}` of a value inside a template string.
fn tpl(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => js::to_js_string(v),
    }
}

fn es_type(t: &str) -> Option<&'static str> {
    Some(match t {
        "nan" => "NaN",
        "string" => "texto",
        "number" => "número",
        "boolean" => "booleano",
        "array" => "arreglo",
        "object" => "objeto",
        "set" => "conjunto",
        "file" => "archivo",
        "date" => "fecha",
        "bigint" => "número grande",
        "symbol" => "símbolo",
        "undefined" => "indefinido",
        "null" => "nulo",
        "function" => "función",
        "map" => "mapa",
        "record" => "registro",
        "tuple" => "tupla",
        "enum" => "enumeración",
        "union" => "unión",
        "literal" => "literal",
        "promise" => "promesa",
        "void" => "vacío",
        "never" => "nunca",
        "unknown" => "desconocido",
        "any" => "cualquiera",
        _ => return None,
    })
}

fn es_format(f: &str) -> Option<&'static str> {
    Some(match f {
        "regex" => "entrada",
        "email" => "dirección de correo electrónico",
        "url" => "URL",
        "emoji" => "emoji",
        "uuid" => "UUID",
        "uuidv4" => "UUIDv4",
        "uuidv6" => "UUIDv6",
        "nanoid" => "nanoid",
        "guid" => "GUID",
        "cuid" => "cuid",
        "cuid2" => "cuid2",
        "ulid" => "ULID",
        "xid" => "XID",
        "ksuid" => "KSUID",
        "datetime" => "fecha y hora ISO",
        "date" => "fecha ISO",
        "time" => "hora ISO",
        "duration" => "duración ISO",
        "ipv4" => "dirección IPv4",
        "ipv6" => "dirección IPv6",
        "cidrv4" => "rango IPv4",
        "cidrv6" => "rango IPv6",
        "base64" => "cadena codificada en base64",
        "base64url" => "URL codificada en base64",
        "json_string" => "cadena JSON",
        "e164" => "número E.164",
        "jwt" => "JWT",
        "template_literal" => "entrada",
        _ => return None,
    })
}

/// `TypeDictionary[x] ?? x` (`None` when `x` is undefined).
fn es_named(v: Option<&Value>) -> Option<String> {
    v.map(|v| match v.as_str().and_then(es_type) {
        Some(t) => t.to_string(),
        None => tpl(Some(v)),
    })
}

/// Messages of `zod/v4/locales/es.js`.
pub fn es_message(iss: &IssueInfo) -> String {
    let f = |k: &str| iss.fields.get(k);
    match iss.code {
        ZodIssueCode::InvalidType => {
            let expected_raw = tpl(f("expected"));
            let received_type = iss.input_val.parsed_type();
            let received = es_type(received_type).unwrap_or(received_type);
            if expected_raw.starts_with(|c: char| c.is_ascii_uppercase()) {
                return format!("Entrada inválida: se esperaba instanceof {expected_raw}, recibido {received}");
            }
            let expected = es_named(f("expected")).unwrap_or(expected_raw);
            format!("Entrada inválida: se esperaba {expected}, recibido {received}")
        }
        ZodIssueCode::InvalidValue => {
            let values = f("values").and_then(Value::as_array).cloned().unwrap_or_default();
            if values.len() == 1 {
                format!("Entrada inválida: se esperaba {}", stringify_primitive_json(&values[0]))
            } else {
                format!("Opción inválida: se esperaba una de {}", join_values(&values, "|"))
            }
        }
        ZodIssueCode::TooBig | ZodIssueCode::TooSmall => {
            let big = iss.code == ZodIssueCode::TooBig;
            let inclusive = f("inclusive").is_some_and(|v| v.as_bool() == Some(true));
            let adj = match (big, inclusive) {
                (true, true) => "<=",
                (true, false) => "<",
                (false, true) => ">=",
                (false, false) => ">",
            };
            let unit = match f("origin").and_then(Value::as_str) {
                Some("string") => Some("caracteres"),
                Some("file") => Some("bytes"),
                Some("array" | "set") => Some("elementos"),
                _ => None,
            };
            let origin = es_named(f("origin"));
            let bound = tpl(f(if big { "maximum" } else { "minimum" }));
            if big {
                let origin = origin.unwrap_or_else(|| "valor".into());
                match unit {
                    Some(u) => format!("Demasiado grande: se esperaba que {origin} tuviera {adj}{bound} {u}"),
                    None => format!("Demasiado grande: se esperaba que {origin} fuera {adj}{bound}"),
                }
            } else {
                let origin = origin.unwrap_or_else(|| "undefined".into());
                match unit {
                    Some(u) => format!("Demasiado pequeño: se esperaba que {origin} tuviera {adj}{bound} {u}"),
                    None => format!("Demasiado pequeño: se esperaba que {origin} fuera {adj}{bound}"),
                }
            }
        }
        ZodIssueCode::InvalidFormat => match f("format").and_then(Value::as_str).unwrap_or("") {
            "starts_with" => format!("Cadena inválida: debe comenzar con \"{}\"", tpl(f("prefix"))),
            "ends_with" => format!("Cadena inválida: debe terminar en \"{}\"", tpl(f("suffix"))),
            "includes" => format!("Cadena inválida: debe incluir \"{}\"", tpl(f("includes"))),
            "regex" => format!("Cadena inválida: debe coincidir con el patrón {}", tpl(f("pattern"))),
            other => format!("Inválido {}", es_format(other).map(str::to_string).unwrap_or_else(|| tpl(f("format")))),
        },
        ZodIssueCode::NotMultipleOf => format!("Número inválido: debe ser múltiplo de {}", tpl(f("divisor"))),
        ZodIssueCode::UnrecognizedKeys => {
            let keys = f("keys").and_then(Value::as_array).cloned().unwrap_or_default();
            let s = if keys.len() > 1 { "s" } else { "" };
            format!("Llave{s} desconocida{s}: {}", join_values(&keys, ", "))
        }
        ZodIssueCode::InvalidKey => format!("Llave inválida en {}", es_named(f("origin")).unwrap_or_else(|| "undefined".into())),
        ZodIssueCode::InvalidUnion => "Entrada inválida".into(),
        ZodIssueCode::InvalidElement => format!("Valor inválido en {}", es_named(f("origin")).unwrap_or_else(|| "undefined".into())),
        _ => "Entrada inválida".into(),
    }
}

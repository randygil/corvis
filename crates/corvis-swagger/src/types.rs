//! Types used in decorators (`type = UserDto`) and handler signatures.
//!
//! A type is written into decorator metadata as a JSON marker: `{"$ctor": "String"}` for the
//! JavaScript constructors the TypeScript decorators accept (`String`, `Number`, `Boolean`, `Object`,
//! `Array`, `Date`, `BigInt`) and `{"$model": "<key>"}` for schema types (an
//! [`ApiSchema`](corvis::ApiSchema) struct or a zod DTO). Model markers are resolved through a
//! process-wide registry that is filled when the marker is created.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use corvis::openapi::ApiSchemaRef;
use corvis::z::Schema;
use serde_json::{Map, Value};

/// `io` of a zod DTO (`createZodDto(schema)` is `input`, `Dto.Output` is `output`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZodIo {
    Input,
    Output,
}

/// How a schema type is described.
#[derive(Clone, Copy, Debug)]
pub enum ModelKind {
    /// `#[derive(ApiSchema)]` / manual [`ApiSchema`](corvis::ApiSchema).
    ApiSchema(ApiSchemaRef),
    /// A zod DTO (`zod_dto!`).
    Zod { schema: fn() -> &'static Schema, io: ZodIo },
    /// A type without OpenAPI metadata (a class without `@ApiProperty()`): an empty object.
    Opaque,
}

/// A registered schema type.
#[derive(Clone, Debug)]
pub struct ModelEntry {
    pub key: String,
    /// Class name (`components.schemas` key before `@ApiSchema`/zod `id` renames).
    pub name: String,
    pub kind: ModelKind,
}

static REGISTRY: LazyLock<RwLock<HashMap<String, ModelEntry>>> = LazyLock::new(Default::default);

/// Registers a model and returns its marker.
pub fn register_model(entry: ModelEntry) -> Value {
    let key = entry.key.clone();
    if let Ok(mut r) = REGISTRY.write() {
        r.entry(key.clone()).or_insert(entry);
    }
    model_marker(&key)
}

pub fn lookup_model(key: &str) -> Option<ModelEntry> {
    REGISTRY.read().ok().and_then(|r| r.get(key).cloned())
}

pub fn model_marker(key: &str) -> Value {
    let mut m = Map::new();
    m.insert("$model".into(), Value::String(key.to_string()));
    Value::Object(m)
}

pub fn ctor_marker(name: &str) -> Value {
    let mut m = Map::new();
    m.insert("$ctor".into(), Value::String(name.to_string()));
    Value::Object(m)
}

/// `{"$ctor": "String"}` -> `Some("String")`.
pub fn as_ctor(v: Option<&Value>) -> Option<&str> {
    let o = v?.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.get("$ctor")?.as_str()
}

/// `{"$model": key}` -> `Some(key)`.
pub fn as_model(v: Option<&Value>) -> Option<&str> {
    let o = v?.as_object()?;
    if o.len() != 1 {
        return None;
    }
    o.get("$model")?.as_str()
}

/// A model registered for an [`ApiSchema`](corvis::ApiSchema) handle.
pub fn api_schema_marker(r: ApiSchemaRef) -> Value {
    let def_name = corvis::openapi::short_type_name((r.type_name)());
    register_model(ModelEntry { key: (r.type_name)().to_string(), name: def_name, kind: ModelKind::ApiSchema(r) })
}

/// A model for a type without OpenAPI metadata (`{ type: "object", properties: {} }`).
pub fn opaque_marker(type_name: &str) -> Value {
    let name = corvis::openapi::short_type_name(type_name);
    register_model(ModelEntry { key: type_name.to_string(), name, kind: ModelKind::Opaque })
}

/// A model registered for a zod schema.
pub fn zod_marker(key: &str, name: &str, schema: fn() -> &'static Schema, io: ZodIo) -> Value {
    register_model(ModelEntry { key: key.to_string(), name: name.to_string(), kind: ModelKind::Zod { schema, io } })
}

/// Types with a fixed meaning in decorators: the JavaScript constructors (`String`,
/// `Number`, ...) or a custom marker. Implement it for your own scalar types:
///
/// ```ignore
/// impl SwaggerType for Decimal {
///     fn swagger_type() -> serde_json::Value { corvis_swagger::ctor_marker("String") }
/// }
/// ```
pub trait SwaggerType {
    fn swagger_type() -> Value;
}

macro_rules! ctor_types {
    ($ctor:literal: $($t:ty),*) => {
        $(impl SwaggerType for $t { fn swagger_type() -> Value { ctor_marker($ctor) } })*
    };
}

ctor_types!("String": String, str, &str, char, std::borrow::Cow<'_, str>);
ctor_types!("Number": i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);
ctor_types!("Boolean": bool);
ctor_types!("Object": Value, Map<String, Value>);

impl<K, V> SwaggerType for HashMap<K, V> {
    fn swagger_type() -> Value {
        ctor_marker("Object")
    }
}
impl<K, V> SwaggerType for std::collections::BTreeMap<K, V> {
    fn swagger_type() -> Value {
        ctor_marker("Object")
    }
}
impl<T> SwaggerType for Vec<T> {
    fn swagger_type() -> Value {
        ctor_marker("Array")
    }
}
impl SwaggerType for std::time::SystemTime {
    fn swagger_type() -> Value {
        ctor_marker("Date")
    }
}

/// `Dto.Output`: the output schema of a zod DTO, named `<Dto>_Output`.
pub struct Output<T>(std::marker::PhantomData<T>);

impl<T: corvis::z::ZodDto> SwaggerType for Output<T> {
    fn swagger_type() -> Value {
        let name = format!("{}_Output", corvis::openapi::short_type_name(std::any::type_name::<T>()));
        zod_marker(std::any::type_name::<Self>(), &name, T::schema, ZodIo::Output)
    }
}

impl<T: SwaggerType> SwaggerType for Option<T> {
    fn swagger_type() -> Value {
        T::swagger_type()
    }
}

/// Autoref-specialization probes used by the macros:
/// [`ApiSchema`](corvis::ApiSchema) > zod DTO > [`SwaggerType`].
pub mod probe {
    use super::*;

    pub struct Probe<T: ?Sized>(std::marker::PhantomData<fn() -> *const T>);

    impl<T: ?Sized> Probe<T> {
        pub const fn new() -> Self {
            Probe(std::marker::PhantomData)
        }
    }

    pub trait ModelProbe {
        fn __swagger_type_marker(&self) -> Value;
    }
    impl<T: corvis::ApiSchema + ?Sized> ModelProbe for &&Probe<T> {
        fn __swagger_type_marker(&self) -> Value {
            api_schema_marker(ApiSchemaRef::of::<T>())
        }
    }

    pub trait ZodProbe {
        fn __swagger_type_marker(&self) -> Value;
    }
    impl<T: corvis::z::ZodDto> ZodProbe for &Probe<T> {
        fn __swagger_type_marker(&self) -> Value {
            let full = std::any::type_name::<T>();
            zod_marker(full, &corvis::openapi::short_type_name(full), T::schema, ZodIo::Input)
        }
    }

    pub trait TypeProbe {
        fn __swagger_type_marker(&self) -> Value;
    }
    impl<T: SwaggerType + ?Sized> TypeProbe for Probe<T> {
        fn __swagger_type_marker(&self) -> Value {
            T::swagger_type()
        }
    }
}

/// Classifies a handler parameter type from its declared name (`Metatype.name`), like the
/// TypeScript `design:paramtypes` reflected by the framework: `String`, `Number`, `Boolean`, `Array`,
/// `Date` or `Object` (`None` for unknown types).
pub fn ctor_from_type_name(name: &str) -> Option<&'static str> {
    let mut n = name.trim();
    loop {
        let base = n.split('<').next().unwrap_or(n);
        let last = base.rsplit("::").next().unwrap_or(base);
        if matches!(last, "Option" | "Box" | "Arc" | "Rc") && n.ends_with('>') {
            let start = n.find('<').map(|i| i + 1).unwrap_or(0);
            n = &n[start..n.len() - 1];
            continue;
        }
        break;
    }
    let n = n.trim_start_matches('&').trim_start_matches("'static").trim();
    if n.starts_with('[') {
        return Some("Array");
    }
    let base = n.split('<').next().unwrap_or(n);
    let last = base.rsplit("::").next().unwrap_or(base);
    Some(match last {
        "String" | "str" | "char" | "Cow" | "Uuid" => "String",
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "f32" | "f64" => "Number",
        "NonZeroU8" | "NonZeroU16" | "NonZeroU32" | "NonZeroU64" | "NonZeroUsize" | "NonZeroI32" | "NonZeroI64" => "Number",
        "bool" => "Boolean",
        "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => "Array",
        "DateTime" | "NaiveDateTime" | "NaiveDate" | "SystemTime" | "OffsetDateTime" => "Date",
        "Value" | "Map" | "HashMap" | "BTreeMap" | "IndexMap" => "Object",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_param_types() {
        assert_eq!(ctor_from_type_name("String"), Some("String"));
        assert_eq!(ctor_from_type_name("Option<String>"), Some("String"));
        assert_eq!(ctor_from_type_name("std::string::String"), Some("String"));
        assert_eq!(ctor_from_type_name("u32"), Some("Number"));
        assert_eq!(ctor_from_type_name("Option<i64>"), Some("Number"));
        assert_eq!(ctor_from_type_name("bool"), Some("Boolean"));
        assert_eq!(ctor_from_type_name("Vec<String>"), Some("Array"));
        assert_eq!(ctor_from_type_name("serde_json::Value"), Some("Object"));
        assert_eq!(ctor_from_type_name("LoginDto"), None);
    }

    #[test]
    fn markers() {
        assert_eq!(as_ctor(Some(&ctor_marker("String"))), Some("String"));
        assert_eq!(as_model(Some(&model_marker("k"))), Some("k"));
        assert_eq!(as_ctor(Some(&model_marker("k"))), None);
    }
}

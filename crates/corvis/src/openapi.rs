//! Type descriptions for OpenAPI generators (used by `corvis-swagger`).
//!
//! [`ApiSchema`] is the equivalent of a TypeScript class whose properties carry `@ApiProperty()`
//! decorators: it describes the component name and the metadata of every property. The
//! `#[routes]` macro records it for `#[body]` / `#[query]` / `#[param]` / `#[headers]`
//! parameters (see [`ParamDef::api_schema`](crate::ParamDef)), so a document generator can
//! inspect handler signatures without knowing the concrete types.
//!
//! You normally do not implement it by hand: `corvis-swagger` provides
//! `#[derive(ApiSchema)]` with `#[api_property(...)]` attributes.

use serde_json::{Map, Value};

/// A type that can be described as an OpenAPI schema (a class with `@ApiProperty()` metadata).
pub trait ApiSchema {
    /// Description of the type: name, class-level metadata and property metadata.
    fn api_schema_def() -> ApiSchemaDef;
}

/// What an OpenAPI document generator reads from a decorated class.
#[derive(Debug, Clone, Default)]
pub struct ApiSchemaDef {
    /// Component name (`#/components/schemas/<name>`): the class name, or `@ApiSchema({ name })`.
    pub name: String,
    /// Other `@ApiSchema({...})` options (`description`, ...), merged into the component.
    pub schema_properties: Map<String, Value>,
    /// `@ApiExtension('x-...', value)` on the class.
    pub extensions: Map<String, Value>,
    /// `(property name, @ApiProperty metadata)` in declaration order. A `"type"` that refers to
    /// another schema is written as `{"$model": "<rust type name>"}` and resolved with `refs`;
    /// constructor types (`String`, `Number`, `Boolean`, `Date`, `Object`, `Array`, `BigInt`)
    /// as `{"$ctor": "String"}`.
    pub properties: Vec<(String, Value)>,
    /// Types referenced from `properties` (and `@ApiExtraModels`).
    pub refs: Vec<ApiSchemaRef>,
    /// `@ApiExtraModels(...)` on the class.
    pub extra_models: Vec<ApiSchemaRef>,
}

/// Type-erased handle to an [`ApiSchema`] implementation.
#[derive(Clone, Copy)]
pub struct ApiSchemaRef {
    /// `std::any::type_name` of the type (unique key).
    pub type_name: fn() -> &'static str,
    /// [`ApiSchema::api_schema_def`].
    pub def: fn() -> ApiSchemaDef,
}

impl ApiSchemaRef {
    pub fn of<T: ApiSchema + ?Sized>() -> Self {
        ApiSchemaRef { type_name: std::any::type_name::<T>, def: T::api_schema_def }
    }

    /// The marker used in property metadata to reference this type: `{"$model": "<type name>"}`.
    pub fn marker(&self) -> Value {
        let mut m = Map::new();
        m.insert("$model".into(), Value::String((self.type_name)().to_string()));
        Value::Object(m)
    }
}

impl std::fmt::Debug for ApiSchemaRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ApiSchemaRef({})", (self.type_name)())
    }
}

/// Short name of a type: `my_app::dto::LoginDto<T>` -> `LoginDto`.
pub fn short_type_name(full: &str) -> String {
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Cat;
    impl ApiSchema for Cat {
        fn api_schema_def() -> ApiSchemaDef {
            ApiSchemaDef { name: "Cat".into(), ..Default::default() }
        }
    }

    #[test]
    fn schema_ref() {
        let r = ApiSchemaRef::of::<Cat>();
        assert!((r.type_name)().ends_with("Cat"));
        assert_eq!((r.def)().name, "Cat");
        assert_eq!(r.marker()["$model"], Value::String((r.type_name)().to_string()));
        assert_eq!(short_type_name("a::b::LoginDto<x::Y>"), "LoginDto");
        assert_eq!(short_type_name("LoginDto"), "LoginDto");
    }
}

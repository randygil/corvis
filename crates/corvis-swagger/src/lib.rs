//! # corvis-swagger
//!
//! OpenAPI for [Corvis](corvis): the document generator (`DocumentBuilder`,
//! `SwaggerModule::create_document` / `setup`, the `@Api*`-style decorators as `#[api_*(...)]`
//! attributes and `#[derive(ApiSchema)]` for model properties), the zod DTO integration
//! (zod DTOs in the document and [`cleanup_open_api_doc`]) and the Scalar API reference page
//! ([`scalar`]). Documents follow the same rules as the generators of the Node.js ecosystem,
//! key order included.
//!
//! ```ignore
//! use corvis_swagger::{DocumentBuilder, SwaggerModule, cleanup_open_api_doc};
//!
//! let config = DocumentBuilder::new().set_title("Cats").set_version("1.0").add_bearer_auth().build();
//! let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
//! SwaggerModule::setup("docs", &mut app, document);
//! ```

pub mod decorators;
pub mod document;
pub mod explorer;
pub mod js;
pub mod metadata;
pub mod scalar;
pub mod schema_factory;
pub mod swagger_module;
pub mod swagger_ui;
pub mod types;
pub mod yaml;
pub mod zod;

pub use document::{DocumentBuilder, OpenApiDocument, TagOptions};
pub use explorer::{OperationIdFactory, SwaggerDocumentOptions};
pub use scalar::ApiReferenceOptions;
pub use swagger_module::{SwaggerCustomOptions, SwaggerModule};
pub use types::{SwaggerType, ZodIo, ctor_marker};
pub use zod::{cleanup_open_api_doc, cleanup_open_api_doc_with};

pub use corvis_swagger_macros::*;

/// Support code for the macros. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use corvis::openapi::short_type_name;
    pub use serde_json::{Map, Value};

    pub use crate::schema_factory::property_metadata;

    /// `serde_json::to_value` that never fails (`null` on error).
    pub fn to_value<T: serde::Serialize + ?Sized>(v: &T) -> Value {
        serde_json::to_value(v).unwrap_or(Value::Null)
    }

    /// Autoref specialization used by `type = T` arguments and `#[derive(ApiSchema)]` fields:
    /// an [`ApiSchema`](corvis::openapi::ApiSchema) type, then a zod DTO, then a
    /// [`SwaggerType`](crate::SwaggerType), then any other type (an empty object schema).
    pub mod probe {
        use std::marker::PhantomData;

        use serde_json::Value;

        use crate::types::{self, SwaggerType, ZodIo};

        pub struct Probe<T: ?Sized>(PhantomData<fn() -> Box<T>>);

        impl<T: ?Sized> Clone for Probe<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T: ?Sized> Copy for Probe<T> {}

        impl<T: ?Sized> Default for Probe<T> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<T: ?Sized> Probe<T> {
            pub const fn new() -> Self {
                Probe(PhantomData)
            }

            /// Fallback: a type without OpenAPI metadata.
            pub fn __swagger_type_marker(self) -> Value {
                types::opaque_marker(std::any::type_name::<T>())
            }
        }

        pub trait ModelProbe {
            fn __swagger_type_marker(&self) -> Value;
        }
        impl<T: corvis::openapi::ApiSchema + ?Sized> ModelProbe for &&Probe<T> {
            fn __swagger_type_marker(&self) -> Value {
                types::api_schema_marker(corvis::openapi::ApiSchemaRef::of::<T>())
            }
        }

        pub trait ZodProbe {
            fn __swagger_type_marker(&self) -> Value;
        }
        impl<T: corvis::z::ZodDto + ?Sized> ZodProbe for &Probe<T> {
            fn __swagger_type_marker(&self) -> Value {
                let full = std::any::type_name::<T>();
                types::zod_marker(&format!("zod:{full}"), &corvis::openapi::short_type_name(full), T::schema, ZodIo::Input)
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
}

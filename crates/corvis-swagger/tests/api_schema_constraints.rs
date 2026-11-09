//! `#[derive(ApiSchema)]` turns the `#[derive(Validate)]` constraints of a field into OpenAPI keywords, like the
//! OpenAPI compiler plugin of the original framework does for class-validator decorators. The fixture
//! `tests/fixtures/class_validator_shim` records the schemas that plugin produces for equivalent classes.

use corvis::Validate;
use corvis::prelude::*;
use corvis_swagger::*;
use serde::Deserialize;

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct LengthRangeDto {
    #[is_string]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    pub age: u32,
    #[min_length(2)]
    pub short: String,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct LengthDto {
    #[length(2)]
    pub only_min: String,
    #[length(2, 20)]
    pub both: String,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormatDto {
    #[is_email]
    pub email: String,
    #[is_url]
    pub site: String,
    #[is_uuid]
    pub id: String,
    #[is_hex_color]
    pub color: String,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatternDto {
    #[matches("^[a-z]+$")]
    pub slug: String,
    #[contains("x")]
    pub has_x: String,
    #[contains("a.b")]
    pub has_dot: String,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignDto {
    #[is_positive]
    pub count: i64,
    #[is_negative]
    pub debt: i64,
    #[is_positive]
    pub ratio: f64,
    #[is_negative]
    pub drop: f64,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllowedDto {
    #[is_in("persian", "siamese")]
    pub breed: String,
    #[is_in(1, 2, 3)]
    pub level: i64,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnmappedDto {
    #[is_int]
    #[is_not_empty]
    pub n: i64,
    #[is_date_string]
    pub when: String,
    #[is_not_empty]
    pub text: String,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArraysDto {
    #[max_length(10, each)]
    #[array_max_size(5)]
    pub tags: Vec<String>,
    #[array_min_size(1)]
    #[array_unique]
    pub names: Vec<String>,
    #[array_not_empty]
    pub items: Vec<String>,
    #[is_in(["a", "b"], each)]
    pub letters: Vec<String>,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrecedenceDto {
    #[min(5)]
    #[is_positive]
    pub min_then_positive: i64,
    #[contains("x")]
    #[matches("^[a-z]+$")]
    pub contains_then_matches: String,
    #[min_length(1)]
    #[length(3, 9)]
    pub min_length_then_length: String,
    #[max(10)]
    #[max(20)]
    pub repeated: i64,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptionalDto {
    #[is_optional]
    #[max_length(5)]
    pub nick: Option<String>,
    #[is_optional]
    #[min(1)]
    pub level: Option<i64>,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplicitDto {
    #[max(40)]
    #[api_property(maximum = 30)]
    pub capped: i64,
    #[max_length(10)]
    #[api_property(description = "Label")]
    pub label: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
}

#[derive(Deserialize, Validate, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnumDto {
    #[is_enum]
    pub role: Role,
}

#[controller("x")]
pub struct XController;

#[routes]
impl XController {
    #[post("length-range")]
    fn a(&self, #[body] _d: LengthRangeDto) {}
    #[post("length")]
    fn b(&self, #[body] _d: LengthDto) {}
    #[post("format")]
    fn c(&self, #[body] _d: FormatDto) {}
    #[post("pattern")]
    fn d(&self, #[body] _d: PatternDto) {}
    #[post("sign")]
    fn e(&self, #[body] _d: SignDto) {}
    #[post("allowed")]
    fn f(&self, #[body] _d: AllowedDto) {}
    #[post("unmapped")]
    fn g(&self, #[body] _d: UnmappedDto) {}
    #[post("arrays")]
    fn h(&self, #[body] _d: ArraysDto) {}
    #[post("precedence")]
    fn i(&self, #[body] _d: PrecedenceDto) {}
    #[post("optional")]
    fn j(&self, #[body] _d: OptionalDto) {}
    #[post("explicit")]
    fn k(&self, #[body] _d: ExplicitDto) {}
    #[post("enum")]
    fn l(&self, #[body] _d: EnumDto) {}
}

#[module(controllers = [XController])]
pub struct AppModule;

async fn schemas() -> Value {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    let config = DocumentBuilder::new().set_title("Constraints").set_version("1.0").build();
    let doc = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
    doc.as_value()["components"]["schemas"].clone()
}

#[corvis::test]
async fn length_and_range() {
    let s = schemas().await;
    let p = &s["LengthRangeDto"]["properties"];
    assert_eq!(p["name"], json!({ "type": "string", "maxLength": 50 }));
    assert_eq!(p["age"], json!({ "type": "number", "minimum": 0, "maximum": 40 }));
    assert_eq!(p["short"], json!({ "type": "string", "minLength": 2 }));
}

#[corvis::test]
async fn email_format() {
    let s = schemas().await;
    assert_eq!(s["FormatDto"]["properties"]["email"], json!({ "type": "string", "format": "email" }));
}

#[corvis::test]
async fn allowed_values() {
    let s = schemas().await;
    assert_eq!(s["AllowedDto"]["properties"]["breed"], json!({ "type": "string", "enum": ["persian", "siamese"] }));
}

#[corvis::test]
async fn rules_without_a_mapping_add_nothing() {
    let s = schemas().await;
    let p = &s["UnmappedDto"]["properties"];
    assert_eq!(p["n"], json!({ "type": "number" }));
    assert_eq!(p["when"], json!({ "type": "string" }));
    assert_eq!(p["text"], json!({ "type": "string" }));
}

#[corvis::test]
async fn constraints_on_array_elements() {
    let s = schemas().await;
    assert_eq!(
        s["ArraysDto"]["properties"]["tags"],
        json!({ "type": "array", "maxItems": 5, "items": { "type": "string", "maxLength": 10 } })
    );
    assert_eq!(s["ArraysDto"]["properties"]["letters"], json!({ "type": "array", "items": { "type": "string", "enum": ["a", "b"] } }));
}

#[corvis::test]
async fn precedence_pairs() {
    let s = schemas().await;
    let p = &s["PrecedenceDto"]["properties"];
    assert_eq!(p["minThenPositive"]["minimum"], 1);
    assert_eq!(p["containsThenMatches"]["pattern"], "^[a-z]+$");
    assert_eq!(p["minLengthThenLength"], json!({ "type": "string", "minLength": 3, "maxLength": 9 }));
    assert_eq!(p["repeated"]["maximum"], 10);
}

#[corvis::test]
async fn float_sign_uses_exclusive_bounds_and_integers_keep_the_original_values() {
    let s = schemas().await;
    let p = &s["SignDto"]["properties"];
    assert_eq!(p["count"], json!({ "type": "number", "minimum": 1 }));
    assert_eq!(p["debt"], json!({ "type": "number", "maximum": -1 }));
    assert_eq!(p["ratio"], json!({ "type": "number", "exclusiveMinimum": 0 }));
    assert_eq!(p["drop"], json!({ "type": "number", "exclusiveMaximum": 0 }));
}

#[corvis::test]
async fn contains_is_escaped() {
    let s = schemas().await;
    assert_eq!(s["PatternDto"]["properties"]["hasDot"]["pattern"], "a\\.b");
}

#[corvis::test]
async fn optional_fields_stay_optional() {
    let s = schemas().await;
    assert!(s["OptionalDto"].get("required").is_none());
    assert_eq!(s["OptionalDto"]["properties"]["nick"], json!({ "type": "string", "maxLength": 5 }));
}

#[corvis::test]
async fn explicit_option_wins() {
    let s = schemas().await;
    let p = &s["ExplicitDto"]["properties"];
    assert_eq!(p["capped"]["maximum"], 30);
    assert_eq!(p["label"], json!({ "type": "string", "maxLength": 10, "description": "Label" }));
}

#[corvis::test]
async fn enum_lists_the_variants() {
    let s = schemas().await;
    assert_eq!(s["EnumDto"]["properties"]["role"]["enum"], json!(["admin", "user"]));
}

/// Differences from the original that are intended, by schema and property: the value the original records and
/// the value Corvis produces.
fn known_differences() -> Vec<(&'static str, &'static str, Value, Value)> {
    vec![
        ("SignDto", "ratio", json!({ "type": "number", "minimum": 1 }), json!({ "type": "number", "exclusiveMinimum": 0 })),
        ("SignDto", "drop", json!({ "type": "number", "maximum": -1 }), json!({ "type": "number", "exclusiveMaximum": 0 })),
        ("PatternDto", "hasDot", json!({ "type": "string", "pattern": "a.b" }), json!({ "type": "string", "pattern": "a\\.b" })),
    ]
}

#[corvis::test]
async fn fixture_replay_matches_the_original() {
    let raw =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/class_validator_shim/class_validator_shim.json"))
            .unwrap();
    let fixture: Value = serde_json::from_str(&raw).unwrap();
    let recorded = fixture["schemas"].as_object().unwrap();
    assert!(recorded.len() >= 12);

    let ours = schemas().await;
    let differences = known_differences();
    for (name, original) in recorded {
        let mut expected = original.clone();
        for (schema, prop, was, now) in &differences {
            if schema == name {
                assert_eq!(&expected["properties"][prop], was, "{name}.{prop}: the fixture no longer records the difference");
                expected["properties"][prop] = now.clone();
            }
        }
        assert_eq!(ours[name], expected, "schema {name}");
    }
}

// Every mapped rule once, with both orders of the derives.

macro_rules! every_rule {
    ($name:ident, $($derive:ident),+) => {
        #[derive(Deserialize, $($derive),+)]
        #[allow(dead_code)]
        pub struct $name {
            #[min(1)] pub a: i64,
            #[max(9)] pub b: i64,
            #[min_length(1)] pub c: String,
            #[max_length(9)] pub d: String,
            #[length(2, 4)] pub e: String,
            #[matches("^a$")] pub f: String,
            #[contains("z")] pub g: String,
            #[is_email] pub h: String,
            #[is_url] pub i: String,
            #[is_uuid] pub j: String,
            #[is_hex_color] pub k: String,
            #[is_positive] pub l: i64,
            #[is_negative] pub m: i64,
            #[array_not_empty] pub n: Vec<String>,
            #[array_unique] pub o: Vec<String>,
            #[array_min_size(2)] pub p: Vec<String>,
            #[array_max_size(3)] pub q: Vec<String>,
            #[is_in("x", "y")] pub r: String,
        }
    };
}
every_rule!(RulesAB, Validate, ApiSchema);
every_rule!(RulesBA, ApiSchema, Validate);

fn check_every_rule(def: corvis::openapi::ApiSchemaDef) {
    let get = |n: &str| def.properties.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap();
    assert_eq!(get("a")["minimum"], 1);
    assert_eq!(get("b")["maximum"], 9);
    assert_eq!(get("c")["minLength"], 1);
    assert_eq!(get("d")["maxLength"], 9);
    assert_eq!((get("e")["minLength"].clone(), get("e")["maxLength"].clone()), (json!(2), json!(4)));
    assert_eq!(get("f")["pattern"], "^a$");
    assert_eq!(get("g")["pattern"], "z");
    assert_eq!(get("h")["format"], "email");
    assert_eq!(get("i")["format"], "uri");
    assert_eq!(get("j")["format"], "uuid");
    assert!(get("k")["pattern"].as_str().unwrap().starts_with("^#?("));
    assert_eq!(get("l")["minimum"], 1);
    assert_eq!(get("m")["maximum"], -1);
    assert_eq!(get("n")["minItems"], 1);
    assert_eq!(get("o")["uniqueItems"], true);
    assert_eq!(get("p")["minItems"], 2);
    assert_eq!(get("q")["maxItems"], 3);
    assert_eq!(get("r")["enum"], json!(["x", "y"]));
}

#[test]
fn every_mapped_rule_in_both_derive_orders() {
    use corvis::openapi::ApiSchema as _;
    check_every_rule(RulesAB::api_schema_def());
    check_every_rule(RulesBA::api_schema_def());
}

#[test]
fn escape_pattern_escapes_regex_metacharacters() {
    assert_eq!(
        corvis_swagger::__private::escape_pattern("a.b*(c)[d]{e}|f\\g^h$i+j?k"),
        "a\\.b\\*\\(c\\)\\[d\\]\\{e\\}\\|f\\\\g\\^h\\$i\\+j\\?k"
    );
    assert_eq!(corvis_swagger::__private::escape_pattern("plain-text_1"), "plain-text_1");
}

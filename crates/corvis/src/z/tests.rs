use serde::Deserialize;
use serde_json::json;

use crate::z::{self, ZodIssueCode, ZodSchema};

#[test]
fn primitives_and_messages() {
    z::with_version(z::ZodVersion::V3, primitives_and_messages_v3)
}

fn primitives_and_messages_v3() {
    assert_eq!(z::string().parse(&json!("hi")).unwrap(), json!("hi"));
    let e = z::string().parse(&json!(42)).unwrap_err();
    assert_eq!(e.issues[0].code, ZodIssueCode::InvalidType);
    assert_eq!(e.issues[0].message, "Expected string, received number");
    assert_eq!(
        e.issues[0].to_json(),
        json!({ "code": "invalid_type", "expected": "string", "received": "number", "path": [], "message": "Expected string, received number" })
    );

    let e = z::string().min(3).email().parse(&json!("ab")).unwrap_err();
    assert_eq!(e.messages(), vec!["String must contain at least 3 character(s)", "Invalid email"]);

    let e = z::string().min(3).message("Muy corto").parse(&json!("a")).unwrap_err();
    assert_eq!(e.messages(), vec!["Muy corto"]);

    assert_eq!(z::string().trim().to_lower_case().parse(&json!("  HoLa ")).unwrap(), json!("hola"));
    assert!(z::string().uuid().is_valid(&json!("123e4567-e89b-12d3-a456-426614174000")));
    assert!(z::string().url().is_valid(&json!("https://corvis.dev/docs?x=1")));
    assert!(!z::string().url().is_valid(&json!("not a url")));
    assert!(z::string().datetime().is_valid(&json!("2024-01-01T10:00:00.123Z")));
    assert!(!z::string().datetime().is_valid(&json!("2024-01-01T10:00:00+02:00")));
    assert!(z::string().datetime_offset().is_valid(&json!("2024-01-01T10:00:00+02:00")));
    assert!(z::string().regex(r"^\d+$").is_valid(&json!("123")));
    assert!(z::string().ip().is_valid(&json!("::1")));

    assert_eq!(z::number().int().parse(&json!(5)).unwrap(), json!(5));
    let e = z::number().int().positive().parse(&json!(-1.5)).unwrap_err();
    assert_eq!(e.messages(), vec!["Expected integer, received float", "Number must be greater than 0"]);
    let e = z::number().min(1).max(10).parse(&json!(11)).unwrap_err();
    assert_eq!(e.messages(), vec!["Number must be less than or equal to 10"]);
    let e = z::number().multiple_of(5).parse(&json!(7)).unwrap_err();
    assert_eq!(e.issues[0].code, ZodIssueCode::NotMultipleOf);

    assert_eq!(z::boolean().parse(&json!(true)).unwrap(), json!(true));
    assert_eq!(z::literal("a").parse(&json!("a")).unwrap(), json!("a"));
    assert_eq!(z::literal("a").parse(&json!("b")).unwrap_err().messages(), vec!["Invalid literal value, expected \"a\""]);
    assert_eq!(z::null().parse(&json!(null)).unwrap(), json!(null));
    assert!(z::never().parse(&json!(1)).is_err());
}

#[test]
fn coercion() {
    z::with_version(z::ZodVersion::V3, coercion_v3)
}

fn coercion_v3() {
    assert_eq!(z::coerce::number().parse(&json!("42")).unwrap(), json!(42));
    assert_eq!(z::coerce::number().parse(&json!("4.5")).unwrap(), json!(4.5));
    assert_eq!(z::coerce::number().parse(&json!("abc")).unwrap_err().messages(), vec!["Expected number, received nan"]);
    assert_eq!(z::coerce::string().parse(&json!(12)).unwrap(), json!("12"));
    assert_eq!(z::coerce::boolean().parse(&json!("false")).unwrap(), json!(true));
    assert_eq!(z::coerce::boolean().parse(&json!("")).unwrap(), json!(false));
    assert_eq!(z::coerce::boolean().parse(&json!("true")).unwrap(), json!(true));
    assert_eq!(z::coerce::date().parse(&json!(0)).unwrap(), json!("1970-01-01T00:00:00.000Z"));
    assert!(z::date().is_valid(&json!("2024-02-29")));
    assert_eq!(z::date().parse(&json!("2023-02-30")).unwrap_err().messages(), vec!["Invalid date"]);
}

#[test]
fn enums() {
    z::with_version(z::ZodVersion::V3, enums_v3)
}

fn enums_v3() {
    let role = z::enum_(["admin", "user"]);
    assert_eq!(role.options(), &["admin".to_string(), "user".to_string()]);
    let e = role.parse(&json!("root")).unwrap_err();
    assert_eq!(e.messages(), vec!["Invalid enum value. Expected 'admin' | 'user', received 'root'"]);
    assert_eq!(e.issues[0].to_json()["options"], json!(["admin", "user"]));
    assert!(role.exclude(&["admin"]).parse(&json!("admin")).is_err());

    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    #[allow(dead_code)]
    enum Color {
        Red,
        Green,
    }
    assert!(z::native_enum::<Color>().is_valid(&json!("green")));
    assert!(!z::native_enum::<Color>().is_valid(&json!("blue")));
}

#[test]
fn objects() {
    z::with_version(z::ZodVersion::V3, objects_v3)
}

fn objects_v3() {
    let user = z::object! {
        name: z::string().min(2),
        email: z::string().email(),
        age: z::number().int().nonnegative().optional(),
        role: z::enum_(["admin", "user"]).default("user"),
        "display-name": z::string().nullish(),
    };

    let out = user.parse(&json!({ "name": "Ana", "email": "ana@corvis.dev", "extra": 1 })).unwrap();
    assert_eq!(out, json!({ "name": "Ana", "email": "ana@corvis.dev", "role": "user" }));

    let e = user.parse(&json!({ "name": "A", "age": -1 })).unwrap_err();
    assert_eq!(
        e.to_json(),
        json!([
            { "code": "too_small", "minimum": 2, "type": "string", "inclusive": true, "exact": false, "path": ["name"], "message": "String must contain at least 2 character(s)" },
            { "code": "invalid_type", "expected": "string", "received": "undefined", "path": ["email"], "message": "Required" },
            { "code": "too_small", "minimum": 0, "type": "number", "inclusive": true, "exact": false, "path": ["age"], "message": "Number must be greater than or equal to 0" },
        ])
    );
    assert_eq!(
        e.flatten(),
        json!({
            "formErrors": [],
            "fieldErrors": {
                "name": ["String must contain at least 2 character(s)"],
                "email": ["Required"],
                "age": ["Number must be greater than or equal to 0"],
            }
        })
    );
    assert_eq!(e.format()["email"]["_errors"], json!(["Required"]));

    let strict = user.clone().strict();
    let e = strict.parse(&json!({ "name": "Ana", "email": "a@b.co", "foo": 1, "bar": 2 })).unwrap_err();
    assert_eq!(e.messages(), vec!["Unrecognized key(s) in object: 'foo', 'bar'"]);

    let pass = user.clone().passthrough();
    assert_eq!(pass.parse(&json!({ "name": "Ana", "email": "a@b.co", "foo": 1 })).unwrap()["foo"], json!(1));

    // pick / omit / partial / extend / required / keyof
    let login = user.clone().pick(&["email"]);
    assert_eq!(login.shape().len(), 1);
    let update = user.clone().omit(&["role"]).partial();
    assert_eq!(update.parse(&json!({})).unwrap(), json!({}));
    let admin = user.clone().extend(z::object! { level: z::number() });
    assert!(admin.parse(&json!({ "name": "Ana", "email": "a@b.co" })).is_err());
    let required = update.clone().required();
    assert!(required.parse(&json!({})).is_err());
    assert_eq!(user.keyof().options().len(), 5);

    // nested paths
    let order = z::object! {
        items: z::array(z::object! { sku: z::string(), qty: z::int().positive() }).nonempty(),
    };
    let e = order.parse(&json!({ "items": [{ "sku": "a", "qty": 1 }, { "sku": 2, "qty": 0 }] })).unwrap_err();
    assert_eq!(e.issues[0].path, vec![json!("items"), json!(1), json!("sku")]);
    assert_eq!(e.issues[1].path_string(), "items.1.qty");
    let e = order.parse(&json!({ "items": [] })).unwrap_err();
    assert_eq!(e.messages(), vec!["Array must contain at least 1 element(s)"]);
}

#[test]
fn unions_refinements_transforms() {
    z::with_version(z::ZodVersion::V3, unions_refinements_transforms_v3)
}

fn unions_refinements_transforms_v3() {
    let id = z::string().or(z::number());
    assert!(id.is_valid(&json!("a")));
    assert!(id.is_valid(&json!(1)));
    let e = id.parse(&json!(true)).unwrap_err();
    assert_eq!(e.issues[0].code, ZodIssueCode::InvalidUnion);
    assert_eq!(e.messages(), vec!["Invalid input"]);

    // a "dirty" option reports its own issues
    let short = z::string().max(3).or(z::number());
    assert_eq!(short.parse(&json!("abcdef")).unwrap_err().messages(), vec!["String must contain at most 3 character(s)"]);

    let shape = z::discriminated_union(
        "kind",
        [
            z::object! { kind: z::literal("circle"), radius: z::number() }.to_schema(),
            z::object! { kind: z::literal("square"), side: z::number() }.to_schema(),
        ],
    );
    assert!(shape.is_valid(&json!({ "kind": "circle", "radius": 2 })));
    let e = shape.parse(&json!({ "kind": "triangle" })).unwrap_err();
    assert_eq!(e.messages(), vec!["Invalid discriminator value. Expected 'circle' | 'square'"]);
    assert_eq!(e.issues[0].path, vec![json!("kind")]);

    let signup = z::object! { password: z::string().min(8), confirm: z::string() }.refine_at(
        |v| v["password"] == v["confirm"],
        "Passwords don't match",
        ["confirm"],
    );
    let e = signup.parse(&json!({ "password": "12345678", "confirm": "nope" })).unwrap_err();
    assert_eq!(e.issues[0].path, vec![json!("confirm")]);
    assert_eq!(e.issues[0].message, "Passwords don't match");

    let even = z::int().super_refine(|v, ctx| {
        if v.as_i64().unwrap() % 2 != 0 {
            ctx.add_issue("must be even");
        }
    });
    assert!(even.is_valid(&json!(4)));
    assert_eq!(even.parse(&json!(3)).unwrap_err().messages(), vec!["must be even"]);

    let len = z::string().transform(|v| json!(v.as_str().unwrap().len()));
    assert_eq!(len.parse(&json!("abc")).unwrap(), json!(3));
    let piped = z::string().transform(|v| json!(v.as_str().unwrap().len())).pipe(z::number().min(5));
    assert!(piped.parse(&json!("abc")).is_err());

    assert_eq!(z::number().catch(0).parse(&json!("x")).unwrap(), json!(0));
    assert_eq!(z::array(z::string()).default(json!([])).parse_undefined().unwrap(), json!([]));
    let pre = z::preprocess(|v| v.and_then(|v| v.as_str()).map(|s| json!(s.split(',').collect::<Vec<_>>())), z::array(z::string()));
    assert_eq!(pre.parse(&json!("a,b")).unwrap(), json!(["a", "b"]));

    let both = z::object! { a: z::string() }.and(z::object! { b: z::number() });
    assert_eq!(both.parse(&json!({ "a": "x", "b": 1 })).unwrap(), json!({ "a": "x", "b": 1 }));

    let rec = z::record(z::number());
    assert!(rec.is_valid(&json!({ "a": 1, "b": 2 })));
    assert_eq!(rec.parse(&json!({ "a": "x" })).unwrap_err().issues[0].path, vec![json!("a")]);
    let tup = z::tuple([z::string().into(), z::number().into()]);
    assert!(tup.is_valid(&json!(["a", 1])));
    assert!(!tup.is_valid(&json!(["a", 1, 2])));

    // recursive
    fn category() -> z::Schema {
        z::object! { name: z::string(), children: z::array(z::lazy(category)).optional() }.into()
    }
    assert!(category().is_valid(&json!({ "name": "a", "children": [{ "name": "b", "children": [] }] })));
    assert!(!category().is_valid(&json!({ "name": "a", "children": [{ "name": 1 }] })));
}

#[test]
fn typed_parse() {
    z::with_version(z::ZodVersion::V3, typed_parse_v3)
}

fn typed_parse_v3() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Dto {
        name: String,
        tags: Vec<String>,
    }
    let schema = z::object! { name: z::string().trim(), tags: z::array(z::string()).default(json!([])) };
    let dto: Dto = schema.parse_as(&json!({ "name": "  Ana " })).unwrap();
    assert_eq!(dto, Dto { name: "Ana".into(), tags: vec![] });
    let r = schema.safe_parse(&json!({}));
    assert!(!r.success);
    assert_eq!(r.error.unwrap().messages(), vec!["Required"]);
}

// ---------------------------------------------------------------------------
// zod 4 (default version).
// ---------------------------------------------------------------------------

fn v4_issues(schema: &impl ZodSchema, input: serde_json::Value) -> serde_json::Value {
    z::with_version(z::ZodVersion::V4, || schema.safe_parse(&input).error.expect("expected failure").to_json())
}

fn v4_messages(schema: &impl ZodSchema, input: serde_json::Value) -> Vec<String> {
    v4_issues(schema, input).as_array().unwrap().iter().map(|i| i["message"].as_str().unwrap().to_string()).collect()
}

#[test]
fn v4_messages_of_ported_apps() {
    let strict = z::object! { name: z::string().min(1), email: z::string().email() }.strict();
    assert_eq!(
        v4_messages(&strict, json!({ "name": "", "email": "x", "extra": 1 })),
        vec!["Too small: expected string to have >=1 characters", "Invalid email address", "Unrecognized key: \"extra\""]
    );
    assert_eq!(v4_messages(&strict, json!({ "name": "a", "email": "a@b.co", "x": 1, "y": 2 })), vec!["Unrecognized keys: \"x\", \"y\""]);
    assert_eq!(
        v4_issues(&strict, json!({ "email": "a@b.co" })),
        json!([{ "expected": "string", "code": "invalid_type", "path": ["name"], "message": "Invalid input: expected string, received undefined" }])
    );
    let o = z::object! { a: z::string() };
    assert_eq!(v4_messages(&o, json!(null)), vec!["Invalid input: expected object, received null"]);
    z::with_version(z::ZodVersion::V4, || {
        let e = o.parse_opt(None, &z::ParseOptions::new()).unwrap_err();
        assert_eq!(e.issues[0].message, "Invalid input: expected object, received undefined");
    });
    let cur = z::enum_(["USD", "VES", "USDT"]);
    assert_eq!(v4_messages(&cur, json!("EUR")), vec!["Invalid option: expected one of \"USD\"|\"VES\"|\"USDT\""]);
    assert_eq!(v4_messages(&z::number().min(1), json!(0)), vec!["Too small: expected number to be >=1"]);
    assert_eq!(v4_messages(&z::coerce::number(), json!("abc")), vec!["Invalid input: expected number, received NaN"]);
    assert_eq!(
        v4_issues(&z::email(), json!("nope")),
        json!([{ "origin": "string", "code": "invalid_format", "format": "email", "pattern": r"/^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/", "path": [], "message": "Invalid email address" }])
    );
    assert_eq!(
        v4_issues(&z::int(), json!(1.5)),
        json!([{ "expected": "int", "format": "safeint", "code": "invalid_type", "path": [], "message": "Invalid input: expected int, received number" }])
    );
}

#[test]
fn v4_pick_extend_partial_keep_strict() {
    let model = z::object! { id: z::string(), currency: z::enum_(["USD", "VES"]), amount: z::number() }.strict();
    let dto = model.clone().pick(&["currency"]).extend(z::object! { note: z::string().optional() });
    assert_eq!(v4_messages(&dto, json!({ "currency": "USD", "id": "x" })), vec!["Unrecognized key: \"id\""]);
    let ok = z::with_version(z::ZodVersion::V4, || dto.parse(&json!({ "currency": "USD" })).unwrap());
    assert_eq!(ok, json!({ "currency": "USD" }));
    let partial = model.partial();
    assert_eq!(z::with_version(z::ZodVersion::V4, || partial.parse(&json!({})).unwrap()), json!({}));
}

#[test]
fn v4_coerce_defaults_refine() {
    let q = z::object! {
        page: z::coerce::number().int().min(1).default(1),
        active: z::coerce::boolean().default(true),
    };
    z::with_version(z::ZodVersion::V4, || {
        assert_eq!(q.parse(&json!({})).unwrap(), json!({ "page": 1, "active": true }));
        assert_eq!(q.parse(&json!({ "page": "3", "active": "" })).unwrap(), json!({ "page": 3, "active": false }));
    });
    let fx = z::object! { from: z::string(), to: z::string() }
        .refine(|v| v["from"] != v["to"], z::params().message("Currencies must differ").path(["to"]));
    assert_eq!(
        v4_issues(&fx, json!({ "from": "USD", "to": "USD" })),
        json!([{ "code": "custom", "path": ["to"], "message": "Currencies must differ" }])
    );
    let rec = z::record_with(z::string().min(1), z::string());
    assert_eq!(
        v4_issues(&rec, json!({ "": "x" })),
        json!([{ "code": "invalid_key", "origin": "record", "issues": [{ "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true, "path": [], "message": "Too small: expected string to have >=1 characters" }], "path": [""], "message": "Invalid key in record" }])
    );
}

#[test]
fn v4_unions_and_errors() {
    let u = z::union([z::string().into(), z::number().into()]);
    assert_eq!(
        v4_issues(&u, json!(true)),
        json!([{ "code": "invalid_union", "errors": [
            [{ "expected": "string", "code": "invalid_type", "path": [], "message": "Invalid input: expected string, received boolean" }],
            [{ "expected": "number", "code": "invalid_type", "path": [], "message": "Invalid input: expected number, received boolean" }]
        ], "path": [], "message": "Invalid input" }])
    );
    let du = z::discriminated_union(
        "type",
        [
            z::object! { "type": z::literal(json!("a")), a: z::string() }.into(),
            z::object! { "type": z::literal(json!("b")), b: z::number() }.into(),
        ],
    );
    assert_eq!(
        v4_issues(&du, json!({ "type": "c" })),
        json!([{ "code": "invalid_union", "errors": [], "note": "No matching discriminator", "discriminator": "type", "options": ["a", "b"], "path": ["type"], "message": "Invalid discriminator value. Expected 'a' | 'b'" }])
    );
    let e = z::with_version(z::ZodVersion::V4, || {
        z::object! { name: z::string(), tags: z::array(z::string()) }.safe_parse(&json!({ "tags": [1] })).error.unwrap()
    });
    assert_eq!(
        e.flatten(),
        json!({ "formErrors": [], "fieldErrors": { "name": ["Invalid input: expected string, received undefined"], "tags": ["Invalid input: expected string, received number"] } })
    );
    assert_eq!(
        z::prettify_error(&e),
        "✖ Invalid input: expected string, received undefined\n  → at name\n✖ Invalid input: expected string, received number\n  → at tags[0]"
    );
}

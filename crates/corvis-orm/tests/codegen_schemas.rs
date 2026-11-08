//! Compiles the models generated from the three real-world schemas
//! (`corvis-orm generate`) and checks their metadata.

#[allow(dead_code)]
#[allow(clippy::doc_lazy_continuation)] // generated doc comments copied verbatim from the schema (list items without indentation)
mod facten {
    include!("generated/facten.rs");
}
#[allow(dead_code)]
mod consi {
    include!("generated/consi.rs");
}
#[allow(dead_code)]
mod houseplan {
    include!("generated/houseplan.rs");
}

use corvis_orm::{FieldKind, Model, OrmEnum};

#[test]
fn houseplan_metadata() {
    let m = houseplan::FxRate::meta();
    assert_eq!(m.table, "fx_rates");
    assert_eq!(m.primary_key, &["date", "source"]);
    assert_eq!(m.compound_pk_name().as_deref(), Some("date_source"));
    let t = houseplan::Transaction::meta();
    let names: Vec<&str> = t.fields.iter().map(|f| f.name).collect();
    assert_eq!(&names[..4], &["id", "type", "status", "occurredAt"]);
    let e = houseplan::Embedding::meta();
    assert_eq!(e.unsupported.len(), 1);
    assert_eq!(e.unsupported[0].db_type, "vector(384)");
    let a = houseplan::Account::meta();
    let aliases = a.field("payMethodAliases").unwrap();
    assert!(aliases.list);
    assert_eq!(aliases.kind, FieldKind::String);
    let entry = houseplan::PlanEntry::meta();
    assert_eq!(entry.field("month").unwrap().native_name(), Some("Date"));
}

#[test]
fn consi_enums_and_uniques() {
    assert!(consi::Terminal::meta().field("methods").unwrap().list);
    assert_eq!(consi::Terminal::meta().field("methods").unwrap().kind, FieldKind::Enum);
    assert!(consi::Wallet::meta().compound_unique("merchantId_currency").is_some());
    let _ = <consi::Currency as OrmEnum>::values();
}

#[test]
fn facten_maps() {
    let inv = facten::Invoice::meta();
    assert!(inv.fields.iter().any(|f| f.column != f.name));
}

#[test]
fn json_roundtrip_key_order() {
    let v = corvis_orm::serde_json::json!({
        "date": "2024-05-01T00:00:00.000Z", "source": "bcv", "vesPerUsd": "36.5"
    });
    let r: houseplan::FxRate = corvis_orm::serde_json::from_value(v.clone()).unwrap();
    assert_eq!(corvis_orm::serde_json::to_value(&r).unwrap(), v);
    assert_eq!(corvis_orm::serde_json::to_string(&r).unwrap(), r#"{"date":"2024-05-01T00:00:00.000Z","source":"bcv","vesPerUsd":"36.5"}"#);
}

//! What a `Decimal` operand costs on SQLite (security review of #188, finding 3): every operand is sent in the
//! shortest form, never as a fixed-point expansion of its exponent (`"1e131071"` was a 131,072-digit parameter).

use serde_json::json;

use super::recorder::capture;
use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, Model)]
#[model(table = "amounts")]
struct Amount {
    #[id]
    #[default(autoincrement)]
    id: i64,
    amount: Decimal,
}

#[tokio::test]
async fn decimal_operands_are_short_on_sqlite() {
    let db = OrmClient::new("sqlite::memory:").unwrap();
    db.db_push::<Amount>().await.unwrap();
    db.query_json(Amount::meta(), "create", json!({ "data": { "amount": "12.5" } })).await.unwrap();
    for (op, v) in [
        ("in", json!(["1e131071", "-1e131071", "1e-131071", "123.5", "1e30", "0.0000001"])),
        ("lt", json!("1e131071")),
        ("gt", json!("1e-131071")),
        ("equals", json!("1e131000")),
    ] {
        let (res, sent) = capture(db.query_json(Amount::meta(), "findMany", json!({ "where": { "amount": { op: v.clone() } } }))).await;
        res.unwrap();
        let longest = sent.iter().flat_map(|s| s.text_lengths.iter()).max().copied().unwrap_or(0);
        assert!(longest < 40, "{op} {v}: a {longest}-byte parameter");
    }
}

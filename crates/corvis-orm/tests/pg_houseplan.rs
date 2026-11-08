//! houseplan schema on Postgres: migrations + generated models.

#[path = "common/pg.rs"]
mod pg;

#[allow(dead_code)]
mod hp {
    include!("generated/houseplan.rs");
}

use corvis_orm::prelude::*;
use hp::*;

async fn setup(name: &str) -> Option<OrmClient> {
    let dir = pg::app_dir("houseplan/api/prisma/migrations")?;
    let url = pg::fresh_db(name).await?;
    let db = OrmClient::new(url).unwrap();
    db.migrate_deploy(&dir).await.unwrap();
    Some(db)
}

#[tokio::test]
async fn crud_and_includes() {
    let Some(db) = setup("hp_crud").await else { return };
    let acc: Account = db
        .account()
        .create(json!({ "data": { "code": "cash_usd", "name": "Cash", "currency": "USD", "kind": "ledger", "openingBalance": "10.5" } }))
        .await
        .unwrap();
    assert_eq!(acc.opening_balance.to_string(), "10.5");
    assert!(acc.pay_method_aliases.is_empty());
    let cat: Category = db.category().create(json!({ "data": { "name": "Food" } })).await.unwrap();
    let child: Category =
        db.category().create(json!({ "data": { "name": "Market", "parent": { "connect": { "id": cat.id } } } })).await.unwrap();
    assert_eq!(child.parent_id, Some(cat.id));
    let tx: Transaction = db
        .transaction()
        .create(json!({
            "data": {
                "type": "expense", "occurredAt": "2024-05-06T10:00:00.000Z", "amount": 12.34, "currency": "USD",
                "source": "manual_text", "fromAccountId": acc.id, "categoryId": child.id
            },
            "include": { "fromAccount": true, "category": { "include": { "parent": true } }, "toAccount": true }
        }))
        .await
        .unwrap();
    assert_eq!(tx.amount.to_string(), "12.34");
    assert_eq!(tx.status, "pending");
    let v = corvis_orm::serde_json::to_value(&tx).unwrap();
    assert_eq!(v["toAccount"], Value::Null);
    assert_eq!(v["fromAccount"]["code"], "cash_usd");
    assert_eq!(v["category"]["parent"]["name"], "Food");
    assert_eq!(v["occurredAt"], "2024-05-06T10:00:00.000Z");

    // Compound id upsert (`date_source`) with @db.Date.
    for rate in ["36.5", "37.25"] {
        let r: FxRate = db
            .fx_rate()
            .upsert(json!({
                "where": { "date_source": { "date": "2024-05-01T00:00:00.000Z", "source": "bcv" } },
                "create": { "date": "2024-05-01T00:00:00.000Z", "source": "bcv", "vesPerUsd": rate },
                "update": { "vesPerUsd": rate }
            }))
            .await
            .unwrap();
        assert_eq!(r.ves_per_usd.to_string(), rate);
    }
    assert_eq!(db.fx_rate().count(()).await.unwrap(), 1);

    // _count include.
    let a: Account = db
        .account()
        .find_unique_or_throw(json!({ "where": { "id": acc.id }, "include": { "_count": { "select": { "outgoing": true } } } }))
        .await
        .unwrap();
    assert_eq!(a._count, Some(json!({ "outgoing": 1 })));

    // Decimal increment and aggregate.
    let b: Bag = db
        .bag()
        .create(json!({ "data": { "p2pTransactionId": tx.id, "accountId": acc.id, "amountVes": "100", "remainingVes": "100", "rate": "36.5", "openedAt": "2024-05-01T00:00:00.000Z" } }))
        .await
        .unwrap();
    let b: Bag = db.bag().update(json!({ "where": { "id": b.id }, "data": { "remainingVes": { "decrement": 12.5 } } })).await.unwrap();
    assert_eq!(b.remaining_ves.to_string(), "87.5");
    let agg = db.transaction().aggregate(json!({ "_sum": { "amount": true }, "where": { "currency": "VES" } })).await.unwrap();
    assert_eq!(agg, json!({ "_sum": { "amount": null } }));
    let agg = db.transaction().aggregate(json!({ "_sum": { "amount": true } })).await.unwrap();
    assert_eq!(agg, json!({ "_sum": { "amount": "12.34" } }));

    // Int[] list.
    let t: ChatTurn = db.chat_turn().create(json!({ "data": { "role": "user", "text": "hi", "txIds": [1, 2] } })).await.unwrap();
    assert_eq!(t.tx_ids, vec![1, 2]);
}

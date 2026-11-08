//! `core/circular-dependencies`: provider cycles are reported, module import cycles are accepted.
//!
//! Every `compile()` goes through the helpers of `main.rs`, which fail after 20 seconds instead of
//! hanging if the container ever loops again.
//!
//! Not covered here (separate issues): cycles that pass
//! through `ModuleRef::resolve` inside a factory (#113) and transient providers that are only
//! validated when first resolved (#114).

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{assert_cycle, chain_names, compile_err, compile_ok};

#[injectable]
pub struct OrdersService {
    _invoices: Arc<InvoicesService>,
}

#[injectable]
pub struct InvoicesService {
    _orders: Arc<OrdersService>,
}

#[module(providers = [OrdersService, InvoicesService])]
pub struct TwoSingletonsModule;

#[corvis::test]
async fn two_singletons_that_inject_each_other() {
    let err = compile_err(Test::create_testing_module().import::<TwoSingletonsModule>()).await;
    assert_cycle(&err, &["OrdersService", "InvoicesService"]);
    // Either rotation is accepted; which side is built first is an implementation detail.
    let chain = chain_names(&err);
    assert_eq!(chain.len(), 3, "{chain:?}");
}

#[injectable]
pub struct Loop {
    _itself: Arc<Loop>,
}

#[module(providers = [Loop])]
pub struct SelfModule;

#[corvis::test]
async fn a_provider_that_injects_itself() {
    let err = compile_err(Test::create_testing_module().import::<SelfModule>()).await;
    assert_cycle(&err, &["Loop"]);
    assert_eq!(chain_names(&err), ["Loop", "Loop"]);
}

#[injectable]
pub struct RingA {
    _b: Arc<RingB>,
}
#[injectable]
pub struct RingB {
    _c: Arc<RingC>,
}
#[injectable]
pub struct RingC {
    _a: Arc<RingA>,
}

#[module(providers = [RingA, RingB, RingC])]
pub struct ThreeModule;

#[corvis::test]
async fn three_providers_in_a_ring() {
    let err = compile_err(Test::create_testing_module().import::<ThreeModule>()).await;
    assert_cycle(&err, &["RingA", "RingB", "RingC"]);
    assert_eq!(chain_names(&err).len(), 4);
}

#[injectable]
pub struct CrossOrders {
    _billing: Arc<CrossBilling>,
}
#[injectable]
pub struct CrossBilling {
    _orders: Arc<CrossOrders>,
}

#[module(imports = [CrossBillingModule], providers = [CrossOrders], exports = [CrossOrders])]
pub struct CrossOrdersModule;

#[module(imports = [CrossOrdersModule], providers = [CrossBilling], exports = [CrossBilling])]
pub struct CrossBillingModule;

#[corvis::test]
async fn a_cycle_across_modules() {
    let err = compile_err(Test::create_testing_module().import::<CrossOrdersModule>()).await;
    assert_cycle(&err, &["CrossOrders", "CrossBilling"]);
}

#[module(providers = [
    Provider::use_factory("X", |ctx| async move { let y = ctx.get_token::<String>("Y").await?; Ok(format!("x{y}")) }),
    Provider::use_factory("Y", |ctx| async move { let x = ctx.get_token::<String>("X").await?; Ok(format!("y{x}")) }),
])]
pub struct FactoryCycleModule;

#[corvis::test]
async fn factories_that_resolve_each_other() {
    let err = compile_err(Test::create_testing_module().import::<FactoryCycleModule>()).await;
    assert_cycle(&err, &["X", "Y"]);
}

#[module(providers = [Provider::use_existing("A", "B"), Provider::use_existing("B", "A")])]
pub struct AliasCycleModule;

#[corvis::test]
async fn aliases_that_point_to_each_other() {
    let err = compile_err(Test::create_testing_module().import::<AliasCycleModule>()).await;
    assert_cycle(&err, &["A", "B"]);
}

#[injectable(scope = "transient")]
pub struct Node {
    _next: Arc<Node>,
}

#[injectable]
pub struct NodeHolder {
    _node: Arc<Node>,
}

#[module(providers = [Node, NodeHolder])]
pub struct TransientSelfModule;

#[corvis::test]
async fn a_transient_provider_that_injects_itself() {
    let err = compile_err(Test::create_testing_module().import::<TransientSelfModule>()).await;
    let chain = chain_names(&err);
    assert!(matches!(err, DiError::Circular { .. }));
    assert_eq!(chain.last().map(String::as_str), Some("Node"), "{chain:?}");
    assert_eq!(chain.iter().filter(|n| *n == "Node").count(), 2, "{chain:?}");
}

// ---------------------------------------------------------------- module cycles

#[injectable]
pub struct OrdersApi;

#[injectable]
pub struct BillingApi;

#[injectable]
pub struct OrdersReport {
    billing: Arc<BillingApi>,
}

#[injectable]
pub struct BillingReport {
    orders: Arc<OrdersApi>,
}

#[module(imports = [BillingModule], providers = [OrdersApi, OrdersReport], exports = [OrdersApi])]
pub struct OrdersModule;

#[module(imports = [OrdersModule], providers = [BillingApi, BillingReport], exports = [BillingApi])]
pub struct BillingModule;

#[corvis::test]
async fn modules_that_import_each_other_start() {
    let module = compile_ok(Test::create_testing_module().import::<OrdersModule>()).await;
    let orders = module.get::<OrdersReport>().unwrap();
    let billing = module.get::<BillingReport>().unwrap();
    assert!(Arc::ptr_eq(&orders.billing, &module.get::<BillingApi>().unwrap()));
    assert!(Arc::ptr_eq(&billing.orders, &module.get::<OrdersApi>().unwrap()));
}

#[injectable]
pub struct Nobody;

#[injectable]
pub struct NeedsNobody {
    _nobody: Arc<Nobody>,
}

#[module(imports = [ReexportB], exports = [ReexportB])]
pub struct ReexportA;

#[module(imports = [ReexportA], exports = [ReexportA])]
pub struct ReexportB;

#[module(imports = [ReexportA], providers = [NeedsNobody])]
pub struct ReexportConsumer;

#[corvis::test]
async fn a_re_export_cycle_with_a_missing_token_terminates() {
    let err = compile_err(Test::create_testing_module().import::<ReexportConsumer>()).await;
    match err {
        DiError::UnknownDependencies { requester, token, .. } => {
            assert_eq!(requester, "NeedsNobody");
            assert_eq!(token, "Nobody");
        }
        other => panic!("expected UnknownDependencies, got {other:?}"),
    }
}

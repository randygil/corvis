//! Regression tests of the dependency injection container, through the public API only.
//! One module per documented area (see `openspec/changes/e13-di-test-coverage`).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use corvis::DiError;
use corvis::testing::{Test, TestingModule, TestingModuleBuilder};

mod custom_providers;
mod cycles;
mod dynamic_modules;
mod global_modules;
mod module_ref;
mod transient;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// A process-wide unique id, for `#[init(next_id())]` fields that tell instances apart.
pub fn next_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

/// Compiles the module, failing the test if it takes too long (a regression to an unbounded
/// recursion in the container would otherwise stall the suite).
pub async fn compile(builder: TestingModuleBuilder) -> corvis::Result<TestingModule> {
    match corvis::tokio::time::timeout(Duration::from_secs(20), builder.compile()).await {
        Ok(result) => result,
        Err(_) => panic!("compile() did not finish within 20 seconds"),
    }
}

/// Compiles the module and returns it, failing the test with the full error otherwise.
pub async fn compile_ok(builder: TestingModuleBuilder) -> TestingModule {
    match compile(builder).await {
        Ok(module) => module,
        Err(e) => panic!("the module should compile, but: {e:#}"),
    }
}

/// Compiles the module, expects a startup failure and returns its `DiError`.
pub async fn compile_err(builder: TestingModuleBuilder) -> DiError {
    match compile(builder).await {
        Ok(_) => panic!("compile() should have failed"),
        Err(e) => match e.downcast_ref::<DiError>() {
            Some(di) => di.clone(),
            None => panic!("the error is not a DiError: {e:#}"),
        },
    }
}

/// The names of a `DiError::Circular` chain, in order.
pub fn chain_names(err: &DiError) -> Vec<String> {
    match err {
        DiError::Circular { chain } => chain.split(" -> ").map(str::to_string).collect(),
        other => panic!("expected DiError::Circular, got {other:?}"),
    }
}

/// The error is `Circular`, its chain starts and ends with the same provider and names every provider in `names`.
pub fn assert_cycle(err: &DiError, names: &[&str]) {
    let chain = chain_names(err);
    assert!(chain.len() >= 2, "chain too short: {chain:?}");
    assert_eq!(chain.first(), chain.last(), "the chain must start and end with the same provider: {chain:?}");
    for name in names {
        assert!(chain.iter().any(|c| c == name), "{name} is missing from the chain {chain:?}");
    }
}

#[corvis::test]
async fn an_empty_testing_module_compiles() {
    compile_ok(Test::create_testing_module()).await;
}

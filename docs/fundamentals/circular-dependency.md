---
status: partial
---

# Circular dependencies

There are two kinds of cycle and Corvis treats them differently: between **modules** (A imports B and B imports A) and between **providers** (A needs B to be built and B needs A).

## Cycles between modules: allowed

Circular imports between static modules work without doing anything special. When walking the graph, a module that is already being processed is not created again: the second import points to the same node. The export lookup also keeps track of visited modules, so it doesn't loop.

```rust
#[module(imports = [BillingModule], providers = [OrdersService], exports = [OrdersService])]
pub struct OrdersModule;

#[module(imports = [OrdersModule], providers = [InvoicesService], exports = [InvoicesService])]
pub struct BillingModule;
```

No deferred reference is needed for modules. That the graph allows it doesn't mean it is good design: a cycle between modules usually indicates there is a third responsibility that deserves its own module.

### Dynamic modules are the exception

The rule above covers static modules. Every dynamic import (`BModule::for_root()`) creates a **new** module instance, so two dynamic modules that import each other would create instances forever. Corvis detects it while building the module graph and fails at startup:

```text
A circular import between dynamic modules has been detected: AModule -> BModule -> AModule. Each dynamic import (for example for_root()) creates a new module instance, so this cycle never ends. Move what both modules need into a separate module that both import, or remove one of the two imports.
```

To fix it, move what both modules need into a third module that both import, or remove one of the two imports. The check fires when the same module imports the same other module dynamically a second time on one chain of imports, so a graph that repeats such an import is rejected even if it would eventually end. See [Dynamic modules](/fundamentals/dynamic-modules#each-call-is-a-new-instance).

## Cycles between providers: detected and fail

If `OrdersService` has an `Arc<InvoicesService>` field and `InvoicesService` has an `Arc<OrdersService>` field, neither can be built before the other. Corvis detects it at startup and returns an error with the full chain:

```text
A circular dependency has been detected: OrdersService -> InvoicesService -> OrdersService. Break the cycle: move the shared logic into a third
provider, resolve one side lazily with ModuleRef (not in its constructor), or communicate through
EventEmitter. See "Circular dependencies" in the docs.
```

There is no deferred-reference mechanism on fields (`forwardRef` in other containers): use one of the following solutions.

### 1. Extract the shared part

The preferred solution. Almost always, each service needs only a part of the other. Move that part to a third provider that both depend on:

```text
OrdersService ──► PricingService ◄── InvoicesService
```

### 2. Resolve one of the sides later with `ModuleRef`

One side of the cycle stops asking for the other as a field and looks it up when it uses it. Since all singletons are created at startup, at request time the instance already exists:

```rust
#[injectable]
pub struct OrdersService {
    module_ref: ModuleRef,
}

impl OrdersService {
    fn invoices(&self) -> Arc<InvoicesService> {
        self.module_ref.get::<InvoicesService>().expect("InvoicesService is registered")
    }

    pub async fn close(&self, id: u64) -> HttpResult<()> {
        self.invoices().issue_for(id).await
    }
}

#[injectable]
pub struct InvoicesService {
    orders: Arc<OrdersService>,      // this side remains a normal field
}
```

`ModuleRef::get` first looks up using the module's visibility rules and, if it doesn't find it, across the whole application. To strictly respect encapsulation use `get_token_in_context`. See [Module reference](/fundamentals/module-ref).

Don't call `get` inside the constructor (for example in an `#[init(...)]`): during construction the other side doesn't exist yet. Calling `resolve` or `create` for the other side inside a constructor or factory is detected: startup fails with the circular-dependency error and the full chain (`OrdersService -> InvoicesService -> OrdersService`), the same as a cycle between fields. Work you start with `tokio::spawn` inside a constructor is not covered by that check.

### 3. Communicate with events

If a service only needs to *notify* the other, emit an event with `EventEmitter` and let the other one listen with `#[on_event]`. Tertulia does it this way: `RoomsService` emits `room.created` and `ChatGateway` listens to it, without `RoomsService` depending on the gateway:

```rust
// RoomsService
self.events.emit("room.created", room.clone());

// ChatGateway
#[handlers]
impl ChatGateway {
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}
```

See [Events](/application/events).

---
status: documented
---

# Events

Events let one part of the application announce that something happened without knowing who is interested. The service that creates a room does not need to know that the chat gateway wants to announce it: it emits `room.created` and the gateway listens for that event. Corvis solves this with `EventEmitterModule`, an in-memory emitter inside the same process.

## Registering the module

```rust
#[module(
    imports = [
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        // ...
    ],
)]
pub struct AppModule;
```

`EventEmitterOptions` has three fields:

| Field | Default | Effect |
|---|---|---|
| `wildcard` | `false` | Enables the `*` (one segment) and `**` (any number of segments) wildcards |
| `delimiter` | `"."` | Separator for the segments of the event name |
| `global` | `true` | Registers the module as global |

Since the module is global by default, any provider can request `Arc<EventEmitter>` without importing anything else.

## Emitting

```rust
#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let room: Room = self.db.room().create(json!({ "data": { /* ... */ } })).await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }
}
```

(From `examples/tertulia/src/rooms/rooms_service.rs`.) The payload can be any `Send + Sync + 'static` type; it does not need to be converted to JSON.

| Method | Behavior |
|---|---|
| `emit(event, payload)` | Launches each listener in a separate task and returns immediately. Returns `true` if there were listeners |
| `emit_async(event, payload).await` | Waits for all listeners and returns a `Vec` with the result of each one |
| `on(event, closure)` | Registers a listener at runtime; the closure receives the untyped payload (`Arc<dyn Any + Send + Sync>`) |
| `listener_count(event)` | How many listeners match the event |
| `remove_all_listeners(event)` | Removes the listeners registered with that exact pattern |

With `emit`, a listener's error is logged with the `EventEmitter` context and does not reach the emitter.

## Listening with `#[on_event]`

Listeners are declared in a `#[handlers]` block of any provider:

```rust
#[injectable]
pub struct CatCreatedListener {
    #[init(Logger::new("CatCreatedListener"))]
    logger: Logger,
}

#[handlers]
impl CatCreatedListener {
    #[on_event("cat.created")]
    async fn handle(&self, event: CatCreatedEvent) {
        self.logger.log(format!("New cat registered: {} (#{})", event.cat.name, event.cat.id));
    }
}
```

(From `examples/cats-app`.) The provider must be registered in some module (`providers = [CatCreatedListener]`). Corvis walks the providers when the application initializes and connects their handlers to the emitter.

Method rules:

- It can be `async` or synchronous, and return `()` or a `Result` (an `Err` is logged as a listener error).
- The first parameter, if present, is the payload. Its type must be **exactly** the one that was emitted and must implement `Clone`: the payload is retrieved by type. If the type does not match, the listener is simply not run, without error. If you emit `json!(...)`, declare the parameter as `Value`.
- Without parameters, the method runs without receiving the payload.
- A single method can listen to several events: `#[on_event("cat.created", "cat.updated")]`.

If a provider uses `#[on_event]` but `EventEmitterModule::for_root` is not imported, startup continues and a warning is logged.

### Wildcards

With `wildcard: true`, patterns with `*` and `**` work:

```rust
EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() })

#[handlers]
impl OrderListener {
    #[on_event("order.*")]       // order.created, order.paid...
    fn on_any(&self, e: OrderCreated) -> corvis::Result<()> { Ok(()) }
}
```

Without `wildcard`, `"order.*"` only matches an event literally named `order.*`.

## Full example: from REST to WebSocket

In Tertulia, `POST /api/rooms` creates the room and `RoomsService` emits `room.created`. The chat gateway listens for the event and notifies all connected clients:

```rust
#[handlers]
impl ChatGateway {
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}
```

The REST controller and the gateway do not know each other; the event name ties them together.

## Limits

- The emitter lives in memory: it does not persist events or distribute them across processes or instances. If you need that, use an external broker (see [Queues](/application/queues)).
- The listeners of `emit` run in parallel and with no guaranteed order among them.

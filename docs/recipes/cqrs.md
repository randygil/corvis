---
status: unavailable
---

# CQRS

Corvis **has no CQRS module**: there are no command, query or event buses with handlers registered by type, and no sagas.

## Alternative: services and `EventEmitter`

For most applications it is enough to separate reads from writes into different services and publish domain events with the framework's event emitter. This is the pattern `examples/tertulia` uses: creating a room over REST reaches the WebSocket clients through an event.

```rust
#[module(
    imports = [EventEmitterModule::for_root(EventEmitterOptions::default())],
    /* ... */
)]
pub struct AppModule;
```

The write side publishes the event (`src/rooms/rooms_service.rs`):

```rust
#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let room: Room = self
            .db
            .room()
            .create(json!({
                "data": { "name": dto.name, "topic": dto.topic, "maxMembers": dto.max_members, "createdBy": created_by }
            }))
            .await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }
}
```

And any provider reacts with a handler (`src/chat/chat_gateway.rs`):

```rust
#[handlers]
impl ChatGateway {
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}
```

The handler receives the same type that was emitted (here `Room`). See [Events](/application/events) for wildcards and emitter options.

If you need a command bus with one handler per type, you can build it as your own provider; there is no official implementation.

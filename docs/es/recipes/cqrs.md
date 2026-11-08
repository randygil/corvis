---
status: unavailable
---

# CQRS

Corvis **no tiene un módulo de CQRS**: no hay buses de comandos, consultas ni eventos con handlers registrados por tipo, ni sagas.

## Alternativa: servicios y `EventEmitter`

Para la mayoría de las aplicaciones basta con separar las lecturas de las escrituras en servicios distintos y publicar eventos de dominio con el emisor de eventos del framework. Es el patrón que usa `examples/tertulia`: crear una sala por REST se propaga a los clientes WebSocket a través de un evento.

```rust
#[module(
    imports = [EventEmitterModule::for_root(EventEmitterOptions::default())],
    /* ... */
)]
pub struct AppModule;
```

El lado de escritura publica el evento (`src/rooms/rooms_service.rs`):

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

Y cualquier provider reacciona con un handler (`src/chat/chat_gateway.rs`):

```rust
#[handlers]
impl ChatGateway {
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}
```

El handler recibe el mismo tipo que se emitió (aquí `Room`). Ver [Eventos](/es/application/events) para los comodines y las opciones del emisor.

Si necesitas un bus de comandos con un handler por tipo, puedes construirlo como un provider propio; no hay una implementación oficial.

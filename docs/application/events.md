---
status: documentado
---

# Eventos

Los eventos permiten que una parte de la aplicación avise de que algo ocurrió sin conocer a quién le interesa. El servicio que crea una sala no tiene por qué saber que el gateway de chat quiere anunciarla: emite `room.created` y el gateway escucha ese evento. Corvis lo resuelve con `EventEmitterModule`, un emisor en memoria dentro del mismo proceso.

## Registrar el módulo

```rust
#[module(
    imports = [
        EventEmitterModule::for_root(EventEmitterOptions::default()),
        // ...
    ],
)]
pub struct AppModule;
```

`EventEmitterOptions` tiene tres campos:

| Campo | Por defecto | Efecto |
|---|---|---|
| `wildcard` | `false` | Activa los comodines `*` (un segmento) y `**` (cualquier número de segmentos) |
| `delimiter` | `"."` | Separador de segmentos del nombre del evento |
| `global` | `true` | Registra el módulo como global |

Como el módulo es global por defecto, cualquier provider puede pedir `Arc<EventEmitter>` sin importar nada más.

## Emitir

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

(De `examples/tertulia/src/rooms/rooms_service.rs`.) El payload puede ser cualquier tipo `Send + Sync + 'static`; no hace falta convertirlo a JSON.

| Método | Comportamiento |
|---|---|
| `emit(evento, payload)` | Lanza cada listener en una tarea aparte y vuelve de inmediato. Devuelve `true` si había listeners |
| `emit_async(evento, payload).await` | Espera a todos los listeners y devuelve un `Vec` con el resultado de cada uno |
| `on(evento, closure)` | Registra un listener en tiempo de ejecución; la closure recibe el payload sin tipar (`Arc<dyn Any + Send + Sync>`) |
| `listener_count(evento)` | Cuántos listeners coinciden con el evento |
| `remove_all_listeners(evento)` | Quita los listeners registrados con ese patrón exacto |

Con `emit`, el error de un listener se registra con el contexto `EventEmitter` y no llega a quien emitió.

## Escuchar con `#[on_event]`

Los listeners se declaran en un bloque `#[handlers]` de cualquier provider:

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

(De `examples/cats-app`.) El provider tiene que estar registrado en algún módulo (`providers = [CatCreatedListener]`). Corvis recorre los providers al inicializar la aplicación y conecta sus handlers al emisor.

Reglas del método:

- Puede ser `async` o síncrono, y devolver `()` o un `Result` (un `Err` se registra como error del listener).
- El primer parámetro, si existe, es el payload. Su tipo debe ser **exactamente** el que se emitió y debe implementar `Clone`: el payload se recupera por tipo. Si el tipo no coincide, el listener simplemente no se ejecuta, sin error. Si emites `json!(...)`, declara el parámetro como `Value`.
- Sin parámetros, el método se ejecuta sin recibir el payload.
- Un mismo método puede escuchar varios eventos: `#[on_event("cat.created", "cat.updated")]`.

Si un provider usa `#[on_event]` pero `EventEmitterModule::for_root` no está importado, el arranque continúa y se registra un aviso.

### Comodines

Con `wildcard: true`, los patrones con `*` y `**` funcionan:

```rust
EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() })

#[handlers]
impl OrderListener {
    #[on_event("order.*")]       // order.created, order.paid...
    fn on_any(&self, e: OrderCreated) -> corvis::Result<()> { Ok(()) }
}
```

Sin `wildcard`, `"order.*"` solo coincide con un evento llamado literalmente `order.*`.

## Ejemplo completo: de REST a WebSocket

En Tertulia, `POST /api/rooms` crea la sala y `RoomsService` emite `room.created`. El gateway de chat escucha el evento y avisa a todos los clientes conectados:

```rust
#[handlers]
impl ChatGateway {
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}
```

El controlador REST y el gateway no se conocen entre sí; los une el nombre del evento.

## Límites

- El emisor vive en memoria: no persiste eventos ni los reparte entre procesos o instancias. Si necesitas eso, usa un broker externo (ver [Colas](/application/queues)).
- Los listeners de `emit` corren en paralelo y sin orden garantizado entre ellos.

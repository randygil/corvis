---
status: documented
---

# Contexto de ejecución

Guards, interceptors, filtros de excepción y decoradores de parámetro (`FromContext`) reciben un `ExecutionContext`. Es la descripción de "lo que se está ejecutando ahora": la petición, el handler elegido, su controlador y el transporte (HTTP o WebSocket). Gracias a él puedes escribir piezas genéricas que funcionan con cualquier ruta.

## `ArgumentsHost` y `ExecutionContext`

En Corvis ambos nombres designan el mismo tipo:

```rust
pub type ArgumentsHost = ExecutionContext;
```

Los filtros de excepción lo reciben como `host: &ArgumentsHost`; guards e interceptors, como `ctx: &ExecutionContext`. Tienen los mismos métodos.

| Método | Devuelve |
|---|---|
| `get_type()` | `ContextType::Http` o `ContextType::Ws` |
| `switch_to_http()` | Vista HTTP: petición, respuesta y estado |
| `switch_to_ws()` | Vista WebSocket: cliente y datos del mensaje |
| `request()` | Atajo de `switch_to_http().get_request()` |
| `get_handler()` | El handler que se va a ejecutar (`&Target`) |
| `get_class()` | El controlador o gateway (`&Target`) |
| `module_ref()` | `Option<&ModuleRef>` del módulo dueño del controlador |

`ContextType` está en la raíz del crate: `use corvis::ContextType;`. También tiene una variante `Rpc` y el método `as_str()` (`"http"`, `"ws"`, `"rpc"`), pero hoy ningún transporte de Corvis produce contextos `Rpc` (ver [Microservicios](/es/microservices/basics)).

### Vista HTTP

```rust
let http = ctx.switch_to_http();
let request: &Request = http.get_request();
let status: u16 = http.status_code();   // estado por defecto de la ruta
let res: Option<Res> = http.get_response();
```

- `get_request()` da la `Request`: método, ruta, cabeceras, query, cuerpo, IP, cookies y las extensiones (`insert` / `get`) para pasar datos entre piezas del pipeline.
- `status_code()` es el estado que tendrá la respuesta si nadie lo cambia: el de `#[http_code]`, `201` en `POST` o `200`.
- `get_response()` devuelve el objeto `Res` solo si el handler declara un parámetro `#[res]`; en los demás casos es `None` (la respuesta se construye a partir del valor de retorno).

### Vista WebSocket

En un gateway, el contexto de cada mensaje es de tipo `Ws`:

```rust
let ws = ctx.switch_to_ws();
let client: Option<Arc<WsClient>> = ws.get_client::<WsClient>();
let data: Option<&Value> = ws.get_data();   // el payload del mensaje, sin validar
```

En ese caso `ctx.request()` es la petición del *handshake* de la conexión (cabeceras, query, IP). El guard `RoomMemberGuard` de `examples/tertulia` lee el payload crudo porque los guards se ejecutan antes de los pipes:

```rust
#[injectable]
pub struct RoomMemberGuard;

#[async_trait]
impl CanActivate for RoomMemberGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let Some(client) = ws.get_client::<WsClient>() else { return Ok(false) };
        let Some(room) = ws.get_data().and_then(|d| d.get("room")).and_then(Value::as_str) else { return Ok(true) };
        if client.in_room(room) {
            return Ok(true);
        }
        Err(WsException::object(json!({
            "status": "error",
            "code": "NOT_IN_ROOM",
            "message": format!("Join #{room} first"),
            "room": room,
        })))
    }
}
```

### Piezas que sirven para los dos transportes

Comprobar `get_type()` permite que un mismo guard funcione en HTTP y en WebSockets. Así lo hace `AuthGuard` en `examples/tertulia/src/auth/auth_guard.rs`:

```rust
async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
    if ctx.get_type() == ContextType::Ws {
        return match ChatUser::of(ctx) {
            Some(_) => Ok(true),
            None => Err(WsException::new("Unauthorized")),
        };
    }
    let request = ctx.switch_to_http().get_request();
    // ... verificar el bearer token
    Ok(true)
}
```

## Handler y clase: `Target`

`get_handler()` y `get_class()` devuelven un `Target`:

```rust
pub struct Target {
    pub name: String,
    pub metadata: HashMap<String, Value>,
}
```

- `name` es el nombre del método (`"find_one"`) o del tipo (`"CatsController"`). Es útil para logs: `format!("{}.{}", ctx.get_class().name, ctx.get_handler().name)`.
- `metadata` contiene lo declarado con `#[set_metadata]`, `#[apply_decorators]` y los atributos `#[api_*]`, como JSON. `target.get_metadata("roles")` devuelve el valor crudo.

## `Reflector`

`Reflector` lee esos metadatos y los deserializa al tipo que pidas. Es un provider global: inyéctalo con un campo `reflector: Arc<Reflector>`.

| Método | Resultado |
|---|---|
| `get::<T>(clave, target)` | El valor de un solo target, o `None` |
| `get_all_and_override::<T>(clave, &[targets])` | El primer valor definido en la lista |
| `get_all_and_merge::<T>(clave, &[targets])` | Un `Vec<T>` que concatena los arrays de todos los targets (los valores sueltos se agregan como un elemento) |
| `get_all::<T>(clave, &[targets])` | Un `Vec<Option<T>>`, uno por target |

```rust
// El handler manda sobre el controlador:
let roles: Option<Vec<String>> =
    self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);

// Los roles del controlador y los del handler, juntos:
let roles: Vec<String> =
    self.reflector.get_all_and_merge(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
```

Si el valor guardado no se puede deserializar en `T`, se trata como ausente.

## Otros usos del contexto

- En un filtro, `host.switch_to_http().get_request().url()` permite incluir la ruta en el cuerpo del error (ver [Filtros de excepción](/es/exception-filters)).
- En un interceptor, `ctx.get_handler().name` identifica la operación para métricas (ver `WsLoggingInterceptor` en `examples/tertulia/src/chat/ws_logging_interceptor.rs`).
- Desde `FromContext`, el contexto es la fuente de cualquier parámetro calculado (ver [Decoradores personalizados](/es/custom-decorators)).
- `ctx.module_ref()` da acceso al contenedor del módulo del controlador para resolver providers bajo demanda (ver [Referencia al módulo](/es/fundamentals/module-ref)).

::: info Diferencias con el modelo de referencia
No existen `getArgs()` ni `getArgByIndex()`: los argumentos del handler se calculan después de los guards e interceptors y no están en el contexto. Para leer datos de la petición usa `request()`.
:::

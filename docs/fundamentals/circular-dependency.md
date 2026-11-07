---
status: parcial
---

# Dependencias circulares

Hay dos tipos de ciclo y Corvis los trata de forma distinta: entre **módulos** (A importa B y B importa A) y entre **providers** (A necesita B para construirse y B necesita A).

## Ciclos entre módulos: permitidos

Los imports circulares entre módulos estáticos funcionan sin hacer nada especial. Al recorrer el grafo, un módulo que ya se está procesando no se vuelve a crear: el segundo import apunta al mismo nodo. La búsqueda de exports también lleva la cuenta de los módulos visitados, así que no entra en bucle.

```rust
#[module(imports = [BillingModule], providers = [OrdersService], exports = [OrdersService])]
pub struct OrdersModule;

#[module(imports = [OrdersModule], providers = [InvoicesService], exports = [InvoicesService])]
pub struct BillingModule;
```

No hace falta ninguna referencia diferida para los módulos. Que el grafo lo admita no significa que sea buen diseño: un ciclo entre módulos suele indicar que hay una tercera responsabilidad que merece su propio módulo.

## Ciclos entre providers: se detectan y fallan

Si `OrdersService` tiene un campo `Arc<InvoicesService>` e `InvoicesService` un campo `Arc<OrdersService>`, ninguno se puede construir antes que el otro. Corvis lo detecta al arrancar y devuelve un error con la cadena completa:

```text
A circular dependency has been detected: OrdersService -> InvoicesService -> OrdersService.
```

No existe un mecanismo de referencia diferida en los campos (`forwardRef` en otros contenedores). El mensaje de error menciona `forwardRef()` por herencia de esos contenedores, pero en Corvis no hay tal función: usa una de las soluciones siguientes.

### 1. Extraer la parte compartida

La solución preferida. Casi siempre, cada servicio necesita solo una parte del otro. Mueve esa parte a un tercer provider del que dependan ambos:

```text
OrdersService ──► PricingService ◄── InvoicesService
```

### 2. Resolver uno de los lados más tarde con `ModuleRef`

Un lado del ciclo deja de pedir el otro como campo y lo busca cuando lo usa. Como todos los singletons se crean al arrancar, en tiempo de petición la instancia ya existe:

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
    orders: Arc<OrdersService>,      // este lado sigue siendo un campo normal
}
```

`ModuleRef::get` busca primero con las reglas de visibilidad del módulo y, si no lo encuentra, en toda la aplicación. Para respetar estrictamente la encapsulación usa `get_token_in_context`. Ver [Referencia al módulo](/fundamentals/module-ref).

No llames a `get` dentro del constructor (por ejemplo en un `#[init(...)]`): durante la construcción el otro lado todavía no existe.

### 3. Comunicar con eventos

Si un servicio solo necesita *avisar* al otro, emite un evento con `EventEmitter` y deja que el otro lo escuche con `#[on_event]`. Tertulia lo hace así: `RoomsService` emite `room.created` y `ChatGateway` lo escucha, sin que `RoomsService` dependa del gateway:

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

Ver [Eventos](/application/events).

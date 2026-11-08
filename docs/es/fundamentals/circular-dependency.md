---
status: partial
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

### Los módulos dinámicos son la excepción

La regla anterior cubre los módulos estáticos. Cada import dinámico (`BModule::for_root()`) crea una instancia de módulo **nueva**, así que dos módulos dinámicos que se importan mutuamente crearían instancias sin fin. Corvis lo detecta al construir el grafo de módulos y falla al arrancar:

```text
A circular import between dynamic modules has been detected: AModule -> BModule -> AModule. Each dynamic import (for example for_root()) creates a new module instance, so this cycle never ends. Move what both modules need into a separate module that both import, or remove one of the two imports.
```

Para arreglarlo, mueve lo que ambos módulos necesitan a un tercer módulo que los dos importen, o elimina uno de los dos imports. La comprobación salta cuando el mismo módulo importa dinámicamente el mismo módulo por segunda vez en una cadena de imports, así que un grafo que repite un import así se rechaza aunque terminara por sí solo. Ver [Módulos dinámicos](/es/fundamentals/dynamic-modules#cada-llamada-es-una-instancia-nueva).

## Ciclos entre providers: se detectan y fallan

Si `OrdersService` tiene un campo `Arc<InvoicesService>` e `InvoicesService` un campo `Arc<OrdersService>`, ninguno se puede construir antes que el otro. Corvis lo detecta al arrancar y devuelve un error con la cadena completa:

```text
A circular dependency has been detected: OrdersService -> InvoicesService -> OrdersService. Break the cycle: move the shared logic into a third
provider, resolve one side lazily with ModuleRef (not in its constructor), or communicate through
EventEmitter. See "Circular dependencies" in the docs.
```

El mensaje está en inglés, como el resto de los errores del framework: propone las tres soluciones de abajo.

No existe un mecanismo de referencia diferida en los campos (`forwardRef` en otros contenedores): usa una de las soluciones siguientes.

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

`ModuleRef::get` busca primero con las reglas de visibilidad del módulo y, si no lo encuentra, en toda la aplicación. Para respetar estrictamente la encapsulación usa `get_token_in_context`. Ver [Referencia al módulo](/es/fundamentals/module-ref).

No llames a `get` dentro del constructor (por ejemplo en un `#[init(...)]`): durante la construcción el otro lado todavía no existe. Llamar a `resolve` o `create` para el otro lado dentro de un constructor o factory se detecta: el arranque falla con el error de dependencia circular y la cadena completa (`OrdersService -> InvoicesService -> OrdersService`), igual que un ciclo entre campos. El trabajo que lances con `tokio::spawn` dentro de un constructor no queda cubierto por esa comprobación.

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

Ver [Eventos](/es/application/events).

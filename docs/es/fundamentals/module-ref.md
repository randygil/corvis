---
status: documented
---

# Referencia al módulo

`ModuleRef` da acceso programático al contenedor de inyección desde dentro de la aplicación. Sirve cuando no sabes qué provider necesitas hasta el momento de usarlo, para resolver providers transient bajo demanda, para crear instancias de tipos que no están registrados y para romper [dependencias circulares](/es/fundamentals/circular-dependency).

## Obtener un `ModuleRef`

Declara un campo de tipo `ModuleRef` en cualquier `#[injectable]`; no lleva atributo:

```rust
#[injectable]
pub struct ExportService {
    module_ref: ModuleRef,
}
```

El `ModuleRef` queda situado en el **módulo dueño del provider**: las búsquedas que respetan la encapsulación parten de ahí.

Otras formas de obtenerlo:

- En una fábrica: `ctx.module_ref()` (ver [Providers personalizados](/es/fundamentals/custom-providers#fabricas-use-factory)).
- Desde la aplicación: `app.module_ref()`, situado en el módulo raíz. `app.get::<T>()` y `app.get_token::<T>(..)` son atajos de `app.module_ref().get...`.

## Métodos

| Método | Devuelve | Busca |
|---|---|---|
| `get::<T>()` | `Result<Arc<T>, DiError>` | Una instancia **existente** de `T`: primero con la visibilidad del módulo y, si no la encuentra, en toda la aplicación. |
| `get_token::<T>(token)` | `Result<Arc<T>, DiError>` | Igual, por token (`"TOKEN"` o `Token`). |
| `get_token_in_context::<T>(token)` | `Result<Arc<T>, DiError>` | Solo lo visible desde el módulo: sus providers, los exports de sus imports y los módulos globales. Es lo mismo que vería un campo `Arc<T>`. |
| `get_strict::<T>()` | `Result<Arc<T>, DiError>` | Solo los providers registrados en el propio módulo. |
| `resolve::<T>().await` | `Result<Arc<T>, DiError>` | Resuelve como la inyección; para un provider transient crea una instancia nueva. Dentro de un constructor o factory, un ciclo de vuelta al provider que se está construyendo falla con el error de dependencia circular. |
| `create::<T>().await` | `Result<T, DiError>` | Construye un `#[injectable]` que **no** está registrado, inyectándole sus campos desde el módulo. Devuelve el valor, no un `Arc`. |
| `module_name()` | `String` | El nombre del módulo. |

Los métodos `get*` son síncronos y no construyen nada: si la instancia no existe (por ejemplo, un provider transient, o un token que nadie registró), devuelven `DiError::NotFound`. Si el token existe pero con otro tipo, `DiError::TypeMismatch`.

::: warning `get` no respeta la encapsulación
Igual que la opción no estricta de otros contenedores, `get` termina buscando en **toda** la aplicación, incluidos providers privados de otros módulos. Si quieres el comportamiento de la inyección por campo, usa `get_token_in_context`; si quieres limitarte al propio módulo, `get_strict`.
:::

## Ejemplos

Elegir un provider por nombre en tiempo de ejecución:

```rust
#[injectable]
pub struct ExportService {
    module_ref: ModuleRef,
}

impl ExportService {
    pub fn exporter(&self, format: &str) -> HttpResult<Arc<dyn Exporter>> {
        self.module_ref
            .get_token_in_context::<dyn Exporter>(format!("EXPORTER:{format}"))
            .map_err(|_| BadRequestException::new(format!("Unknown format {format}")))
    }
}

// providers = [
//     Provider::use_class_as::<CsvExporter, dyn Exporter>("EXPORTER:csv", |e| e),
//     Provider::use_class_as::<JsonExporter, dyn Exporter>("EXPORTER:json", |e| e),
//     ExportService,
// ]
```

Una instancia nueva de un provider transient en cada operación:

```rust
pub async fn run(&self) -> corvis::Result<()> {
    let timer = self.module_ref.resolve::<OperationTimer>().await?;
    // ...
    Ok(())
}
```

Construir un tipo no registrado con sus dependencias inyectadas:

```rust
#[injectable]
pub struct ReportBuilder {
    rooms: Arc<RoomsService>,
    #[default]
    lines: Vec<String>,
}

let builder: ReportBuilder = self.module_ref.create::<ReportBuilder>().await?;
```

Cada llamada a `create` produce un valor nuevo; el contenedor no lo guarda.

## En los tests

`TestingModule` y la aplicación de pruebas exponen `get` de la misma forma, así que puedes obtener un provider (o su doble) para hacer aserciones. Ver [Testing](/es/fundamentals/testing).

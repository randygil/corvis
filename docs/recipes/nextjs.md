---
status: no-disponible
---

# Next.js

No hay una integración de Corvis con Next.js: Corvis no puede ejecutar el servidor de Next.js ni su renderizado del lado del servidor, que necesitan Node.js.

## Alternativas

**Exportación estática.** Si tu aplicación de Next.js puede exportarse como archivos estáticos, Corvis la sirve con `ServeStaticModule`, incluido el *fallback* de una aplicación de una sola página:

```rust
#[module(
    imports = [
        ServeStaticModule::for_root(vec![
            ServeStaticOptions::new("./web/out")
                .exclude(&["/api/(.*)"])
                .spa_fallback(true),
        ]),
    ],
    controllers = [/* tu API */],
)]
pub struct AppModule;
```

Con `app.set_global_prefix("api")`, la API y el frontend conviven en el mismo origen. Ver [Archivos estáticos](/recipes/serve-static).

**Dos servicios.** Si necesitas renderizado en el servidor, ejecuta Next.js y la API de Corvis como procesos separados y únelos detrás de un proxy inverso, o habilita [CORS](/security/cors) en la API.

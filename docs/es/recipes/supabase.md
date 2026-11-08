---
status: unavailable
---

# Supabase

Corvis **no tiene una integración específica con Supabase**: no hay un cliente para su API, su autenticación, su almacenamiento ni sus canales en tiempo real.

## Lo que sí funciona

**La base de datos.** La base de datos de un proyecto de Supabase es PostgreSQL, y `corvis-orm` se conecta a PostgreSQL con TLS (feature `tls-rustls`, activa por defecto). Usa la cadena de conexión del proyecto como `DATABASE_URL`:

```rust
use corvis::prelude::*;
use corvis_orm::OrmModule;

#[module(
    imports = [
        ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() }),
        OrmModule::for_root_from_config(true),
    ],
)]
pub struct AppModule;
```

En producción usa `sslmode=verify-full` en la URL. Ver [corvis-orm](/es/data/corvis-orm).

**Los tokens de inicio de sesión.** Son JWT. Si tu proyecto los firma con un secreto compartido, `JwtService` puede verificarlos configurando `JwtModule` con ese secreto, y un [guard](/es/guards) protege las rutas. Si usa claves asimétricas, configura `JwtModuleOptions` con `.algorithm(..)` y `.public_key(pem)`. Comprueba en tu proyecto qué algoritmo y qué claves usa antes de confiar en la verificación.

El resto de servicios (almacenamiento, funciones, tiempo real) se consumen con su API HTTP; Corvis no trae un cliente HTTP propio (ver [Cliente HTTP](/es/application/http-client)).

---
status: documentado
---

# Conexiones keep-alive

Con HTTP/1.1, un cliente puede reutilizar la misma conexión TCP para varias peticiones
(*keep-alive*). Corvis lo soporta siempre y cierra las conexiones inactivas después de
`keep_alive_timeout`, que por defecto es de 5 segundos, el mismo valor que Node.js.

## Cambiar el timeout

```rust
use std::time::Duration;
use corvis::ServerOptions;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.set_server_options(ServerOptions::new().keep_alive_timeout(Duration::from_secs(65)));
app.listen(3000).await
```

`.keep_alive_timeout(None)` desactiva el límite.

## Detrás de un balanceador

Si un balanceador o proxy reutiliza conexiones hacia la aplicación, el timeout de inactividad de
Corvis debe ser **mayor** que el del balanceador. Si no, Corvis puede cerrar una conexión justo
cuando el balanceador envía una petición por ella, y el cliente recibe un `502` esporádico. Por
ejemplo, con un balanceador que cierra a los 60 segundos, usa 65.

## Qué cuenta como inactividad

Solo el tiempo entre peticiones de una misma conexión. Una petición en curso (un handler lento o una
respuesta [Server-Sent Events](/http/server-sent-events)) no cuenta como inactividad.

Los demás timeouts del servidor son independientes:

| Opción | Por defecto | Qué limita |
|---|---|---|
| `headers_timeout` | 60 s | Tiempo para recibir las cabeceras; protege de clientes que las envían gota a gota |
| `request_timeout` | 300 s | Tiempo para recibir el cuerpo; responde `408` |
| `keep_alive_timeout` | 5 s | Inactividad entre peticiones |

Ver [Adaptadores HTTP](/http/performance) para el resto de `ServerOptions`.

---
status: partial
---

# HTTPS y varios servidores

## HTTPS

Corvis no termina TLS: el servidor solo acepta HTTP/1.1 y HTTP/2 sin cifrar (h2c). No hay opciones
para cargar un certificado y una clave en la aplicación.

La forma recomendada de servir HTTPS es poner un proxy inverso delante (nginx, Caddy, Traefik, un
balanceador de la nube) que termine TLS y reenvíe a la aplicación por HTTP en la red interna. En ese
caso:

- Escucha solo en la interfaz interna o en `127.0.0.1`, por ejemplo
  `app.listen("127.0.0.1:3000").await`. Ojo: `app.listen(3000)` (un número) escucha en `0.0.0.0`.
- Configura `trust proxy` para que `req.ip()`, `req.ips()` y `req.hostname()` lean las cabeceras
  `X-Forwarded-*` del proxy: `app.set_trust_proxy(1)`. Ver [Adaptadores HTTP](/es/http/performance).
- Fija `Strict-Transport-Security` en el proxy o con un middleware. Ver
  [Cabeceras de seguridad](/es/security/helmet).

## Escuchar en varios puertos

`listen` consume la aplicación y bloquea hasta que termina, pero
`listen_in_background(addr)` arranca un servidor en segundo plano y devuelve la dirección en la que
escucha. Puedes combinarlos para atender la misma aplicación en dos direcciones:

```rust
#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.enable_shutdown_hooks();
    let internal = app.listen_in_background("127.0.0.1:9000").await?;
    Logger::new("Bootstrap").log(format!("Internal listener on {internal}"));
    app.listen("0.0.0.0:3000").await
}
```

Ambos servidores comparten rutas, providers y configuración (se inicializa una sola vez). Ten en
cuenta que:

- Las opciones de `set_server_options` se aplican a los dos.
- Con `enable_shutdown_hooks`, al recibir la señal se detiene el servidor de `listen` y se ejecutan los
  hooks de cierre; el de `listen_in_background` no se drena: termina cuando termina el proceso.
- `app.get_url()` devuelve la dirección del último servidor que empezó a escuchar.

`listen_in_background("127.0.0.1:0")` elige un puerto libre, lo que también resulta útil en pruebas
*end to end*.

No hay forma de servir rutas distintas en cada puerto: si necesitas eso, crea dos aplicaciones con
módulos raíz distintos.

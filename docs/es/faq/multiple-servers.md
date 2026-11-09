---
status: documented
---

# HTTPS y varios servidores

## HTTPS

Corvis puede terminar TLS por sí mismo con la feature de cargo `tls` (desactivada por defecto, basada en rustls, sin
OpenSSL):

```toml
corvis = { version = "0.1", features = ["tls"] }
```

```rust
use corvis::tls::TlsOptions;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let app = CorvisFactory::create::<AppModule>().await?;
    // Los errores de los archivos (inexistentes, clave cifrada, clave de otro certificado) salen aquí, antes de arrancar.
    let tls = TlsOptions::from_pem_files("cert.pem", "key.pem")?;
    app.listen_tls("0.0.0.0:3443", tls).await
}
```

`listen_tls(addr, tls)` y `listen_tls_in_background(addr, tls)` funcionan como `listen` y `listen_in_background`: la
aplicación se inicializa (incluida la comprobación de postura de rutas) antes de abrir el puerto, y se sirven las mismas
rutas, middleware y gateways. `ServerOptions` (`max_connections`, timeouts, `max_header_size`, la comprobación de
framing de HTTP/1, `send_timeout`, `max_pending_response_bytes`, los límites de HTTP/2) se aplica a las conexiones HTTPS
igual que a las planas. `app.get_url()` devuelve una URL `https://` cuando el último listener iniciado es TLS, y los
gateways de WebSocket responden en `wss://`.

### Construir las opciones

- `TlsOptions::from_pem_files(cert_chain, key)` y `TlsOptions::from_pem(bytes_de_la_cadena, bytes_de_la_clave)`: una
  cadena de certificados PEM (primero la hoja, luego los intermedios) y exactamente una clave privada (PKCS#8, PKCS#1 o
  SEC1). Las claves cifradas se rechazan con un mensaje que pide descifrarlas antes
  (`openssl pkcs8 -in key.pem -out key-decrypted.pem`). Los errores nombran archivos, nunca material de la clave.
- `TlsOptions::from_server_config(config)`: un `rustls::ServerConfig` ya construido (`corvis::tls::rustls` reexporta la
  versión que usa Corvis). Úsalo para varios certificados según el nombre del servidor
  (`ResolvesServerCertUsingSni`), certificados de cliente, un resolver que recargue archivos o solo TLS 1.3. Corvis
  aplica su endurecimiento a una copia.
- `.handshake_timeout(Duration)` y `.http2(bool)`.

### Valores por defecto

- Solo TLS 1.2 y 1.3, las suites de cifrado por defecto de rustls (AEAD con secreto perfecto hacia adelante), sin
  certificados de cliente, el mismo certificado para cualquier nombre de servidor (o ninguno).
- Los datos tempranos 0-RTT siempre se rechazan, también con una configuración propia: las peticiones repetidas por un
  atacante no son seguras para handlers que no son idempotentes.
- ALPN ofrece `h2` y `http/1.1`. Una conexión que negocia `h2` se sirve solo como HTTP/2; una que negocia `http/1.1` (o
  nada) se sirve solo como HTTP/1.x, con la comprobación de framing de HTTP/1, y el preface de HTTP/2 se rechaza incluso
  con `h2c(true)`. `.http2(false)` ofrece solo `http/1.1`, que es lo que hace un servidor HTTPS de Express.
- El handshake corre en la tarea propia de la conexión, limitado a **10 segundos** (el `https` de Node espera 120): un
  cliente que se detiene en el handshake no puede retener una plaza de conexión mucho tiempo, y las conexiones en
  handshake cuentan para `max_connections`. Un handshake fallido se cierra sin respuesta y no se registra en el log.
- El proveedor criptográfico es el predeterminado del proceso en rustls cuando la aplicación instaló uno (por ejemplo
  `aws-lc-rs`, para FIPS), y `ring` en otro caso.

### Certificados

Los archivos se leen una vez, al construir las opciones: renovar un certificado requiere reiniciar. Un
`ResolvesServerCert` propio pasado con `from_server_config` puede recargar certificados sin reiniciar. Los certificados
de cliente (mTLS) son posibles con `from_server_config`, pero los handlers aún no pueden leer el certificado del par.

### HTTPS en el 443 más un puerto HTTP interno

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_shutdown_hooks();
app.listen_in_background("127.0.0.1:9000").await?; // plano, interno
let tls = TlsOptions::from_pem_files("cert.pem", "key.pem")?;
app.listen_tls("0.0.0.0:443", tls).await
```

### Cuándo sigue siendo mejor un proxy inverso

Pon nginx, Caddy, Traefik o un balanceador de la nube delante cuando necesites certificados automáticos (ACME), recargar
certificados sin reiniciar, HTTP/3, un puerto compartido por varias aplicaciones o un WAF. En ese caso:

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

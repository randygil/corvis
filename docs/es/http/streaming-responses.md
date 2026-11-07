---
status: documented
---

# Respuestas en streaming

Por defecto, lo que devuelve un handler se serializa completo y se envía con `Content-Length`. Para
archivos grandes o datos que se generan poco a poco, Corvis puede enviar el cuerpo por partes.

## `StreamableFile`

`StreamableFile` representa un archivo para descargar. Sus cabeceras (`Content-Type`,
`Content-Disposition`, `Content-Length`) solo se escriben si la respuesta no las tiene ya, así que un
`#[header(...)]` en la ruta tiene prioridad.

```rust
#[routes]
impl ReportsController {
    /// Desde memoria.
    #[get(":id/csv")]
    fn csv(&self, #[param("id")] id: String) -> StreamableFile {
        let bytes = self.reports.render_csv(&id);
        StreamableFile::new(bytes).content_type("text/csv").filename(format!("{id}.csv"))
    }

    /// Desde disco, por partes de hasta 64 KiB.
    #[get(":id/pdf")]
    async fn pdf(&self, #[param("id", ParseUUIDPipe)] id: String) -> HttpResult<StreamableFile> {
        let file = StreamableFile::from_path(format!("./reports/{id}.pdf"))
            .await
            .map_err(|_| NotFoundException::new("Report not found"))?;
        Ok(file.content_type("application/pdf"))
    }
}
```

Formas de construirlo:

| Constructor | Origen de los datos |
|---|---|
| `StreamableFile::new(bytes)` | Datos en memoria (`Vec<u8>`, `Bytes`, `String`...) |
| `StreamableFile::with_options(bytes, StreamableFileOptions::new()...)` | Igual, con opciones |
| `StreamableFile::from_stream(stream, options)` | Un `ByteStream` |
| `StreamableFile::from_path(ruta).await?` | Un archivo, leído por partes; el tipo se deduce de la extensión y la longitud del tamaño |

Y sus opciones, como métodos de `StreamableFile` o de `StreamableFileOptions`:

- `.content_type(..)` / `StreamableFileOptions::r#type(..)`: `Content-Type`
  (`application/octet-stream` por defecto).
- `.disposition(..)`: `Content-Disposition` literal.
- `.filename(nombre)`: arma `Content-Disposition: attachment` como `res.attachment()` de Express
  (usa solo el nombre base, escapa comillas y agrega `filename*` si el nombre no es latin1).
- `.length(n)`: `Content-Length` para un stream; sin él, el stream se envía con
  `Transfer-Encoding: chunked`.

## Un stream propio

`Response::stream` envía cualquier `ByteStream`, que es un
`Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>`:

```rust
use corvis::bytes::Bytes;
use corvis::futures::stream;

#[get("numbers")]
fn numbers(&self) -> Response {
    let chunks = stream::iter((1..=1000).map(|n| Ok(Bytes::from(format!("{n}\n")))));
    Response::stream(Box::pin(chunks), "text/plain; charset=utf-8")
}
```

Los cuerpos en stream se escriben tal cual: sin `ETag` y con el `Content-Type` exacto que indiques.

## Escribir la respuesta a mano con `#[res]`

Con `#[res] res: Res` el handler es dueño de la respuesta y puede terminarla más tarde, desde otra
tarea. Es útil cuando el resultado llega de un callback:

```rust
#[get("slow")]
async fn slow(&self, #[res] res: Res) {
    corvis::tokio::spawn(async move {
        corvis::tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        res.json(json!({ "done": true }));
    });
}
```

La petición queda abierta hasta que se llama a `send`, `json`, `end`, `redirect` o `send_status`.
`Res` no ofrece escritura incremental (como `res.write` en Node.js): para enviar por partes devuelve
un stream.

## Server-Sent Events

Para enviar eventos al navegador por una conexión abierta, usa `#[sse]`. Ver
[Server-Sent Events](/es/http/server-sent-events).

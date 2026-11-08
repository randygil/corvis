---
status: documented
---

# Streaming responses

By default, what a handler returns is serialized in full and sent with `Content-Length`. For
large files or data generated gradually, Corvis can send the body in chunks.

## `StreamableFile`

`StreamableFile` represents a file to download. Its headers (`Content-Type`,
`Content-Disposition`, `Content-Length`) are only written if the response does not already have them, so a
`#[header(...)]` on the route takes precedence.

```rust
#[routes]
impl ReportsController {
    /// From memory.
    #[get(":id/csv")]
    fn csv(&self, #[param("id")] id: String) -> StreamableFile {
        let bytes = self.reports.render_csv(&id);
        StreamableFile::new(bytes).content_type("text/csv").filename(format!("{id}.csv"))
    }

    /// From disk, in chunks of up to 64 KiB.
    #[get(":id/pdf")]
    async fn pdf(&self, #[param("id", ParseUUIDPipe)] id: String) -> HttpResult<StreamableFile> {
        let file = StreamableFile::from_path(format!("./reports/{id}.pdf"))
            .await
            .map_err(|_| NotFoundException::new("Report not found"))?;
        Ok(file.content_type("application/pdf"))
    }
}
```

Ways to build it:

| Constructor | Data source |
|---|---|
| `StreamableFile::new(bytes)` | In-memory data (`Vec<u8>`, `Bytes`, `String`...) |
| `StreamableFile::with_options(bytes, StreamableFileOptions::new()...)` | Same, with options |
| `StreamableFile::from_stream(stream, options)` | A `ByteStream` |
| `StreamableFile::from_path(path).await?` | A file, read in chunks; the type is deduced from the extension and the length from the size |

And its options, as methods of `StreamableFile` or `StreamableFileOptions`:

- `.content_type(..)` / `StreamableFileOptions::r#type(..)`: `Content-Type`
  (`application/octet-stream` by default).
- `.disposition(..)`: literal `Content-Disposition`.
- `.filename(name)`: builds `Content-Disposition: attachment` like Express's `res.attachment()`
  (uses only the base name, escapes quotes and adds `filename*` if the name is not latin1).
- `.length(n)`: `Content-Length` for a stream; without it, the stream is sent with
  `Transfer-Encoding: chunked`.

## A custom stream

`Response::stream` sends any `ByteStream`, which is a
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

Streamed bodies are written as is: without `ETag` and with the exact `Content-Type` you specify.

## Writing the response by hand with `#[res]`

With `#[res] res: Res` the handler owns the response and can finish it later, from another
task. It is useful when the result arrives from a callback:

```rust
#[get("slow")]
async fn slow(&self, #[res] res: Res) {
    corvis::tokio::spawn(async move {
        corvis::tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        res.json(json!({ "done": true }));
    });
}
```

The request stays open until `send`, `json`, `end`, `redirect` or `send_status` is called.
`Res` does not offer incremental writes (like `res.write` in Node.js): to send in chunks, return
a stream.

## Server-Sent Events

To send events to the browser over an open connection, use `#[sse]`. See
[Server-Sent Events](/http/server-sent-events).

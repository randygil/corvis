---
status: documented
---

# Raw body

Some cases need the exact bytes the client sent, not the already parsed JSON: the typical example
is verifying a webhook's HMAC signature. In Corvis you do not have to enable anything: every `Request`
keeps its original body, and `req.raw_body()` returns it as `Bytes`.

```rust
#[post("payments")]
#[http_code(200)]
fn payments(&self, #[req] req: Request, #[body] event: Value) -> HttpResult<Value> {
    let raw: &[u8] = req.raw_body();
    // `verifier` is a provider of yours that checks the signature against the original bytes.
    self.verifier.check(req.header("x-signature"), raw)?;
    Ok(json!({ "received": event["id"] }))
}
```

`#[body]` and `req.raw_body()` can be used at the same time: the first receives the parsed value and the second
the original bytes. It is also available in guards, interceptors and middleware through the
request (`ctx.request().raw_body()`).

See [Webhooks](/http/webhooks) for a complete signature verification guard.

## What is parsed and what is not

| `Content-Type` | `#[body]` / `req.body()` | `req.raw_body()` |
|---|---|---|
| `application/json` | Parsed JSON (strict: only objects or arrays at the top level) | Original bytes |
| `application/x-www-form-urlencoded` | Object, with `qs` syntax (`a[b]=1`) | Original bytes |
| Any other (`text/plain`, `application/xml`...) | `null` | Original bytes |
| `multipart/*` | Text fields, after a file interceptor | Empty (see below) |

Limits:

- JSON and urlencoded: 100 kb each by default, with `413 request entity too large` when exceeded.
  They are changed with `app.use_body_parser("json", bytes)` and `app.use_body_parser("urlencoded", bytes)`.
- Other types are read up to the transport limit, 1 MiB by default (`app.set_body_limit(bytes)`).

Parser details:

- `application/json` and `application/x-www-form-urlencoded` are parsed with or without parameters such as `charset`. A request with no body leaves `#[body]` as `null`.
- JSON is strict (only `{` or `[` at the top level), an empty body is `{}`, a BOM is ignored, and syntax errors use the same messages as V8's `JSON.parse` (`Expected property name or '}' in JSON at position 1 (line 1 column 2)`, `Unexpected token '"', ""x"" is not valid JSON`...). The response is `400 { "message": ..., "error": "Bad Request", "statusCode": 400 }`.
- Unlike Express, JSON nesting is limited to 256 levels (`400 Maximum nesting depth of 256 exceeded in JSON at position 256 (line 1 column 257)`). The recursive `serde_json::Value` would overflow the stack with a few kB of `[[[...]]]`. `app.set_body_parser_max_depth(n)` changes it, and it also caps the depth of urlencoded bodies. `corvis::http::json_parse::parse` applies the same limit; use `parse_utf16_with_depth` for another value.
- urlencoded uses `qs` with the `body-parser` options (nested objects and arrays). More than 32 levels responds `400 The input exceeded the depth` and more than 1000 parameters `413 too many parameters`. `%XX` escapes need two hexadecimal digits (`%+A` is not valid).
- The 100 kb limit responds `413 { "statusCode": 413, "message": "request entity too large" }`.
- The body is read only up to the limit of the parser that will process it: a JSON or urlencoded body that announces a larger `Content-Length` gets `413` without being read. Other types are read up to the transport limit so that `req.raw_body()` stays available.
- A body that cannot be read (the client disconnects, malformed *chunked* encoding) responds `400 request aborted`, and one that does not arrive in time, `408 Request Timeout`.
- An unsupported charset responds `415 unsupported charset "LATIN1"`. `Content-Type` and `Content-Encoding` are read as latin1, like Node.
- Parser errors go through CORS and the global exception filters, like a `next(err)` in Express.

`app.disable_body_parser()` turns off JSON and urlencoded parsing: `#[body]` receives `null` and
`req.raw_body()` still has the bytes. Useful if you want to parse yourself with `serde_json`.

## Multipart bodies

`multipart/*` bodies are not read by the server: they are read, in *streaming*, by the route's file
interceptor. In those requests `req.raw_body()` is empty. If you want to process the multipart on your
own, read it with `corvis::multer::read_body(&req, limit).await?`. See
[File upload](/http/file-upload).

## Compressed bodies

A body with a `Content-Encoding` other than `identity` is rejected with
`415 unsupported content encoding` (Express decompresses them). See [Compression](/http/compression).

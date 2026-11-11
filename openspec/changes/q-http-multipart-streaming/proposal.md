# Proposal

Part of epic [#187](https://github.com/randygil/corvis/issues/187) (code quality audit); implements the issue filed from
`quality/q-http-multipart-streaming.issue.md`. Source: the engineering-quality review of the HTTP layer, the server and
the multipart module, read against `dev` at `d46758a`. Fourth of four changes (see `q-http-server-robustness/proposal.md`);
it relies on the boxed pending body of `q-http-server-robustness` and adds its scenarios to the benchmark of
`q-http-request-hot-path`. The `StorageEngine` trait is public API and this change redesigns it now, before 1.0.

## Why

- **Every uploaded file is held whole in memory, and the public trait fixes that.** `StorageEngine::handle_file`
  receives the file as one `Bytes` (`crates/corvis/src/multer/storage.rs:19-23`), so the parser must keep every chunk of a
  file until its end, and `DiskStorage` writes it in one call (`storage.rs:132`). The docs admit it ("Corvis keeps each
  file in memory until the storage engine receives it", `multer/mod.rs:189-192`). Memory per upload is the multipart body
  limit, which grows with `file_size * files + 1 MiB`; concurrent uploads multiply it. In the original library a storage
  engine's `_handleFile` receives the file as a stream (`file.stream`), and its disk storage pipes it to the file. A trait published with `Bytes` cannot be changed later without breaking every custom engine.
- **The parser copies every byte twice.** `StreamParser::push` copies the whole chunk behind the previous tail on almost
  every push (`multer/busboy.rs:177-185`), and `FileEnd` concatenates the chunks of a file into a new buffer
  (`busboy.rs:410-421`), so the peak is twice the file. The boundary finder is rebuilt on every push (`busboy.rs:186`).
- **File buffers live in a process-wide registry.** `buffer_to_value` stores every `file.buffer` of every request in one
  `static Mutex<HashMap<String, Bytes>>` (`multer/buffer.rs:19-37,55-56`), resolved by a string handle from any request
  (`buffer.rs:59-66`): one lock shared by all uploads, and a handle that is not scoped to the request that created it.
- **Cleanup errors vanish.** `remove_file` failures are discarded with `let _ =` (`multer/interceptors.rs:74,333`): a
  rejected upload that could not be deleted leaves a file on disk and no trace.

## What Changes

- **`StorageEngine` receives a stream** (breaking, on purpose before 1.0): `handle_file(&self, req, file, stream:
  FileStream)` where `FileStream` is a boxed `Stream<Item = io::Result<Bytes>> + Send`. `DiskStorage` writes chunk by
  chunk and removes the partial file on any failure; `MemoryStorage` collects the stream. A helper,
  `multer::collect_file(stream)`, gives a custom engine the bytes when it needs them whole. `remove_file` is unchanged.
- **Backpressure, no spawn.** The file interceptor hands each chunk to the engine through a channel of one chunk and polls
  the engine's future in every wait of its read loop, in the request's own task: a slow disk slows the read of the socket
  instead of filling memory. A limit, a client abort or a timeout in the middle of a file ends the stream with an error;
  the engine cleans up and the interceptor answers as today.
- **The parser streams.** File data leaves the parser as zero-copy slices of the body (`FileData` events) instead of being
  collected; a boundary split across two chunks is found by copying at most the tail and the first boundary-length bytes of
  the next chunk; the finder is built once.
- **File buffers live with their request.** `buffer_to_value` stores into a per-request store kept in the request
  extensions and freed with the request; `buffer_from_value` resolves handles only against the store of the request being
  processed (a task-local scope set by the router around the route pipeline). No global lock, no cross-request handle.
- **Cleanup failures are logged** (rate-limited warning with the path), and an engine that returns `Ok` for a file whose
  stream ended in error gets `remove_file` called on it.
- **Limits unchanged.** The multipart body limit and its defaults stay; the rustdoc explains it as a bound on the body
  read, and memory storage as the only engine that holds files.

## Capabilities

### New Capabilities

- `http/multipart-storage`: how file interceptors hand files to storage engines, the memory an upload holds, cleanup on
  failure and the scope of file buffers.

### Modified Capabilities

- `bench/http-hot-path`: the scenario list gains two upload scenarios.

## Impact

- Edited: `crates/corvis/src/multer/{storage,busboy,interceptors,buffer,mod,file,fuzz}.rs`,
  `crates/corvis/src/router/mod.rs` (the buffer scope around the pipeline), `crates/corvis/src/common/parse_file.rs`
  (resolution through the scope), `bench/http/src/main.rs` and `bench/http/run.sh` (scenarios), tests of `tests/multer.rs`,
  `tests/transport_security.rs` and a new `tests/multer_memory.rs`, and in `docs/` and `docs/es/`: `http/file-upload.md`,
  `application/file-storage.md`.
- **Public API, breaking:** `StorageEngine::handle_file` takes a `FileStream` instead of `Bytes`. Additions: `FileStream`,
  `multer::collect_file`. `buffer_from_value` resolves a handle only while the request that created it is being
  processed (inside its guards, interceptors, pipes, handler and filters).
- Behavior visible to applications and clients: design.md "Behavior visible to applications". The ones that matter: a
  custom storage engine must consume a stream; disk uploads no longer hold the file in memory; a partial file is removed
  when an upload fails mid-way.
- The port apps: consi uses `disk_storage` (no code change; its uploads are compared by the parity check); the others do
  not upload.
- No new dependency (`futures` and `tokio` channels already used).

## Out of scope

- Raising the default multipart body limit now that disk uploads stream: a separate decision (design.md Open question 2).
- A streaming `read_body` for handlers that parse multipart themselves: it keeps returning the whole body by contract.

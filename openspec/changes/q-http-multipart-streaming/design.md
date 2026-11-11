# Design

## Context

Source: the engineering-quality review of the HTTP layer, the server and the multipart module (issue filed from
`quality/q-http-multipart-streaming.issue.md`, epic #187). Code read on `dev` at `d46758a` (2026-11-11). The review was
made by reading; the memory claims are confirmed by reading what is kept and copied, and each decision names the number a
counting-allocator test and the benchmark must show.

### Findings as checked

| # | Finding | Where now | Verdict |
|---|---|---|---|
| 2a | Files held whole in memory; the public trait fixes it | `storage.rs:19-23` (`handle_file(&self, req, file, data: Bytes)`), `storage.rs:117-138` (`DiskStorage` writes with one `tokio::fs::write`), `interceptors.rs:222-241` (the engine is called at `FileEnd`), `mod.rs:186-195` (the rustdoc of `body_limit` says files are kept in memory) | Confirmed. The parser keeps every slice of an included file in `file.chunks` until the closing boundary (`busboy.rs:386-389`) |
| 2b | The parser copies each byte twice | `busboy.rs:177-185` (`push` copies tail + whole chunk into a new `BytesMut` whenever the tail is not empty, which is after every push that did not end on a boundary, since the last `needle.len() - 1` bytes are always kept, `busboy.rs:194-197`), `busboy.rs:410-421` (`FileEnd` concatenates the chunks: peak twice the file), `busboy.rs:186` (`Finder::new` per push) | Confirmed. The slices kept in `file.chunks` are zero-copy slices of the concatenated buffer, so the first copy is per push and the second per file |
| 15 | Global buffer registry with string handles | `buffer.rs:19-20` (`static REGISTRY: LazyLock<Mutex<HashMap<String, Bytes>>>`), `buffer.rs:29-37` (`Lease::drop` locks it), `buffer.rs:55-56` (insert), `buffer.rs:59-66` (`buffer_from_value` resolves any handle, from any request) | Confirmed: one lock for all uploads of the process, and handles not scoped to their request. Narrowed: guessing another request's handle needs 64 random bits (`buffer.rs:54`), so this is not an exploitable leak; and the `unwrap()` in `Lease::drop` can only panic if a panic poisoned the lock, which only `HashMap` operations hold, so the double-panic abort is theoretical. Both still go with the registry |
| — | `remove_file` errors discarded | `interceptors.rs:72-76` (`Cleanup::run`), `interceptors.rs:331-334` (after a failed read) | Confirmed |
| — | `buffer_from_value` callers | `multer/file.rs:149` (`UploadedFile` deserialization), `common/parse_file.rs:192` (`ParseFilePipe`) | Both run inside the route pipeline (handler arguments and pipes), which Decision 4 relies on |

## Goals / Non-Goals

**Goals:**

- A `StorageEngine` contract that can be published: files arrive as a stream, with backpressure.
- A disk upload holds a bounded amount of memory whatever the file size; a memory upload holds the file once.
- File buffers scoped to their request, without a process-wide lock.
- Every security bound of the multipart path kept, each pinned by a test.

**Non-Goals:**

- Changing the multipart limits or their defaults.
- A streaming variant of `read_body`.

## Decisions

### 1. `StorageEngine` receives a stream

```rust
pub type FileStream = Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send + 'static>>;

#[async_trait]
pub trait StorageEngine: Send + Sync + 'static {
    async fn handle_file(&self, req: &Request, file: &FileInfo, stream: FileStream)
        -> Result<Map<String, Value>, HttpException>;
    async fn remove_file(&self, req: &Request, file: &mut Map<String, Value>) -> Result<(), HttpException>;
}
```

- The item error is `std::io::Error`, so an engine can wrap the stream with `tokio_util::io::StreamReader` or any
  `AsyncRead` adapter. An error item ends the stream: the client aborted, the request timed out, the file passed
  `limits.file_size`, or the body passed the multipart body limit. The engine must stop, undo what it wrote and return
  `Err`; the interceptor answers with its own error (the multer error, `408`, `413`, `400`), not the engine's.
- `multer::collect_file(stream) -> io::Result<Bytes>` collects a stream for engines that need the whole file; the size is
  already bounded by the interceptor's limits.
- `MemoryStorage` uses `collect_file` (one copy into one buffer, no second concatenation) and returns `buffer` and
  `size` as today.
- `DiskStorage` resolves the destination and the name and checks the name exactly as today (`check_file_name`, before
  anything is created), opens the file with `File::create`, writes each chunk, flushes, and returns the same keys with
  `size` counted from the chunks written. On an error item, a write error or a flush error it removes the file (a failure
  to remove is logged, Decision 5) and returns `Err`.
- Rustdoc of the trait: the stream contract, that `file.originalname` is the client's (the existing warning), that the
  engine runs in the request's task and must not block it, and an example of a custom engine that streams to an
  `AsyncWrite`.

Alternatives considered: `AsyncRead` instead of a stream (rejected: chunks are already `Bytes` slices, a reader would copy
them into the engine's buffer, and `StreamReader` gives a reader to engines that want one); a channel receiver in the
signature (rejected: exposes the transport; a boxed stream lets tests and wrappers build streams from anything); keeping
`Bytes` and adding a second, streaming trait (rejected: two contracts to publish and maintain, and the in-memory one keeps
the problem).

### 2. Backpressure in the request's task

The read loop of `interceptors.rs` (`read_events`) gains the file in progress: `current: Option<FileInProgress>` with the
channel's sender (`tokio::sync::mpsc::channel(1)`) and the engine's future, pinned in the loop's own frame (it borrows the
request and the `FileInfo`, so it is not spawned).

- `FileStart` of an included file: create the channel, call `handle_file` with the receiver as a `FileStream`, keep the
  future.
- `FileData { id, data }` of the current file: `send(Ok(data))`, polled together with the engine's future
  (`tokio::select!`), so a full channel waits for the engine to take the chunk while the engine keeps making progress.
- `FileEnd`: drop the sender and await the engine's future; its result goes to the appender as today.
- Every other wait of the loop (the next chunk of the body) also polls the engine's future, so an engine that does other
  work (a network upload) progresses while the client sends.
- The engine finishing early: with `Err`, the upload fails with that error as today; with `Ok` before `FileEnd`, the rest
  of the part is discarded (`parser.discard_file`) and the result stands (documented: an engine that stops reading
  accepts what it read).
- A limit, a client abort, a timeout or a parse error while a file is in progress: send one error item, drop the sender,
  await the engine's future (bounded: the engine has nothing more to read), then fail with the interceptor's error. If the
  engine nevertheless returned `Ok`, the interceptor calls `remove_file` on what it returned.
- Memory held between the parser and the engine: the chunk in the channel and the one being sent. Chunks are slices of
  the body frames hyper delivers.

Alternative considered: spawning the engine on its own task (rejected: the engine receives `&Request`, a spawn would need
`'static` data and a copy of the request, and a cancelled request would leave the task running).

### 3. The parser streams and copies only boundary candidates

- `Event::FileData { id, data: Bytes }` is emitted for each slice of an included, not discarded file, as it is parsed;
  `Event::FileEnd { id }` carries no data. `file.chunks` goes. Discarded files (filtered out, or past the file limit) emit
  no data, as today they keep none.
- `push(chunk)`: when the tail is empty, search the chunk directly. When it is not, copy the tail plus the first
  `needle.len() - 1` bytes of the chunk into a small reusable buffer and search there for a boundary that starts in the
  tail; emit the part of the tail that cannot start a boundary from that buffer, then continue in the chunk itself without
  copying it. At most `2 * (needle.len() - 1)` bytes are copied per push.
- The `memmem::Finder` is built in `StreamParser::new` (`Finder::new(&needle).into_owned()`).
- Header parsing, limits, `FileLimit` (inclusive, as busboy), `discard_file`, the epilogue rule and the events' order are
  unchanged.

Tests: `streaming_is_split_invariant` (the same events for any split of the body, now comparing the concatenation of the
`FileData` of each file), `discarded_file_keeps_no_data` (no `FileData` for a discarded file), `file_size_limit_is_inclusive_like_busboy`,
`parses_fields_and_files`, and the fuzz properties of `multer/fuzz.rs` adapted to `FileData`: `busboy_parse_is_total_and_bounded`
(the kept bytes are the sum of `FileData` and field values, still at most twice the body), `chunked_parsing_equals_whole_parsing`,
`content_type_parsing_is_total`, `append_field_is_bounded`. A unit test with a counting allocator: pushing a 1 MiB file in 4
KiB chunks copies at most `2 * (needle.len() - 1)` bytes per push (measured as bytes allocated by `push`).

### 4. File buffers live with their request

- `buffer_to_value(req, bytes)` stores the bytes in a `BufferStore` (a `Mutex<Vec<Bytes>>` behind an `Arc`) kept in the
  request extensions and created on first use; the handle is the index in that store (`{ "type": "Buffer", "handle": "<n>"
  }`). The store is freed with the request's extensions. No global registry, no `Lease`.
- `buffer_from_value(v)` keeps its signature: it resolves the handle against the store of the request being processed,
  found through a task-local (`tokio::task_local!`) that holds the request's extensions handle (an `Arc` clone, no
  allocation). The router sets the scope around the route pipeline in `execute_route` (guards, interceptors, pipes, the
  handler and the filters). Outside a scope, or for a handle the store does not have, it returns `None`, as for an unknown
  handle today. Node's `{ "type": "Buffer", "data": [...] }` form is still accepted anywhere.
- Consequence: a handle can only reach bytes of its own request; a code path that resolves a handle after the request
  finished, or in a spawned task, gets `None` (rustdoc of both functions).

Tests: two concurrent uploads each resolve their own `file.buffer` (`UploadedFile` and `ParseFilePipe`); a request whose
JSON body carries the handle of a concurrent request's buffer resolves `None`; after a request ends its store is freed (a
`Weak` to the store does not upgrade); the existing `single_file_and_fields`, `returned_file_serializes_like_express` and
`parse_file_pipe` pass unchanged.

### 5. Cleanup failures are visible

`Cleanup::run`, the cleanup after a failed read and `DiskStorage`'s removal of a partial file log a failed removal as a
warning with the path and the error, rate-limited to one line a minute per kind with the count left out (the logger
pattern of the server's other client-provoked lines). An engine's `Ok` for a file whose stream ended in error is followed by
`remove_file` (Decision 2).

Tests: a storage engine whose `remove_file` fails: the response is the same as today and one warning line is logged; a
disk upload cut by the file limit in the middle of the file leaves no file in the destination; one cut by a client abort
likewise.

### 6. The memory an upload holds, measured

- `tests/multer_memory.rs` (counting allocator tracking the live heap and its peak): a 64 MiB upload through
  `disk_storage` with `body_limit(80 MiB)` and `file_size(64 MiB)`, sent in 64 KiB frames over a real socket, raises the
  peak live heap by less than 2 MiB over the request's baseline (today: more than 128 MiB); the same upload through
  `memory_storage` raises it by less than 64 MiB plus 2 MiB (today: about twice the file).
- Benchmark: `bench/http` gains `upload-disk-8m` (an 8 MiB file to a `disk_storage` route) and `upload-memory-1m` (a 1 MiB
  file to a `memory_storage` route). Numbers it must move, recorded before and after: bytes allocated per request of
  `upload-disk-8m` fall from more than 16 MiB to less than 1 MiB; those of `upload-memory-1m` fall by at least 1 MiB (the
  second concatenation); requests per second of both do not fall.

## Security invariants that must hold

From the multipart and transport areas of the first security audit and `e0-finish-fuzzing-serve-static-end-to-end-and-orm-filters`.

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| A multipart body is read only by a file interceptor, after the guards; unknown routes answer without reading it | first audit, multipart | 2 | `multipart_to_unknown_route_is_answered_without_reading_the_body` |
| The multipart body limit applies to a declared length (before reading) and while streaming | first audit, multipart | 2 | `multipart_body_limit_rejects_a_declared_length`, `multipart_body_limit_applies_while_streaming` |
| `limits.file_size` aborts the read as soon as it is crossed | first audit, multipart | 2, 3 | `file_size_limit_aborts_the_read`, `file_size_limit_is_inclusive_like_busboy` |
| An unread or half-read body is drained within bounds after the response | first audit, transport | 2 | `unread_body_is_drained_after_the_response`, `drain_after_an_early_error_response_is_bounded_in_bytes` |
| Disk storage names stay inside the destination; no streams or device names on Windows; checked before writing | first audit, multipart | 1 | `disk_storage_stays_inside_the_destination`, `disk_storage_rejects_streams_and_device_names_on_windows`, unit tests `names_must_stay_inside_the_destination`, `windows_names_reject_streams_and_devices`, `path_join_matches_node` |
| Files written are removed when a later pipe or the handler fails | first audit, multipart | 1, 5 | `files_are_removed_when_a_pipe_or_the_handler_fails` |
| The parser is total, every part is backed by a delimiter, kept content at most twice the body, no path in file names without `preservePath` | request-parsing spec | 3 | `busboy_parse_is_total_and_bounded`, `chunked_parsing_equals_whole_parsing`, `content_type_parsing_is_total` |
| Field nesting and sparse indexes bounded; nesting limit is a `400` | request-parsing spec | none (fields unchanged) | `append_field_is_bounded`, `sparse_index_in_field_name_does_not_allocate`, `deeply_nested_field_name_does_not_overflow`, `field_nesting_limit_answers_400` |
| `read_body` reads the pending body once, with the same limits | first audit | none | `read_body_reads_the_pending_multipart_body` |

Added by this change: a partial file never survives a failed upload (Decision 5 tests); a buffer handle never resolves
another request's bytes (Decision 4 tests).

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Custom `StorageEngine` | Receives `Bytes` | Receives a `FileStream`; `collect_file` gives the bytes | Breaking, compile-time; a one-line change for engines that want the bytes |
| Disk upload memory | The whole file, twice at the end | A few chunks | Large uploads stop costing memory |
| Upload cut mid-file (limit, abort, timeout) with disk storage | Nothing was written yet | The partial file is removed before the error answer | No leftover; same response |
| A slow storage engine | Body read at full speed into memory | Body read at the engine's pace | Backpressure to the client |
| `buffer_from_value` outside the request's pipeline | Resolved while the request was alive | `None` | Code that resolves buffers in spawned tasks must copy the bytes first |
| A client-sent buffer handle | Resolved against every request's buffers | Only against its own request's | No cross-request lookup |
| `remove_file` failure | Silent | One rate-limited warning | Visible in logs |

## Order with the other changes

- After `q-http-server-robustness` (the pending body is a boxed body, used by `BodySource`; the test client reaches the
  file interceptor through the streaming path, so the tests of this change run through it).
- After `q-http-request-hot-path` (its benchmark gains the two upload scenarios); if that change slips, this one may land
  first and record its numbers with the counting-allocator tests of Decision 6 only.
- After `e3-authentication-hot-path-and-consolidation` (`Request::insert` and `get` no longer panic on a poisoned lock,
  which the per-request store uses).
- Independent of `e2-socket-io-hot-path-and-api-hygiene` and of `q-http-response-consistency`.

## Risks / Trade-offs

- Every custom storage engine breaks → before 1.0, on purpose; the rustdoc and `docs/application/file-storage.md` show the
  two-line migration with `collect_file`.
- Polling the engine inside the read loop is easy to get wrong (a missed poll stalls the upload) → the loop has one
  `select!` per wait, and tests cover a slow engine (it awaits a signal before taking each chunk) and an engine that does
  unrelated async work between chunks.
- The task-local scope must cover every place a buffer is resolved → the two callers in the code run in the pipeline; the
  rustdoc says where resolution works; a test covers a custom pipe and a filter.
- The parser change touches a fuzzed, security-relevant parser → the existing properties are adapted, not weakened, and
  run with 100,000 cases before merge.

## Open questions

1. Name of the stream type: `FileStream` or `FileBody`? Recommendation: `FileStream` (multer's `file.stream`).
2. Raise the default multipart body limit now that disk uploads stream? Recommendation: no, not here; the limit also bounds
   memory storage and the time a client can hold a connection; a separate decision.
3. Channel capacity: one chunk or a few? Recommendation: one; the engine's future is polled in the same task, so a larger
   buffer only adds memory.

## Review notes

These settle the open questions and override the design where they differ.

1. The type is `FileStream`.
2. The default multipart body limit is unchanged; its rustdoc no longer says files are kept in memory, and says memory
   storage holds the file.
3. Channel capacity is one chunk.
4. The security review covers the stream contract (errors, early completion), cleanup of partial files, the limits while a
   file is in progress, the parser's boundary search across chunks, and the scope of buffer handles.

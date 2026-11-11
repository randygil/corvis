# Spec Delta

## Purpose

How file interceptors hand uploaded files to storage engines: the stream contract of `StorageEngine`, the memory an
upload holds, what is cleaned up when an upload fails, and where file buffer handles resolve. Introduced by the issue of
`q-http-multipart-streaming` (epic #187).

## ADDED Requirements

### Requirement: Storage engines receive the file as a stream
A file interceptor SHALL hand each accepted file to its `StorageEngine` as a stream of chunks, in the request's own task,
reading the next part of the body only as fast as the engine takes the chunks. When a limit is crossed, the client
aborts or the request times out while a file is in progress, the stream MUST end with an error item and the interceptor
MUST answer with its own error.

#### Scenario: Slow engine
- **WHEN** a custom engine takes each chunk only after the test signals it
- **THEN** the server reads no more than two chunks of the file ahead of the engine

#### Scenario: File limit in the middle of a file
- **WHEN** an upload crosses `limits.file_size` while the engine is writing the file
- **THEN** the engine's stream ends with an error item and the client receives the `File too large` answer it receives today

#### Scenario: Engine that stops early
- **WHEN** an engine returns `Ok` before its stream ended
- **THEN** the rest of the file's part is discarded and the engine's result is used for the file

### Requirement: An upload holds bounded memory
An upload through `disk_storage` SHALL hold a bounded number of chunks of the body in memory, whatever the file size. An
upload through `memory_storage` SHALL hold each file once.

#### Scenario: Large disk upload
- **WHEN** a 64 MiB file is uploaded through `disk_storage` over a socket
- **THEN** the server's peak live heap during the request grows by less than 2 MiB

#### Scenario: Memory upload
- **WHEN** a 64 MiB file is uploaded through `memory_storage`
- **THEN** the peak live heap grows by less than the file size plus 2 MiB

### Requirement: A failed upload leaves no partial file
When an upload through `disk_storage` fails while a file is being written (a limit, a client abort, a timeout, a write
error), the partial file MUST be removed before the error is answered. A removal that fails SHALL be logged as a
rate-limited warning with the path.

#### Scenario: Client abort mid-file
- **WHEN** a client closes the connection halfway through a file written by `disk_storage`
- **THEN** no file of that upload remains in the destination

#### Scenario: Removal that fails
- **WHEN** a storage engine's `remove_file` fails while an upload is undone
- **THEN** the response is the same as when it succeeds and one warning line names the file

### Requirement: File buffers resolve only within their request
A `file.buffer` handle created by `buffer_to_value` SHALL resolve, through `buffer_from_value`, only while its own request
is processed (guards, interceptors, pipes, handler and filters), and MUST NOT resolve against another request's buffers.
The buffers MUST be freed with their request.

#### Scenario: Two concurrent uploads
- **WHEN** two requests upload different files to a `memory_storage` route at the same time
- **THEN** each handler's `UploadedFile` holds its own request's bytes

#### Scenario: A foreign handle in a body
- **WHEN** a request's JSON body carries the buffer handle of another request in progress
- **THEN** `buffer_from_value` returns `None` for it

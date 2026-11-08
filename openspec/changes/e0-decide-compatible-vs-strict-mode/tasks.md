# Tasks

## 1. Decision

- [ ] 1.1 Maintainer reviews design.md (decisions 1 and 2, open questions 1-4) and records the outcome in a
  "Review notes" section at the end of design.md; if an option other than C is chosen, revise the proposal, spec
  delta and these tasks with `/opsx:update` before continuing; verify proposal.md no longer says "pending maintainer
  approval"

## 2. Multipart nesting limit

- [ ] 2.1 Add a failing test to `crates/corvis/tests/multer.rs`: a route with `NoFilesInterceptor` and
  `Limits::new().field_nesting_depth(2)`; a field `a[b][c][d]` expects `400` with
  `{ "message": "Field name nesting too deep - a[b][c][d]", "error": "Bad Request", "statusCode": 400 }`, and a field
  `a[b][c]` expects the handler's `req.body` `{ "a": { "b": { "c": "v" } } }`; verify it fails today with `500`
  (`cargo test -p corvis --test multer`)
- [ ] 2.2 Move `LimitFieldNesting` into the `BadRequestException` arm of `transform_multer_error`
  (`crates/corvis/src/multer/mod.rs:365-377`); verify the test from 2.1 passes and
  `deeply_nested_field_name_does_not_overflow` (`crates/corvis/tests/transport_security.rs`) still passes
- [ ] 2.3 Repeat the Node probe of design.md (raw socket against `http.createServer`, bound to `127.0.0.1`) on Node 22
  and Node 24 and keep the script and its output in the change folder (`openspec/changes/<this change>/node-probe/`);
  verify the table in design.md matches both versions, or note the differences there

## 3. Chunked framing kept on purpose

- [ ] 3.1 Add raw-socket tests to `crates/corvis/tests/transport_security.rs` (reuse `serve` and `read_head`): chunk
  size `00000000000000000000000002` with a two-byte chunk reaches a JSON echo route with the body; chunk size
  `10000000000000002` gets `400` or a closed connection; `2\nhi\n0\n\n` gets `400` or a closed connection; verify with
  `cargo test -p corvis --test transport_security`
- [ ] 3.2 Add a short comment above the new tests pointing to the `http/request-parsing` requirements and to
  `docs/security/http-parsing.md`; verify the tests' names describe the behavior (no "quirk" wording)

## 4. Follow-up and documentation

- [ ] 4.1 Draft the follow-up issue text in the change folder (`follow-up-issue.md`): reject bare LF line endings in
  the request head, whitespace after a chunk size and `Content-Length` + `Transfer-Encoding`, with
  `ServerOptions::insecure_http_parser(true)` as opt-out; options from design.md decision 3; the Node probe as the
  oracle for a differential test; the maintainer opens it in the epic chosen in open question 3; verify the issue
  number is written back into design.md and the docs page
- [ ] 4.2 Write `docs/security/http-parsing.md`: what Corvis accepts and rejects on the wire and in multipart
  (table of design.md decision 2, written for users), how it compares with Node's `http` module, the advice to let a
  front proxy normalize requests until the follow-up lands, and links to `/http/performance` (`ServerOptions`),
  `/http/file-upload` and `/security/cors`; no use of the original framework's name; verify the page renders with
  `npm run docs:dev` in `docs/`
- [ ] 4.3 Add the page to the Security group of `docs/.vitepress/sidebar.mts`
  (`['HTTP parsing', 'Análisis de HTTP', '/security/http-parsing']` or similar labels) and change the
  `.field_nesting_depth(n)` row of `docs/http/file-upload.md` to say it answers
  `400 Field name nesting too deep - <field>`; verify the links resolve
- [ ] 4.4 Translate both pages into Spanish (`docs/es/security/http-parsing.md`, `docs/es/http/file-upload.md`) with
  the same headings and code blocks; verify `npm run docs:check` in `docs/` passes

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes, including the new tests in `crates/corvis/tests/multer.rs` and
  `crates/corvis/tests/transport_security.rs`
- [ ] 5.4 Regression tests exist for every requirement of the `http/request-parsing` delta (2.1, 3.1)
- [ ] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 5.6 corvis-parity: framework runtime code changed (`multer/mod.rs`), so run the `corvis-parity` check; the three
  port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#8](https://github.com/randygil/corvis/issues/8)
- [ ] Tick #8 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [ ] Move the card to Done in the Corvis Roadmap project

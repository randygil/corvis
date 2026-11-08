# Decide compatible vs strict mode

Implements [randygil/corvis#8](https://github.com/randygil/corvis/issues/8) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

> **Status: decided (2026-11-08).** The maintainer approved option C (per-behavior secure defaults, no global
> mode); design.md, "Review notes", records the decision and the answers to the open questions.

## Why

The security audit left two questions open: Corvis answers `500` when a multipart field name nests deeper than
`limits.field_nesting_depth`, and the HTTP/1 parser accepts bare LF line endings and leading zeros in chunk sizes (no
request smuggling was found). The issue describes these as "Express quirks" and asks whether Corvis should offer a
strict mode, by default or opt-in.

Reading the code and testing the original stack shows the premise only half holds:

- The `500` for nesting copies `@nestjs/platform-express` 11.1.28, the version consi pins. Versions 11.2.1 (facten) and
  12.0.4 (houseplan) already map `LIMIT_FIELD_NESTING` to `400 Field name nesting too deep - <field>`.
- Bare LF is not an Express behavior: it comes from `httparse`, the parser under hyper. Node 24 (llhttp 9.3.0) rejects
  it with `400`. Being strict here moves Corvis closer to Node, not away from it.
- Leading zeros in chunk sizes are accepted by Node too. That is not a divergence at all.

So the decision is less about compatibility versus security than about who owns each leniency, and whether one global
switch is the right tool for a handful of unrelated behaviors.

## What Changes

- **Decision record** (design.md): the inventory of behaviors with security relevance in `crates/corvis/src/http`,
  `server.rs` and `multer/`, what the original Node stack does for each, the parity impact on the three port apps,
  three options (strict by default, opt-in strict, per-behavior switches) and a recommendation.
- **Recommended option C, applied to the one item that is local and cheap:** exceeding `field_nesting_depth` answers
  `400 Field name nesting too deep - <field>`, as current `@nestjs/platform-express` does, instead of `500`.
- **Regression tests that pin the framing behaviors Corvis keeps:** leading zeros in chunk sizes accepted, a chunk size
  that overflows 64 bits rejected, bare LF inside the chunked framing rejected.
- **A docs page, "HTTP parsing and compatibility"** (`docs/security/http-parsing.md` and its Spanish translation): which
  lenient behaviors Corvis has, which it rejects, how it compares with Node, and which ones are planned to change.
- **A follow-up issue** (opened by the maintainer, see task 4.1) to reject bare LF line endings and whitespace after a
  chunk size by default, with an opt-out `ServerOptions::insecure_http_parser(true)` named after Node's
  `insecureHTTPParser`. It needs support in hyper/httparse, so it does not fit this spike.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/request-parsing`: adds the status of the multipart nesting limit and the chunked-framing behaviors that are
  kept on purpose.

## Impact

- Code: `crates/corvis/src/multer/mod.rs` (`transform_multer_error`, one match arm). No new public API.
- Tests: `crates/corvis/tests/multer.rs` (nesting limit), `crates/corvis/tests/transport_security.rs` (chunked
  framing over a raw socket).
- Docs: new `docs/security/http-parsing.md` and `docs/es/security/http-parsing.md`, a sidebar entry in
  `docs/.vitepress/sidebar.mts`, one row fixed in `docs/http/file-upload.md` (and its translation).
- Behavior change: a request that exceeds a configured `field_nesting_depth` gets `400` instead of `500`. None of the
  three port apps sets `field_nesting_depth` (checked with grep), so apidiff is not affected.

## Out of scope

- Rejecting bare LF line endings in the request head and whitespace after a chunk size: a follow-up issue (task 4.1),
  because hyper and httparse expose no option for it (design.md, decision 3).
- Rejecting requests that carry both `Content-Length` and `Transfer-Encoding` with `400`, as Node does (hyper processes
  them as chunked and closes the connection): goes with the same follow-up issue.
- A startup warning when `CorsOptions::reflect_origin()` is combined with `credentials(true)`, as the audit suggested:
  separate issue for the CORS page.
- Turning h2c off by default (Node's `http` does not speak it): separate issue; today `ServerOptions::h2c(false)` exists.
- Mapping busboy configuration errors (`Malformed content type`, `Unsupported content type: ...`) to a 4xx: the
  original answers `500` too and it is not a vulnerability (design.md, open question 2).

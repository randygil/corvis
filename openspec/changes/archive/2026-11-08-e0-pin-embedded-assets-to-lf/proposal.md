# Pin embedded assets to LF

Implements [randygil/corvis#110](https://github.com/randygil/corvis/issues/110).

## Why

On a Windows checkout with `core.autocrlf=true`, the corvis-swagger templates embedded with `include_str!` get CRLF
line endings. The served Swagger UI and Scalar pages then differ byte for byte from the originals, and three lib tests
fail (`swagger_ui::tests::html`, `swagger_ui::tests::init_js`, `scalar::tests::page`).

## What Changes

- A root `.gitattributes` that pins to LF every file embedded at compile time from `crates/*/src/assets/`, and the text
  fixtures under `crates/*/tests/fixtures/` (`json`, `sql`, `pem`, `prisma`, `mjs`), which tests compare or parse.
- The index is renormalized (no content change: the files were already stored with LF).

## Capabilities

None: build hygiene only, no behavior or spec change.

## Impact

- New `.gitattributes`. No Rust code changes.
- Out of scope: line endings of the rest of the repository (sources, docs), which are not byte-compared.

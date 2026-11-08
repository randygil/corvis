# Design

## Context

`include_str!` / `include_bytes!` in the workspace (checked with grep):

- `crates/corvis-swagger/src/swagger_ui.rs`: `assets/swagger-ui-index.html.tpl`, `swagger-ui-init.js.tpl`,
  `swagger-ui-favicon.html.tpl`; `crates/corvis-swagger/src/scalar.rs`: `assets/scalar-theme.css`. Served to clients and
  compared byte for byte with the originals by the lib tests. These are the files that break.
- Test fixtures: `crates/corvis/tests/fixtures/` (`v8_json_parse.json`, `res_express.json`, `zod4/*.json`,
  `jwt_rsa_*.pem`) and `crates/corvis-orm/tests/fixtures/oracle/` (`migration.sql`, `results.json`). Parsed rather than
  compared today, but they are recorded output of the Node originals, so they are pinned too.

## Decisions

1. **`text eol=lf` by path, not `-text`.** Files stay diffable and normalized in the index, and every platform checks
   them out with LF. Patterns name the asset folder and fixture extensions, so binary fixtures added later are not
   forced through text conversion.
2. **No global `* text=auto eol=lf`.** It would rewrite the whole working tree for every Windows contributor; only the
   byte-sensitive files need it.

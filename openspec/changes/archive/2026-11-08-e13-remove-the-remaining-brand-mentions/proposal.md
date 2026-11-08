# Proposal

Implements [randygil/corvis#22](https://github.com/randygil/corvis/issues/22) (epic [#13](https://github.com/randygil/corvis/issues/13)).

## Why

The public naming rule says the words "Nest", "NestJS" and "Prisma" do not appear in public APIs, docs or error
messages, apart from nominative phrases such as "compatible with schema.prisma". The docs site already enforces it
(`docs/scripts/check.mjs:107`), but published surfaces outside the site still break it: rustdoc comments that docs.rs
will render, the `corvis` crate description (which copies the original framework's tagline), the benchmark report,
and `corvis-orm` defaults (`prisma/` folder, `_prisma_migrations` table) that were never explicitly decided. Nothing
stops new mentions from coming back.

## What Changes

- Rewrite the rustdoc and code comments that cite the original framework or its packages (`crates/corvis/src/http/sse.rs`,
  `jwt.rs`, `modules/serve_static.rs`, `modules/throttler.rs`, one test comment) so they describe the behavior itself.
- Replace the `corvis` crate description in `crates/corvis/Cargo.toml` and the opening of `crates/corvis/src/lib.rs`
  with Corvis's own wording instead of the original framework's tagline.
- Benchmark: title, labels and generated text of `bench/report.js` / `bench/results.md` / `bench/run.sh` say
  "Node.js (Express)" and "Node.js (Fastify)"; the Node app folder becomes `bench/node-app` and the framework ids
  `node-express` / `node-fastify`. The Node app's own source keeps importing its real packages.
- `corvis-orm` migrations: keep `_prisma_migrations` as the control table and the `prisma/` folder in the default
  schema search (both are compatibility identifiers of existing projects, so they are allowlisted, not renamed).
- A repository naming check (`scripts/check-naming.mjs`) over published surfaces, with an allowlist of nominative
  uses, so the rule is enforced outside the docs site too.
- Record every occurrence category and its decision in design.md.

## Capabilities

### New Capabilities

- `repo/naming`: published surfaces of the repository (crate sources and metadata, examples, README, benchmark
  report, docs) contain no banned brand words except allowlisted nominative uses, checked by a script.

### Modified Capabilities

None (no existing spec covers these).

## Impact

- Comments only: `crates/corvis/src/http/sse.rs`, `crates/corvis/src/jwt.rs`, `crates/corvis/src/modules/serve_static.rs`,
  `crates/corvis/src/modules/throttler.rs`, `crates/corvis/tests/static_response_security.rs`,
  `bench/corvis-app/src/main.rs`.
- Metadata: `crates/corvis/Cargo.toml`, `crates/corvis/src/lib.rs` (crate docs).
- `crates/corvis-orm/src/migrate.rs`, `crates/corvis-orm/src/client.rs` (new option), `crates/corvis-orm/src/bin/corvis-orm.rs`
  (flag, search order, help text): small runtime change, default behavior unchanged.
- `bench/` (folder rename, report text), `.gitignore` (bench paths).
- New `scripts/check-naming.mjs`; docs pages for `corvis-orm` migrations in English and Spanish.

## Out of scope

- The root `README.md` mentions: #3 rewrites it.
- `crates/corvis-orm/README.md` is in Spanish and mentions the original project nominatively; its translation and
  rewrite belong to the corvis-orm publishing epic's README task.
- Error messages copied verbatim for parity that contain `pris.ly` links (`crates/corvis-orm/src/error.rs:284`,
  `migrate.rs:215`, `migrate.rs:263`): kept, see design Decision 6 and Open questions; may become an issue.
- Translating the benchmark report and scripts to English: belongs to #12 (reproducible benchmark).
- The Cargo target path `C:/Users/Usuario/Devel/nestrs/target` in `.claude/settings.json`: #7 (folder rename).
- Internal planning files (`openspec/config.yaml`, `ROADMAP.md`, `scripts/roadmap/*`, `.claude/*`) and
  `THIRD_PARTY_NOTICES.md` (license attribution must name the projects).
- The `ParseError` display prefix `schema.prisma:<line>` in `crates/corvis-orm/src/psl.rs:249`, which is wrong when the
  file has another name or the schema is a folder: an accuracy issue, not a naming one; possible new issue.

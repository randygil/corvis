# Design

## Context

See proposal.md for motivation; the guarantees are in the `repo/naming` and `orm/migrations` spec deltas. The whole
tracked tree was searched with `git grep -P -i 'nest(?!ed|ing|le)|prisma'` (to skip "nested", "nesting", and words
like "soonest") plus `git grep -i 'pris\.ly|nestjs\.com|prisma\.io'`. The root `README.md` and identifiers of the
public API were also checked: no public type, function, attribute or error variant contains the banned words (the
application types are `CorvisFactory`, `CorvisApplication`, `CorvisInterceptor`, ...; `crates/corvis/src/lib.rs:87-126`).

### Occurrence inventory by category

**A. Rustdoc and code comments in crate sources (published on docs.rs or read by users) — remove.**

| Location | Text today |
|---|---|
| `crates/corvis/src/http/sse.rs:46` | `/// The event as written on the wire (Nest 11 \`SseStream\`): ...` |
| `crates/corvis/src/jwt.rs:27` | `/// ... As in \`@nestjs/jwt\`, a non-empty \`secret\` takes precedence ...` |
| `crates/corvis/src/jwt.rs:204` | `// \`@nestjs/jwt\`: \`options.secret \|\| this.options.secret \|\| ...\`` |
| `crates/corvis/src/jwt.rs:252` | `// \`@nestjs/jwt\`: \`this.options.secret \|\| this.options.publicKey\`` |
| `crates/corvis/src/jwt.rs:395` | `// As in \`@nestjs/jwt\`, an empty per-call secret falls back to the module secret.` |
| `crates/corvis/src/jwt.rs:449` | `/// \`publicKey\` / \`privateKey\` for RS256, as in \`@nestjs/jwt\`.` |
| `crates/corvis/src/modules/serve_static.rs:3` | `//! Like \`@nestjs/serve-static\` on Express, ...` |
| `crates/corvis/src/modules/throttler.rs:65` | `/// Like \`@nestjs/throttler\`, keys are stored as \`sha256(key)\` ...` |
| `crates/corvis/tests/static_response_security.rs:252` | `/// static-response #4: SSE framing like Nest 11 ...` |

**B. Crate metadata and crate-level docs — replace.**

- `crates/corvis/Cargo.toml:3` and `crates/corvis/src/lib.rs:3-4`: "A progressive Rust framework for building
  efficient and scalable server-side applications" is the original framework's tagline with "Node.js" swapped for
  "Rust". Other crate descriptions (`corvis-macros`, `corvis-orm`, `corvis-orm-macros`, `corvis-swagger`,
  `corvis-swagger-macros`) are Corvis's own; `corvis-orm`'s says "compatible with schema.prisma projects" (nominative,
  allowed). `crates/corvis-orm-macros/src/lib.rs:2` and `crates/corvis-orm/src/lib.rs:6` say "the `schema.prisma`
  equivalent" (nominative, allowed). `crates/corvis-cli/Cargo.toml` has no description (packaging, #10).

**C. corvis-orm defaults, CLI help and rustdoc — decide (Decision 4 and 5).**

- Control table `_prisma_migrations`: `crates/corvis-orm/src/migrate.rs:109`, `121` (CREATE TABLE for Postgres and
  SQLite), `183` (SELECT), `231`, `243`, `256` (INSERT/UPDATE), docs at `migrate.rs:1-3`, `20`, `202`. The name is a
  fixed string literal in every statement.
- Default schema search `["prisma/schema.prisma", "schema.prisma", "prisma/schema"]` and fallback
  `prisma/schema.prisma`: `crates/corvis-orm/src/bin/corvis-orm.rs:60-67`; `.env` search `[".env", "prisma/.env"]`:
  `corvis-orm.rs:69-70`; migrations folder derived from the schema location: `corvis-orm.rs:150-153`.
- Help text: `corvis-orm.rs:5-7` (module docs), `17-18` (`generate` "from schema.prisma", default path), `22`
  ("Apply the pending prisma/migrations/*/migration.sql", which is not how the folder is chosen).
- `crates/corvis-orm/src/psl.rs:1`, `10`, `249`, `773`, `779-786`, `827`; `codegen.rs:1`, `119`; `meta.rs:1`, `6`:
  file-format references (nominative, allowed). `psl.rs:779` names the preview feature `prismaSchemaFolder`
  (nominative, allowed).

**D. Error messages — keep for parity (Decision 6).**

- `crates/corvis-orm/src/error.rs:284` (P2024): "... More info: http://pris.ly/d/connection-pool".
- `crates/corvis-orm/src/migrate.rs:215` (P3009) and `263` (P3018): "... https://pris.ly/d/migrate-resolve".
  No banned word, but the links point to the original vendor's docs.

**E. Benchmark — rename user-visible text.**

- `bench/results.md` (generated): title "Benchmark de throughput: NestJS vs Corvis" (line 1), environment line 14,
  table rows 22-35, heading 37, columns 39, memory rows 52-53, notes 64, command 75.
- `bench/report.js:60`, `70`, `80-84`, `125` (the generator of those texts).
- `bench/run.sh:2`, `11`, `30`, `37`, `92-99`, `111-112`, `123-124` (ids `nest-express`/`nest-fastify`,
  `NEST_MAIN`, the `bench/nest-app` path).
- `bench/corvis-app/src/main.rs:1-2`, `8` (comments).
- `bench/nest-app/package.json:2`, `5` (package name and description); `bench/nest-app/src/*.ts` import
  `@nestjs/common` and `@nestjs/core` — that is the original framework the benchmark measures, nominative.
- `.gitignore:10-11` (`bench/nest-app/node_modules`, `dist`).

**F. Tests and fixtures that run against or record the original libraries — keep (allowlisted).**

- `crates/corvis/tests/fixtures/res_express.oracle.mjs:2-6`, `46`; `crates/corvis/tests/fixtures/zod4/pipe.mjs:3-11`;
  `crates/corvis/tests/fixtures/v8_json_parse.json` (generated output).
- `crates/corvis-orm/tests/pg_oracle.rs:5`, `66-69`, `143` (maps `OrmError` to the original client's error class
  names to compare with `tests/fixtures/oracle/results.json`, 104 matches); `tests/fixtures/oracle/schema.prisma:2`.
- `crates/corvis-orm/tests/common/pg.rs:31-37`, `tests/pg_houseplan.rs:15`, `tests/generated/*.rs:1` (paths of the
  original reference apps in `corvis-tests/nest/...`, outside the repository), `tests/generated/houseplan.rs:621`
  (a schema comment copied by the generator).
- `examples/blog-orm/src/models.rs:1` ("The `schema.prisma` of the blog", nominative).

**G. Documentation — already compliant.**

- `docs/scripts/check.mjs:107` enforces `\b(Nest|NestJS|Prisma)\b` on the site; the remaining matches are
  `schema.prisma` (`docs/data/corvis-orm.md:103-106`, `249`; `docs/data/overview.md:14`, and the `/es/` copies).
  `docs/README.md:52` states the rule. `crates/corvis-orm/README.md` (Spanish) mentions `prisma/migrations`,
  `_prisma_migrations` and `schema.prisma` nominatively; out of scope (corvis-orm README task).
- Root `README.md:919`, `1369`, `1396`, `1429`: handled by #3.

**H. Internal planning, tooling and legal files — keep.**

- `openspec/config.yaml`, `ROADMAP.md`, `scripts/roadmap/*.mjs` and `state.json` (positioning and task text),
  archived OpenSpec changes and `openspec/specs/*` (they quote the rule or the original behavior), `.claude/agents/*`,
  `.claude/skills/corvis-parity/SKILL.md` (names the reference apps), `.claude/settings.json:3` (`nestrs` target path,
  #7). `THIRD_PARTY_NOTICES.md:15-18`, `51`, `223-225`: license attribution must name the projects.

## Goals / Non-Goals

**Goals:**
- No banned word on any published surface except allowlisted nominative uses.
- Explicit, documented decisions for the `corvis-orm` defaults.
- A check that keeps the rule from regressing.

**Non-Goals:**
- Changing migration behavior for existing databases or folder layouts.
- Removing references from internal planning files, license notices or differential test oracles.
- Translating the benchmark report (#12) or the `corvis-orm` README.

## Decisions

1. **Comments describe behavior, not lineage.** Each comment in category A is rewritten to state the rule it
   documents: e.g. `jwt.rs:27` becomes "A non-empty `secret` takes precedence over `private_key`/`public_key`";
   `jwt.rs:204` keeps the precedence order as plain text ("per-call secret, then module secret, then per-call private
   key, then module private key"); `sse.rs:46` describes the framing; `throttler.rs:65` says why keys are hashed.
   Where the parity source matters to a maintainer, the comment may say "as the original Node module does" without
   naming it. Alternative considered: keep `@nestjs/...` package names as nominative references; rejected because
   the issue lists these files explicitly and rustdoc comments are public docs on docs.rs.
2. **New `corvis` description.** Proposed text for `Cargo.toml` and the first paragraph of `lib.rs`: "A modular
   backend framework for Rust: modules, dependency injection, controllers, guards, interceptors, pipes, exception
   filters, middleware and WebSockets." (aligned with `openspec/config.yaml` and the README tagline). The final wording
   is an open question for the maintainer.
3. **Benchmark naming.** User-visible labels become "Node.js + Express" and "Node.js + Fastify" (already the README's
   wording), the title "Throughput benchmark: Node.js vs Corvis". The folder `bench/nest-app` is renamed to
   `bench/node-app` (package name `node-bench-app`), ids to `node-express`/`node-fastify` and `NEST_MAIN` to
   `NODE_MAIN`; `.gitignore` follows. The Node sources keep their real imports. `bench/results.md` is regenerated
   only if a benchmark run is available; otherwise its text is edited to match what `report.js` now generates, keeping
   the measured numbers. Alternative considered: keep folder and ids and change only titles; rejected because the
   ids appear in the published report's "how to rerun" command.
4. **Keep `_prisma_migrations` as the default, make it configurable.** The table name is part of the compatibility
   contract ("a database migrated by the original tooling can be migrated by corvis-orm and the other way around",
   `migrate.rs:1-3`); renaming the default would break every ported app's existing database. It is a database
   object name, a nominative use like `schema.prisma`. New `OrmClientOptions::migrations_table` (default
   `"_prisma_migrations"`, builder `migrations_table(name)`), validated in `OrmClient::new` against
   `^[A-Za-z_][A-Za-z0-9_]{0,62}$` (63 is the Postgres identifier limit) and always double-quoted, so no user text is
   ever interpolated unchecked; every statement in `migrate.rs` uses it. CLI flag `--migrations-table <name>` on
   `migrate deploy|status`. Alternative considered: a different default (`_corvis_migrations`) with an opt-in
   compatibility flag; rejected as a breaking change whose only gain is naming. Alternative considered: no option at
   all; rejected because teams adopting `corvis-orm` from scratch reasonably want their own table name.
5. **Keep `prisma/` in the schema search, add `db/`.** Existing projects keep the `prisma/` layout, so it stays first
   and the order is unchanged; `db/schema.prisma` and `db/schema` (the layout the docs use, `docs/data/corvis-orm.md:106`)
   are appended. `prisma/.env` stays in the `.env` search and `db/.env` is appended. The help text lists the search
   order and says the migrations folder is `<schema dir>/migrations`. Alternative considered: dropping `prisma/` from
   the defaults; rejected, it breaks the main migration path for no user benefit.
6. **Error texts copied for parity stay verbatim.** P2024, P3009 and P3018 messages mirror the original client's
   messages, including their `pris.ly` links; ported apps may log or match them, and the rule bans words, not links.
   The naming check allowlists them with that reason. Flagged as an open question.
7. **Naming check as a Node script.** `scripts/check-naming.mjs` (Node, no dependencies, same style as
   `docs/scripts/check.mjs`) runs `git ls-files` over the published surfaces of the `repo/naming` spec and matches
   `\b(Nest|NestJS)\b|Nest[A-Z]\w*|\bPrisma\w*|@nestjs/|nestjs-` (case-sensitive, so `nested`, `_prisma_migrations`
   and `schema.prisma` do not match), plus the copied tagline `efficient and scalable server-side`. Allowlist entries are `{ path glob, optional pattern, reason }` in the script;
   an empty reason fails. CI (#6) runs it; `CONTRIBUTING.md` documents it. Alternative considered: a Rust test in
   `cargo test --workspace`; rejected because the scan covers non-Rust files and has nothing to do with any crate.

## Risks / Trade-offs

- [A rewritten comment loses the pointer to the upstream behavior a maintainer needs for parity work] → the comment
  keeps the exact rule (precedence order, framing), which is what the code must do; the upstream source is in
  `THIRD_PARTY_NOTICES.md`.
- [The configurable table name is new SQL surface] → strict identifier validation before any statement, quoted
  identifiers, and a test with a hostile name; the security reviewer checks it.
- [Renaming `bench/nest-app` breaks local `node_modules` paths] → only a reinstall; `run.sh` installs when missing.
- [The allowlist hides a real violation] → every entry needs a reason and a path; reviewers see the list in one place.

## Open questions

- Final wording of the `corvis` crate description (Decision 2).
- Keep the `pris.ly` links in parity error messages (Decision 6), or replace them with links to Corvis docs and
  accept a message difference from the original client?
- Should `prisma/.env` keep being read by default, or only when the schema is found under `prisma/`?
- Should the naming check also run inside `npm run docs:check`, so it runs where contributors already run checks
  before CI exists?

## Review notes (2026-11-08)

The maintainer's review narrows decisions 4 and 5 to what the issue asks (remove brand mentions):

- The migration control table stays `_prisma_migrations` and the schema search keeps `prisma/`; both are compatibility
  identifiers and go in the naming check's allowlist. A configurable table name is a new feature, out of scope here.
- Adding `db/schema.prisma` to the CLI's schema search fixes a mismatch between the docs (`docs/data/corvis-orm.md`)
  and the CLI, so it belongs to #20 (CLI defaults that contradict the docs), not to this change.
- `pris.ly` links in P2024/P3009/P3018 messages stay for parity (they contain no banned word).

## Implementation notes

- **Naming check patterns.** Besides the case-sensitive brand pattern of Decision 7, `scripts/check-naming.mjs` has
  four more groups: packages and ids (`@nestjs/`, `nestjs`, `nest-<id>`, `NEST_<VAR>`), a lowercase `prisma` that is
  not a file extension (folders, commands, the client variable; `schema.prisma` and `*.prisma` do not match because of
  the leading dot) plus `_prisma_migrations`, vendor links (`pris.ly`, `prisma.io`, `nestjs.com`) and the copied
  tagline. Without the lowercase and link groups, categories C and D would never match, and the allowlist entries the
  spec and task 3.1 ask for would allow nothing. A lowercase `nest` is not matched: it is an English verb
  (`multer/fuzz.rs` uses it).
- **File set.** `git ls-files --cached --others --exclude-standard`, so untracked files a contributor is about to commit
  are checked too, filtered to `Cargo.toml`, `README.md`, `crates/**` (sources, tests, fixtures, crate READMEs and
  manifests), `examples/**`, `docs/**`, `bench/report.js`, `bench/results.md`, `bench/run.sh` and
  `bench/corvis-app/**`. `bench/node-app/` is not a published surface: its sources import the framework they measure.
- **Allowlist semantics.** An entry with a `pattern` blanks only the text that pattern matches, so another mention on
  the same line is still reported; an entry without one allows the whole file. Besides an empty reason, an entry that
  no longer allows any banned word fails the check, so entries such as the temporary root `README.md` one (#3) have
  to be removed when their reason goes away. The `crates/corvis-orm/README.md` folder paths are allowlisted as
  compatibility identifiers; that README's rewrite is out of scope.
- **Self-test.** `node scripts/check-naming.mjs --self-test` runs the matcher on built-in samples (planted mentions
  are reported; `nested`, `schema.prisma`, `_prisma_migrations` and allowlisted fixtures pass; an empty reason
  fails). This is the regression test of task 6.4, runnable without cargo. Verified as well by planting
  `// As in @nestjs/jwt` in an untracked file under `crates/corvis/src/`: the check reported it with file and line
  and exited 1. The naming check does not run inside `npm run docs:check` (open question left for CI, #6).
- **Comments.** The `asymmetric_keys` test comment in `jwt.rs` described the test with the original option names (`publicKey` / `privateKey`); it now uses
  the Rust field names. The `serve_static.rs` module doc keeps `express.static`, the library whose behavior it copies,
  which is not a banned name.
- **Crate description.** Used the wording proposed in Decision 2 for `Cargo.toml`; the `lib.rs` opening became "A
  modular backend framework for Rust, built around modules, controllers, ...", keeping its existing list.
  `git grep 'efficient and scalable'` now only finds this change's planning files and the check's own pattern and
  self-test sample.
- **Benchmark language.** The report and `run.sh` stay in Spanish (translation is #12); only the labels changed:
  "Benchmark de throughput: Node.js vs Corvis", "Node.js + Express", "Node.js + Fastify". `bench/results.md` was
  edited, not regenerated (no run was made); every number is unchanged. `bash -n bench/run.sh` and
  `node --check bench/report.js` pass; a real run needs `cargo build` and was left to the maintainer. The
  `package-lock.json` name fields follow the new package name. The `e0-reproducible-benchmark-port-vs-original`
  change still mentions `bench/nest-app` in its design; it was not edited (another change).
- **Docs.** No docs page changed: the benchmark pages already say "Node.js + Express" / "Node.js + Fastify" and no
  page quotes the old crate description or the rewritten comments. `npm run docs:check` passes (173 + 173 pages).
- **Parity.** No runtime string or code path changed (comments, crate metadata, benchmark scripts and a new Node
  script only), so the corvis-parity check is not needed.

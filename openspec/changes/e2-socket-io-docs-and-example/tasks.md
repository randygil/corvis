# Tasks

Start after #37 (`e2-socket-io-differential-tests`) is merged into `dev`, and with it #32 to #36. No framework
runtime code changes here: a defect of the framework found on the way is reported as an issue under epic #30 and
worked around or waited for, not fixed in this change. Servers bind to `127.0.0.1` only. Tests wait on events,
never on time. The docs pages are written with the `docs-writer` agent; English first, Spanish in group 6.

## 1. Reconcile

- [ ] 1.1 Compare design.md Decisions 2, 4 and 8 with the code merged by #32 to #37: `use_websocket_adapter`,
  `IoAdapter` and its options, `guard_connections` and what a guard sees for the `connect` pseudo-handler,
  `IoSocket` (`handshake`, `insert` / `get`, `on`, `join`, `to`), `IoGatewayServer`, the generic lifecycle traits,
  `fetch_sockets`, `RedisAdapter::connect`, the posture entry of a connection; record renamed or different items in
  a new "Implementation notes" section of design.md; verify `cargo doc -p corvis --features socket-io-redis
  --no-deps` builds and every item named in Decision 2 is found in it
- [ ] 1.2 Inventory the "Socket.IO (preview)" section of `docs/websockets/adapter.md` and the Socket.IO parts of
  `gateways.md` and `guards.md` as merged: list in Implementation notes each subsection and table with its
  destination (guide section, limits table, migration page, stays, or dropped as duplicated); verify nothing of the
  section is left without a destination

## 2. The example

- [ ] 2.1 `examples/socket-io-chat/`: `Cargo.toml` (`publish = false`, `corvis` with `socket-io` and
  `socket-io-redis`), `src/main.rs` (`127.0.0.1:$PORT`, default 5160), `src/lib.rs` with `create_app()` (the
  Socket.IO adapter with `guard_connections(true)`, the origin allowlist from `ALLOWED_ORIGINS`, no CORS, global
  prefix `/api`, static files, `require_route_posture()`), `JWT_SECRET` required; `tests/startup.rs` for spec
  "Missing secret"; verify `cargo test -p socket-io-chat --test startup` and that `cargo build --workspace` has no
  warnings
- [ ] 2.2 `src/auth/` (login for any valid nickname, `ChatUser`, `AuthGuard` for HTTP, for the `connect`
  pseudo-handler and for messages, as in design.md Decision 2) and `tests/support/mod.rs` (the raw Socket.IO client
  of Decision 4); tests in `tests/chat.rs` for "No token" and "Valid token"; verify
  `cargo test -p socket-io-chat --test chat`
- [ ] 2.3 `src/chat/` and `src/rooms/`: the gateway on `/chat` with `join`, `leave`, `message`, `typing`, `rooms`,
  zod payloads with length limits, `NOT_IN_ROOM` as a `WsException` with a code, `welcome` on connection, rooms told
  on disconnect, in-memory history of 50 messages per room; tests for "Join is acknowledged", "Broadcast without the
  sender", "Error as an exception event", "Oversized message" and typing; verify the tests
- [ ] 2.4 REST: `GET /api/rooms/:room/members` (`fetch_sockets`) and `POST /api/rooms/:room/announcements`
  (moderators, `IoGatewayServer`); the low-level `avatar` handler registered in the connection hook with a 64 KiB
  bound; tests for "Announcement", "Members", "Report in a test" (posture), "Handshake from another origin" and "No
  CORS headers"; verify the tests
- [ ] 2.5 The Redis adapter when `REDIS_URL` is set (URL never logged) and `tests/cluster.rs` with the gating of
  Decision 4; a startup test for "Redis unreachable at startup"; verify `cargo test -p socket-io-chat` passes
  without Redis (cluster test skips with a notice), passes with `CORVIS_REDIS_URL` set (spec "Two instances"), and
  fails with `CORVIS_REDIS_REQUIRED=1` and no server
- [ ] 2.6 `public/`: `index.html`, `app.js`, `styles.css` and `vendor/socket.io.min.js` with its `LICENSE`, copied
  from the harness's installed `socket.io-client` (Decision 3); the entry in `THIRD_PARTY_NOTICES.md`; verify by
  hand in two browser windows (login, join, messages both ways, typing, a refused connection after clearing the
  token, an announcement, the avatar round trip), that the page requests nothing from another origin (browser
  network panel), and that `node scripts/check-naming.mjs` passes
- [ ] 2.7 `parity/socket-io/example-checks/socket-io-chat.mjs` (two official clients: connect error, connect, join,
  message, `exception`, binary acknowledgement; the vendored bundle equals the installed one) and
  `tests/official_client.rs` gated by `CORVIS_NODE_DIFF=1`; one step in the `socket-io-diff` job of
  `.github/workflows/ci.yml` running `cargo test -p socket-io-chat --test official_client --test cluster`; verify
  the test skips with a notice without the variable, passes with it, fails after changing one byte of the vendored
  bundle (revert), and that the job is green on a pull request
- [ ] 2.8 `examples/socket-io-chat/README.md` (English: what it shows, how to run, variables, the protocol table,
  two instances with Redis, the structure, the tests, the warning about the login); the `socket-io-chat` entry of
  `scripts/smoke-examples.mjs` and its mention in `CONTRIBUTING.md` ("Smoke test of the examples"); verify
  `cargo build -p socket-io-chat` then `node scripts/smoke-examples.mjs socket-io-chat` passes, and every command of
  the README runs as written

## 3. The guide (English)

- [ ] 3.1 `docs/websockets/socket-io.md`, sections 1 to 7 of design.md Decision 5 (enabling, first gateway and
  client, options per adapter with the table of the two meanings of `namespace`, handles and the message body,
  results and acknowledgements, exceptions, lifecycle hooks), status `documented`; verify every Rust sample names
  its file in `examples/socket-io-chat` or is a rustdoc example, and that the page builds with `npm run docs:dev`
- [ ] 3.2 Sections 8 to 12 (authenticating connections with the warning of spec "Reader assuming a global guard
  protects the connection" and the link to route posture, rooms and broadcast, server acknowledgements and timeouts,
  binary payloads and the low-level API, CORS and origins); verify each scenario of the spec requirements "The guide
  covers gateways over Socket.IO end to end" and "The guide states how connections are authenticated" against the
  page
- [ ] 3.3 Sections 13 to 16 (several instances with Redis: feature, `connect` / `connect_lazy`, outages, the trust
  boundary, session affinity; shutdown; testing with the example's raw client and with the official client; see
  also); verify the scenarios of "The guide documents scaling out with the Redis adapter"

## 4. Limits and migration pages (English)

- [ ] 4.1 `docs/websockets/socket-io-limits.md`: one table per layer moved from the preview section (option,
  default, Node's value, what the client observes), "Differences from Node", "Not supported"; verify every default
  against the rustdoc of `EngineOptions`, `IoOptions`, `IoAdapter` and `RedisAdapterOptions` as merged
- [ ] 4.2 Cross-check with `parity/socket-io/expected-differences.json`: list in Implementation notes each entry and
  the row of the page that documents it, and add the missing rows; verify no entry is left without a row
- [ ] 4.3 "How this is verified" on the limits page: the pinned versions read from `parity/socket-io/package.json`
  and from the gateway reference's lockfile, the two protocol suites, the harness and its CI job, and where to
  report a difference; verify the versions against those files
- [ ] 4.4 `docs/websockets/socket-io-migration.md` with the sections of design.md Decision 8; verify every row of
  the server API table has its Node call in `parity/socket-io/reference/*.mjs` (Decision 6) and its Corvis call in
  the example or in a rustdoc example, and each scenario of the spec requirement "Migration page" against the page

## 5. Cross-links, cleanup and CI

- [ ] 5.1 `docs/websockets/adapter.md`: replace the preview section with "Gateway adapters" and rewrite the opening
  paragraph (Decision 5); `gateways.md`: the note in the protocol section and a link from the options table;
  `guards.md`: link the connection sections to the guide; `docs/.vitepress/sidebar.mts`: the three pages in both
  locales; verify `grep -ri "preview" docs/websockets docs/es/websockets` finds no Socket.IO preview
- [ ] 5.2 `docs/migration-guide.md` (a "Socket.IO" entry under "Differences worth knowing" linking the migration
  page), `docs/faq/examples.md` (the `socket-io-chat` section and the example tests paragraph), root `README.md`
  (examples table, the WebSockets row of the equivalence table); verify the README still passes its checks
  (`openspec/specs/docs/readme`: samples compile or are marked as fragments)
- [ ] 5.3 Measure the Linux `test` job with the example in the workspace: confirm from its log whether the workspace
  step now runs the `socket-io` and `socket-io-redis` test files of `crates/corvis`; if it does, remove the two
  explicit feature steps added by #32 and #36 and update the CI table of `CONTRIBUTING.md`, otherwise keep them and
  record why; verify the job is green and no test file that ran before is missing from the log

## 6. Spanish and checks

- [ ] 6.1 Translate the three pages to `docs/es/websockets/` and mirror the edits of 5.1 and 5.2 in `docs/es/`,
  with the same headings, code fences and status, in neutral Spanish without voseo; verify `npm run docs:check` in
  `docs/` passes
- [ ] 6.2 Read both languages in `npm run docs:dev`: the three pages, the sidebar order of Decision 5, the language
  switcher on each page, and every link added in group 5; verify by following each one

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature clippy loop of
  `ci.yml` still passes (the feature-off builds of `corvis`)
- [ ] 7.3 `cargo test --workspace` passes; with `CORVIS_NODE_DIFF=1` and `CORVIS_REDIS_URL` the example's
  `official_client` and `cluster` tests pass, and the strict harness run of #37 still passes
- [ ] 7.4 Regression tests: one test per scenario of `examples/socket-io-chat` (raw client, official client, cluster
  or startup test, as listed in group 2)
- [ ] 7.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`;
  `node scripts/check-naming.mjs` passes
- [ ] 7.6 No framework runtime code changed: record in the archive that the `corvis-parity` check is not needed; if
  any file under `crates/` other than tests changed, run the `corvis-parity` check and verify the three port apps
  keep 0 apidiff differences
- [ ] 7.7 `openspec validate e2-socket-io-docs-and-example --strict` passes without warnings

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `docs/socket-io` and `examples/socket-io-chat` deltas are applied).
- Close issue #38, tick it in epic #30 and move its card to Done (see the `corvis-task` skill); with it the epic's
  task list is complete: close epic #30 if the maintainer agrees.
- Open the follow-up issues of proposal.md "Out of scope" that the maintainer accepts (adapter authoring guide,
  sticky-session recipe, a compose file for the mixed-cluster walkthrough, a shared test-support Socket.IO client).

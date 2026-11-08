# Proposal

Implements [randygil/corvis#23](https://github.com/randygil/corvis/issues/23) (epic
[#13](https://github.com/randygil/corvis/issues/13)). This is a spike: its output is a decision per feature and one
new issue per feature, not code.

## Why

The documentation review found six features that people moving from the original framework expect and that Corvis does
not have. The docs are honest about it (each page says the feature is missing and gives a workaround), but users still
hit these gaps:

- REQUEST-scoped providers (`#[injectable(scope = "request")]` is a compile error).
- Exception filters on WebSocket gateways (`#[use_filters]` on a gateway is a compile error) and global guards that
  protect gateway messages (today global enhancers are HTTP-only).
- TLS in the built-in server (only plain HTTP/1.1 and h2c).
- Response compression (none; compressed request bodies get a 415).
- Async registration (`for_root_async` / `register_async`) for `ConfigModule`, `CacheModule` and `ThrottlerModule`.
- `Retry-After` and `X-RateLimit-*` headers from `ThrottlerGuard`.

Each touches a different part of the framework, so the issue asks to split it before implementing anything.

## What Changes

- `design.md`: one section per feature with the current state (verified in the code), the proposed scope, size,
  dependencies and risks, plus the decisions that apply to all of them.
- `new-issues.md`: six ready-to-file issues (title, body, size, priority, labels, OpenSpec change name), children of
  epic #13, with a suggested place in the execution plan (#111).
- After review, the maintainer files those issues, adds them to epic #13, `ROADMAP.md` and #111, and closes #23.
- No code, docs or spec changes.

## Capabilities

### New Capabilities

None in this change. The follow-up changes will introduce or modify:

- `core/di` (request scope)
- `ws/gateways` (exception filters and global guards on gateways)
- `http/server` (TLS listeners)
- `http/compression` (response compression)
- `core/modules` (async registration of Config, Cache and Throttler)
- `security/rate-limiting` (throttler headers, and the response-header mechanism for enhancers)

### Modified Capabilities

None.

No spec deltas: there is no requirement common to all six features, so each follow-up change writes the deltas for its
own capability. `.openspec.yaml` sets `skip_specs: true` and the change is archived without spec updates.

## Impact

- New files only: `openspec/changes/e13-missing-core-features-users-expect/` (`proposal.md`, `design.md`, `tasks.md`,
  `new-issues.md`).
- GitHub: six new issues under epic #13 (filed by the maintainer from `new-issues.md`), `ROADMAP.md` rows for them,
  and their place in #111.
- No runtime change, so no effect on the port apps.

## Out of scope

- Implementing any of the six features: each new issue becomes its own OpenSpec change.
- Decompressing request bodies (`Content-Encoding: gzip/deflate/br`, today a 415 in
  `crates/corvis/src/http/body_parser.rs:108-117`; Express inflates them). Could be a new issue.
- Global pipes and interceptors on gateways (only guards and filters are named in #23). Could be a new issue, or folded
  into the gateway issue if the maintainer prefers (design.md, Open questions).
- `req.protocol()`, `req.secure()` and `X-Forwarded-Proto` under trust proxy (missing in
  `crates/corvis/src/http/request.rs`, needed for secure cookies behind a proxy). Could be a new issue.
- Throttler features beyond headers: `blockDuration`, `skipIf`, custom trackers, per-name overrides, shared storage
  (Redis). Could be new issues in a later phase.
- Durable providers and context identification strategies (request scope variants of the original).
- TLS extras: certificate hot reload, client certificates (mTLS), ACME.
- The `Scope` doc comment that mentions a `Request` variant (`crates/corvis/src/di/provider.rs:30`): already in #14.

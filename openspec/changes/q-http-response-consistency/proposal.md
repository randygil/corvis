# Proposal

Part of epic [#187](https://github.com/randygil/corvis/issues/187) (code quality audit); implements the issue filed from
`quality/q-http-response-consistency.issue.md`. Source: the engineering-quality review of the HTTP layer, the server and
the multipart module, read against `dev` at `d46758a`. Second of four changes (see `q-http-server-robustness/proposal.md`
for the split); it lands after `openspec/changes/e3-authentication-hot-path-and-consolidation/`, which edits the same
cookie code (`has_cookie_prefix` in `http/res.rs`, the session and OAuth2 callers of `serialize_cookie`), and before
`q-http-request-hot-path`, which edits the same response code.

## Why

This layer promises the observable behavior of the original HTTP stack byte for byte where applications can see it, and
three private applications are compared against their originals after every change. The review found that the promise is
kept twice, by copies that already disagree, and that several public functions panic on input a caller passes at run
time:

- **Two cookie serializers.** `Cookie::try_to_string` (`crates/corvis/src/http/response.rs:273-308`) percent-encodes
  with `NON_ALPHANUMERIC`, writes the attributes in another order, writes no `Expires` for a `max_age`, keeps the
  `SameSite` value as typed and refuses `"true"`; `serialize_cookie` (`http/res.rs:246-302`) follows `res.cookie` of the
  original. The same cookie leaves differently depending on which API set it.
- **Two `encodeurl`** (`http/res.rs:339-379` and `http/serve_static.rs:205-222`) that disagree on `\`, `^`, `|` and on
  malformed `%` escapes; **two `fresh`** (`http/express.rs:137-163` and `http/serve_static.rs:587-612`): the static one
  takes `Cache-Control: no-cache-x` as `no-cache`, and they also disagree on `If-None-Match: *` and on a request that sends
  both conditional headers; two `vary` (`router/cors.rs:331-346` rebuilds the list, `http/compression/mod.rs:309-321`
  keeps the text), two `js_number` (`http/body_parser.rs:159-175` misses `0o` and `0b`), two `parse_http_date`, and
  identical copies of `escape_html` and `latin1`.
- **Panics on run-time input.** `impl Display for Cookie` panics (`response.rs:313-320`), so a `format!` or a log line
  that prints a cookie can bring a worker down; `serialize_cookie` panics on a name or value (`res.rs:246-299`), and the
  `Res` methods `status`, `set_header`, `set_header_values`, `set`, `append`, `type_`, `vary`, `links`, `cookie`,
  `cookie_json`, `clear_cookie`, `send_status` and `redirect_with` panic through `header_name` / `header_value` /
  `StatusCode` (`res.rs:531-542`, `570-575`). Under `panic = "abort"` that is the process.
- **Serialization errors become `200 null`.** `Response::json`, `Res::json` and `MessageEvent::new` turn a value that does
  not serialize (a map with non-string keys, a `Serialize` that fails) into `null` (`response.rs:98`, `res.rs:793`,
  `sse.rs:30`), while `Json<T>` returns the error. Found while confirming: every typed handler result is deep-copied once
  for nothing, because `SerializeTag::convert` passes an already built `Value` to `Response::json`, which serializes it
  again (`__private.rs:285-293`).
- **`IntoStatus for i32` truncates** (`http/exception.rs:351-355`): `65736` becomes `200 OK`.

## What Changes

- **A recorded fixture decides.** `crates/corvis/tests/fixtures/node_compat/` (`package.json` pinning the original
  packages at the versions the original framework's express 5.3.0 resolves, `generate.mjs`, `node_compat.json`) records
  `res.cookie` / `res.clearCookie` headers or errors, `encodeurl`, `fresh`, the conditional answers of the original static
  middleware, `vary`, `escape-html` and `Number()` over tables of inputs. Corvis replays every case and matches it, or the
  case carries a `divergence` with its reason.
- **One implementation of each helper**, in a crate-private `http::node_compat` module (`encode_url`, `fresh`,
  `vary_append`, `escape_html`, `http_date` / `parse_http_date`, `latin1`, `js_number`), replacing every copy; the cookie
  serializer stays `http::res::serialize_cookie` and `Cookie` is built on it.
- **Errors instead of panics.** `serialize_cookie` returns `Result<String, CookieError>`; `Cookie` loses `Display` and
  keeps `try_to_string`; `Response::with_cookie` / `set_cookie` return `Result`; the validating `Res` methods return
  `Result<&Self, ResError>` (or `Result<(), ResError>`), so a migrated handler writes `res.cookie(..)?` where the original
  threw. `CookieError` and `ResError` are errors whose `Display` is the original `TypeError` / `RangeError` text, and `?`
  turns them into the generic `500`, the answer a panic gives today.
- **Cookie attributes as types.** `SameSite` (moved from `auth::session`, re-exported there) and `CookiePriority`
  replace the strings of `CookieOptions` and `Cookie`.
- **No silent `null`.** `Response::json` and `MessageEvent::new` take `impl Into<Value>` (they cannot fail);
  `Response::try_json`, `MessageEvent::try_new` and `Res::json` take `impl Serialize` and return the error. The typed
  handler result is no longer copied twice.
- **Statuses.** `IntoStatus for i32` converts through `u16::try_from`; an out-of-range code is a `500`, as an invalid
  `u16` already is, and the rustdoc says so.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `http/cookies`: the purpose and the max-age requirement now cover the whole header; added requirements for one
  serialization behind both APIs, verified against the original, and for cookie errors as values.
- `http/responses`: added requirement that a value which cannot be serialized is never answered as `null`.
- `http/static-files`: added requirement that conditional requests follow the original freshness rule.
- `http/exceptions`: added requirement for run-time status codes outside the valid range.

## Impact

- New: `crates/corvis/src/http/node_compat.rs`, `crates/corvis/tests/fixtures/node_compat/{package.json,
  package-lock.json,generate.mjs,node_compat.json}`, `crates/corvis/tests/node_compat.rs`.
- Edited: `crates/corvis/src/http/{response,res,express,serve_static,sse,exception,body_parser,mod}.rs`,
  `crates/corvis/src/http/compression/{mod,negotiate}.rs`, `crates/corvis/src/router/cors.rs`,
  `crates/corvis/src/multer/busboy.rs` (`latin1`), `crates/corvis/src/__private.rs`,
  `crates/corvis/src/auth/session/{options,manager,mod}.rs`, `crates/corvis/src/auth/oauth2/state.rs`, the tests that
  call the changed APIs (`cookie_max_age.rs`, `static_response_security.rs`, `res.rs`, `guard_responses.rs`,
  `signed_cookies.rs` and others found by the build), and in `docs/` and `docs/es/`: `http/cookies.md`,
  `recipes/express-migration.md`, `controllers.md`, `http/server-sent-events.md`, `security/csrf.md`.
- **Public API, breaking, before 1.0 on purpose:** `serialize_cookie`, `Cookie::try_to_string` (typed error),
  `Response::with_cookie` / `set_cookie`, the validating `Res` methods and `Res::json` return `Result`; `Cookie` has no
  `Display`; `CookieOptions::{same_site, priority}` and `Cookie::same_site` hold enums; `Response::json` and
  `MessageEvent::new` take `impl Into<Value>`. Additions: `CookieError`, `ResError`, `CookiePriority`,
  `http::SameSite`, `Response::try_json`, `MessageEvent::try_new`.
- **The port apps:** two call sites need a `?` (`consi-backend/src/settlement/settlement.controller.rs:37-40`,
  `houseplan-api/src/http/auth.controller.rs:23`); task 8.3 makes that edit in the port repository before the parity
  check. The `Response::json` calls of the ports pass a `Value` and compile unchanged.
- Behavior visible to applications and clients: design.md "Behavior visible to applications". The ones that matter: a
  `Cookie` set through `Response::with_cookie` is serialized like `res.cookie` (encoding, attribute order, `Expires`,
  `SameSite` case); static files honor `Cache-Control` exactly like the original; a value that does not serialize is a
  `500`, not `200 null`.
- No new dependency. The fixture package pins the original packages it records.

## Out of scope

- The same silent `unwrap_or(Value::Null)` outside the HTTP layer (`websockets.rs:98,160,207,262,487,515`,
  `modules/cache.rs:88`): recorded as a follow-up for their own modules.
- `Response::set_header` and `with_header` ignoring an invalid name or value: kept (a builder that drops what it cannot
  write), now documented in their rustdoc.
- Fewer copies when finishing a response and the request-side costs: `q-http-request-hot-path`.

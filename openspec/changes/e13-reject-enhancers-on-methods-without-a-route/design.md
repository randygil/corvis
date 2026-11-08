# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code:

**`#[routes]` (`crates/corvis-macros/src/controller.rs`)**

- `expand_routes` (`controller.rs:174`) walks every `ImplItem::Fn`. For each one it drains `method.attrs`
  (`:184-244`): a name found in `METHODS` (`:146-157`: `get`, `post`, `put`, `delete`, `patch`, `options`, `head`,
  `all`, `search`, `sse`) sets `route` (`:186-190`); `http_code`, `header`, `redirect`, `use_guards`,
  `use_interceptors`, `use_pipes`, `use_filters`, `set_metadata` and `version` push builder calls (`:192-238`);
  anything else goes through `decorator_tokens` (`:239-242`), which consumes `apply_decorators` and every name starting
  with `api_` (`:34-51`) and keeps the rest.
- Only after the loop does it check the route: `method.attrs = keep; let Some((variant, paths, is_sse)) = route else {
  continue };` (`:245-246`). The builder calls collected for that method are dropped with it, and the attributes are
  already gone from the method, so the compiler never sees them.
- A second route attribute overwrites the first (`route = Some(..)` at `:188`; Out of scope).
- Errors are returned with `?` / `return Err(..)`, so the first error in the block stops the expansion
  (`:193`, `:200`, `:256`).

**`#[handlers]` (`crates/corvis-macros/src/handlers.rs`)**

- `expand` (`handlers.rs:77`) drains `method.attrs` (`:89-116`): `on_event`, `cron`, `interval`, `timeout` and
  `subscribe_message` set `kind` (`:97-98`); `use_guards`, `use_interceptors` and `use_pipes` (`:91-96`) and
  `set_metadata` (`:105-112`) push `WsEnhancers` builder calls and record the first such attribute in
  `enhancer_attr` (`:104`, `:112`); anything else is kept (`:114`). `use_filters` is not consumed today, so it is kept
  and fails as an unknown attribute (#116 adds it).
- `let Some((kind, args)) = kind else { continue };` (`:118`) skips a method without a handler attribute, dropping its
  enhancers. Right after, enhancers on a method whose kind is not `subscribe_message` are already an error
  (`:119-123`), spanned on `enhancer_attr`.

**Misplaced-attribute stubs and re-exports**

- `misplaced!` (`crates/corvis-macros/src/lib.rs:176-200`) defines `get`, `post`, `put`, `patch`, `delete`,
  `use_guards`, `use_interceptors`, `use_pipes` and `use_filters` as attribute macros that emit `compile_error!`.
- `corvis` re-exports only `Validate`, `controller`, `handlers`, `injectable`, `main`, `module`, `routes`, `test` and
  `websocket_gateway` from `corvis_macros` (`crates/corvis/src/lib.rs:137-139`), so the stubs only resolve when a user
  imports `corvis_macros` directly. Inside `#[routes]` / `#[handlers]` the attributes are removed by name before name
  resolution, which is why they need no import there and why dropping them leaves no error at all.

**Tests and doctests**

- `corvis-macros` has no unit tests (`grep '#\[test\]' crates/corvis-macros/src` finds none) and no dev-dependencies
  (`crates/corvis-macros/Cargo.toml`). `expand_routes` and `handlers::expand` take `syn::ItemImpl` and return
  `syn::Result<proc_macro2::TokenStream>`, so they can be unit-tested without the compiler plugin interface.
- No `compile_fail` doctest or `trybuild` test exists in the workspace. The only `#[cfg(doctest)]` item is
  `ReadmeDoctests` (`crates/corvis/src/lib.rs:72-75`), which compiles the README's Rust blocks with
  `cargo test --doc -p corvis`.
- `crates/corvis/tests/schedule.rs:288-305` shows the existing pattern for a duplicate-name check at startup; there is
  no compile-time equivalent to copy.

**Existing code**

- A scan of every `.rs` file under `crates/`, `examples/` and the three port apps
  (`C:\Users\Usuario\Devel\corvis-tests\corvis\{consi-backend,facten-api,houseplan-api}`) for methods inside `#[routes]`
  or `#[handlers]` blocks that carry a consumed attribute but no route or handler attribute found none. The change
  should not break any code in the repository or the ports.

**Related changes**

- #26 (`e1-public-metadata-and-startup-check`, design decision 3) adds `#[public]` to `expand_routes` and an error for
  `#[public]` on a method without a route attribute, checked before the `continue` at `controller.rs:246`, "only for
  `#[public]`"; its proposal lists the other attributes as out of scope (this issue).
- #27 (`e1-route-posture-for-websockets-and-sse`, design decision 6) does the same for `#[public]` in `#[handlers]`.
- #116 (`e13-exception-filters-and-global-guards-on-gateways`, decision 1) adds `"use_filters" => Some("filter")` to
  the `handlers.rs` match.

## Goals / Non-Goals

**Goals:**
- No attribute that `#[routes]` or `#[handlers]` consumes can disappear without a compile error.
- The error tells the user which method and which attribute, and what is missing.
- Future consumed attributes get the check for free.

**Non-Goals:**
- Friendlier messages for attributes that already fail to compile (parameter attributes, misplaced stubs).
- Rejecting two route or handler attributes on one method (Out of scope in proposal.md).
- Any runtime change.

## Decisions

1. **Error when a method has consumed attributes and no route/handler attribute, in the macro.** While draining
   attributes, each macro records the first consumed attribute that is not a route/handler attribute (name and the
   `syn::Attribute` for the span). In `expand_routes` this is any branch of the `match` at `controller.rs:191-243`
   except the `keep` fallback; in `handlers::expand` it is the existing `enhancer_attr` (`handlers.rs:88`). At the
   `continue` points (`controller.rs:246`, `handlers.rs:118`), a recorded attribute produces a `syn::Error` instead of
   a silent skip.
   Alternatives considered: (a) keep the attributes on the method so the compiler reports "cannot find attribute".
   Rejected: the message does not say a route attribute is missing, and with `corvis_macros` imported directly the
   `misplaced!` stub would say "must be placed below #[controller] or on a method inside `#[routes]`", which is
   exactly where the attribute already is. (b) A warning instead of an error. Rejected: proc macros cannot emit
   warnings on stable Rust without tricks (deprecated items), and the issue asks for an error. (c) Check at startup.
   Rejected: nothing reaches the runtime; the information only exists at expansion time.
2. **Which attributes count.** In `#[routes]`: everything the macro consumes on a method today (`http_code`,
   `header`, `redirect`, `use_guards`, `use_interceptors`, `use_pipes`, `use_filters`, `set_metadata`, `version`,
   `apply_decorators`, `api_*`). The issue names the enhancers "and similar attributes"; the response attributes,
   version and decorators are dropped the same way, and a silently ignored `#[api_security]` or `#[version("2")]` is
   as misleading as a guard. In `#[handlers]`: the attributes that set `enhancer_attr` (enhancers and
   `set_metadata`). The record happens in the same branch that consumes the attribute, so #26's and #27's
   `#[public]` and #116's `use_filters` are covered as soon as they are consumed there.
   Alternative considered: enhancers only (`use_*`). Rejected: leaves `#[set_metadata(IS_PUBLIC_KEY, ..)]`,
   `#[version]` and the response attributes silently dropped, for no saving in code.
3. **Message wording.** `#[routes]`: `` `<method>` has #[<attr>] but no route attribute (#[get], #[post], #[put],
   #[patch], #[delete], #[options], #[head], #[search], #[all] or #[sse]); add one or remove #[<attr>] ``.
   `#[handlers]`: `` `<method>` has #[<attr>] but no #[subscribe_message] attribute; enhancers and metadata in
   `#[handlers]` only apply to message handlers ``. `<attr>` is `attr_name` (the last path segment, so
   `#[corvis::use_guards]` reads `#[use_guards]`). The route list is built from `METHODS` so it cannot drift. The
   `#[handlers]` text points to `#[subscribe_message]` only, because enhancers on the other handler kinds are already
   rejected (`handlers.rs:119-123`).
   Alternative considered: one shared text ("... has no route or message attribute"). Rejected: the two macros accept
   different attributes, and naming the right one is the point of the message.
4. **One error per offending method, all reported together.** The error is spanned on the first consumed attribute
   of the method; errors from several methods are merged with `syn::Error::combine` and returned after the loop, so a
   user sees them all in one build. Other errors (bad arguments, missing `&self`) keep returning early as today.
   Alternative considered: one error per offending attribute. Rejected: noisy for a method with three enhancers, and
   the fix (add the route attribute) is the same for all of them.
5. **Tests: unit tests for the text, `compile_fail` doctests for the user view.** Unit tests in a `#[cfg(test)] mod
   tests` at the bottom of `controller.rs` and `handlers.rs` parse an `ItemImpl` with `syn::parse_quote!`, call
   `expand_routes` / `expand`, and assert the exact `to_string()` of the error (and the two messages of the combined
   case, through `syn::Error::into_iter`). The `compile_fail` doctests live in a new `crates/corvis/src/doctests.rs`,
   declared in `lib.rs` as `#[cfg(doctest)] mod doctests;` next to `ReadmeDoctests`: one doc-commented item per case,
   each with a ` ```compile_fail ` block and a ` ``` ` block that is identical except for the route or handler
   attribute, so the passing twin shows the failure comes from the missing attribute. The `#[handlers]` doctest uses a
   plain `#[injectable]` provider, so it does not depend on the `websockets` feature.
   Alternatives considered: `trybuild` UI tests, which also check the message. Rejected for now: a new
   dev-dependency and a separate build directory per run, and the unit tests already pin the text. Doctests in
   `corvis-macros` itself: rejected because they would need `corvis` as a dev-dependency of the crate it depends on.
6. **Coordination with #26 and #27.** If #26 or #27 lands first, their `#[public]`-only checks are folded into this
   one (same place, same message format) and their tests are updated to the new text. If this change lands first,
   #26 and #27 only need to consume `#[public]` in the existing branch to get the check.

## Risks / Trade-offs

- [Code that compiles today stops compiling] → the dropped attributes never had an effect, so every such case is a bug
  or dead code; the scan found none in the repository or the ports; the docs list the error and its fix.
- [A `compile_fail` doctest can pass for the wrong reason (any compile error)] → each has a passing twin that differs
  only by the route/handler attribute, and the unit tests pin the exact messages.
- [Third-party attribute macros whose name starts with `api_` on helper methods] → they are already consumed (and
  turned into `.decorate(api_x!(..))` calls) by `decorator_tokens` on every method; on a helper method they were
  dropped silently and now fail. Acceptable: that prefix is reserved for decorators by the current macro.

## Open questions

1. **Scope of the check in `#[routes]`.** This change rejects every consumed attribute (Decision 2), not only the
   enhancers the issue names. Confirm, or restrict to `use_*` and `set_metadata`.
2. **#26/#27 message for `#[public]`.** #26 proposed "`#[public]` must be on a route method (with `#[get]`,
   `#[post]`, ...)" for this case. This change would replace it with the shared format, which also names the method.
   Confirm, or keep the #26 text as a special case.

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- Broad scope confirmed: every attribute the macro consumes and would drop, not only `#[use_*]` and `#[set_metadata]`.
  A silently dropped `#[http_code]` or `#[api_*]` is the same bug.
- `#[public]` (#26/#27) uses this common message format, which names the method; that change adopts it when it lands.
- Check during implementation: in `#[handlers]`, a method with `#[cron]`, `#[interval]`, `#[timeout]` or `#[on_event]`
  plus an enhancer also loses the enhancer. The rule is "enhancers need `#[subscribe_message]`", so the error must
  fire for those methods too, and the requirement text must say "no `#[subscribe_message]`" rather than "no handler
  attribute". Align the spec delta with whatever the code does today before writing the tests.
- Spec requirements were split so each states one behavior (validation limit); the folder was renamed to
  `e13-reject-enhancers-on-methods-without-a-route` (issue #124).

# Proposal

Implements [randygil/corvis#124](https://github.com/randygil/corvis/issues/124) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Found while drafting the change for #27 (`e1-route-posture-for-websockets-and-sse`).

## Why

`#[routes]` and `#[handlers]` remove every attribute they understand from a method before they check whether the
method is a route or a handler. A method without a route attribute (`#[get]`, `#[post]`, ...) or a handler attribute
(`#[subscribe_message]`, `#[on_event]`, ...) is then skipped, and the attributes that were removed go with it. So this
compiles and does nothing:

```rust
#[routes]
impl AdminController {
    #[use_guards(AdminGuard)] // dropped: the method has no route attribute
    async fn purge(&self) -> HttpResult<()> { /* ... */ }
}
```

A user who forgot the verb attribute, or who wrote it on the wrong method, believes `AdminGuard` protects something.
The same happens to `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]`, `#[http_code]`,
`#[header]`, `#[redirect]`, `#[version]`, `#[apply_decorators]` and the OpenAPI `#[api_*]` attributes in `#[routes]`,
and to `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` and `#[set_metadata]` in `#[handlers]`. Silently dropping
a guard is the kind of mistake route posture (#24) exists to prevent, and it cannot be caught at startup because the
attribute leaves no trace in the compiled code.

## What Changes

- In a `#[routes]` block, a method that carries any attribute `#[routes]` consumes (enhancers, metadata, response
  attributes, version, decorators) but no route attribute is a compile error. The message names the method and the
  attribute and says that a route attribute is missing.
- In a `#[handlers]` block, a method that carries an enhancer or `#[set_metadata]` but no handler attribute is a
  compile error. The message names the method and the attribute and says that `#[subscribe_message]` is missing,
  since enhancers in `#[handlers]` only apply to message handlers.
- The check covers every attribute each macro consumes through one list per macro, so attributes added later
  (`#[public]` from #26 and #27, `#[use_filters]` in `#[handlers]` from #116) are covered without extra code.
- All such errors in one `impl` block are reported together.
- Unit tests in `corvis-macros` check the exact messages; `compile_fail` doctests in `corvis` show both errors from a
  user's point of view, each paired with a passing doctest that differs only by the missing attribute.
- Docs (EN + ES): the "Attribute compile errors" table in `faq/common-errors.md`, plus one sentence in `controllers.md`
  and `websockets/gateways.md`.

No runtime code changes: methods with a route or handler attribute expand exactly as before.

## Capabilities

### New Capabilities

- `core/enhancers`: compile-time rejection of enhancer and route attributes that would be discarded because the
  method has no route or handler attribute. (`e13-enhancers-and-routing-edge-cases`, #17, also creates
  `core/enhancers`; whichever change is archived first creates the spec and the other adds its requirements.)

### Modified Capabilities

None.

## Impact

- `crates/corvis-macros/src/controller.rs` (`expand_routes`), `crates/corvis-macros/src/handlers.rs` (`expand`).
- `crates/corvis/src/lib.rs` (a `#[cfg(doctest)]` module) and a new `crates/corvis/src/doctests.rs` holding the
  `compile_fail` doctests.
- Docs: `faq/common-errors.md`, `controllers.md`, `websockets/gateways.md` and their Spanish translations under
  `docs/es/`.
- Users: code that compiles today and has one of these attributes on a method without a route or handler attribute
  stops compiling. That code never did what it said. A scan of `crates/`, `examples/` and the three port apps found no
  such method (design.md, Context).
- Port apps: no change expected; the parity check is not required because no runtime code changes (see tasks.md,
  Definition of done).

## Out of scope

- Two route attributes on one method (`#[get("a")] #[post("b")]`): the last one wins silently
  (`controller.rs:188`); possible new issue.
- Two handler attributes on one method (`#[on_event("a")] #[interval(1000)]`): the last one wins silently
  (`handlers.rs:97-98`); possible new issue.
- Parameter attributes (`#[body]`, `#[param]`, `#[message_body]`, `#[connected_socket]`) on a method without a route
  or handler attribute: already a compile error ("cannot find attribute"), only the message could be friendlier;
  possible new issue.
- `#[http_code]` and `#[redirect]` without arguments make the macro panic (`a[0]` at `controller.rs:194` and `:207`)
  instead of returning a spanned error; possible new issue.
- Re-exporting the `misplaced!` stubs from `corvis` so that `#[use_guards]` outside `#[controller]` / `#[routes]` shows
  its own message instead of "cannot find attribute": #26 does it for `#[public]` only; possible new issue for the rest.
- Enhancers on methods with `#[on_event]`, `#[cron]`, `#[interval]` or `#[timeout]`: already a compile error
  (`handlers.rs:119-123`), unchanged.
- `#[use_filters]` on gateways and message handlers: #116.

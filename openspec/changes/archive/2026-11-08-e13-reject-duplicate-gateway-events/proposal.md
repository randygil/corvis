# Proposal

Implements [randygil/corvis#125](https://github.com/randygil/corvis/issues/125) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Found while drafting the change for #27 (`e1-route-posture-for-websockets-and-sse`), which lists it as out of scope.

## Why

When one gateway has two `#[subscribe_message("x")]` handlers, `build_gateways` stores them in a map keyed by event
name and the later one replaces the earlier one. Nothing is logged except the usual "subscribed to the "x" message"
line, printed once per handler, so the log even suggests both are active. The earlier handler never runs.

This is a security problem as much as a correctness one. If the earlier handler carries `#[use_guards(AdminGuard)]`
and the later one has no guard, every client reaches the unguarded one, while the source code still shows a guarded
handler. Route posture (#27) checks each handler's guards, so it would also report the shadowed handler as protected.

The issue asks to check what the original does. Its behavior depends on the adapter and is silent in both cases
(source read on 2026-11-08, see design.md, Context): the `ws` adapter keeps the last handler (the same as Corvis
today), and the socket.io adapter runs both handlers for one event, so a client can get two replies or two acks.
There is no single original behavior to match, neither is something a user would choose on purpose, and following
the project rule "security over parity", Corvis rejects duplicates.

## What Changes

- **Behavior change:** `init()` (and therefore `listen`) fails when a gateway declares more than one
  `#[subscribe_message]` handler for the same event name. The error names the gateway, the event and both handler
  methods. The check runs before that gateway is registered or its `after_init` hook runs.
- Event names are compared exactly, as frames are dispatched (`"message"` and `"Message"` are different events).
  Different gateways may still handle the same event name.
- A regression test in `crates/corvis/tests/websockets.rs`.
- Docs (EN + ES): `websockets/gateways.md` (one handler per event) and `migration-guide.md` (how the original's
  adapters behave and why Corvis refuses to start).

## Capabilities

### New Capabilities

- `ws/gateways`: one handler per event name in a gateway, checked at startup.
  (`e13-exception-filters-and-global-guards-on-gateways`, #116, also creates `ws/gateways`; whichever change is
  archived first creates the spec and the other adds its requirement.)

### Modified Capabilities

None.

## Impact

- `crates/corvis/src/websockets.rs` (`build_gateways`).
- `crates/corvis/tests/websockets.rs`.
- Docs: `websockets/gateways.md`, `migration-guide.md` and their Spanish translations under `docs/es/`.
- Users: an application with duplicate events in one gateway stops starting after the upgrade, with a message that
  says which handlers collide. Under the `ws` adapter of the original the earlier handler was dead code, so removing
  it keeps the behavior; under socket.io both ran, so the user merges them into one handler.
- Examples: `examples/cats-app/src/chat.rs` and `examples/tertulia/src/chat/chat_gateway.rs` have no duplicate events.
- Port apps: none of consi, facten or houseplan has a gateway, so apidiff should not move; the parity check confirms
  it.

## Out of scope

- Two gateways on the same path: the second replaces the first in `out.insert(info.path.clone(), ..)`
  (`crates/corvis/src/websockets.rs:591-594`) and in `SERVERS` (`:585`), also silently; possible new issue.
- Several `#[on_event]` handlers for the same event in one provider: that is the expected fan-out of the event
  emitter, not a duplicate.
- Duplicate `#[cron]` / `#[interval]` / `#[timeout]` names: already a startup error
  (`crates/corvis/src/modules/schedule/mod.rs:286-289`).
- HTTP routes that collide in the router only log an error (`RouteTable::insert`,
  `crates/corvis/src/router/mod.rs:188-190`) and the application starts; possible new issue.
- A compile-time check for duplicate literal event names inside one `#[handlers]` block (design.md, Decision 2).
- The socket.io adapter (#35): it reuses this check; whether it should instead allow several handlers per event, as
  socket.io does, is left to that change (design.md, Open questions).

# Spec Delta

## Purpose

Which global enhancers apply to WebSocket gateway handlers, in which order, and how an application keeps them
HTTP-only.

## ADDED Requirements

### Requirement: Global guards, interceptors and pipes apply to gateway handlers
By default the framework SHALL apply the application's global guards, interceptors and pipes to every
`#[subscribe_message]` handler, whether they were registered with `use_global_guards`, `use_global_interceptors`,
`use_global_pipes` or with `Provider::app_guard`, `app_interceptor`, `app_pipe`, as the original framework does in a
non-hybrid application.

#### Scenario: Global guard rejects a message
- **WHEN** the application registers `Provider::app_guard::<DenyGuard>()`, `DenyGuard` returns `false`, and a client
  sends `{ "event": "join", "data": "lobby" }` to a gateway without its own guards
- **THEN** the handler does not run and the client receives
  `{ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }`

#### Scenario: Guard tells transports apart
- **WHEN** a global guard returns `true` when `ctx.get_type() == ContextType::Ws` and `false` otherwise
- **THEN** HTTP routes answer 403 and gateway messages reach their handlers

#### Scenario: Gateway marked public through metadata
- **WHEN** a global guard skips targets with `#[set_metadata(IS_PUBLIC_KEY, true)]` and the gateway declares that
  metadata below `#[websocket_gateway]`
- **THEN** messages to the gateway reach their handlers without credentials

#### Scenario: Global pipe transforms a message body
- **WHEN** the application calls `use_global_pipes(TrimPipe)` and a handler takes `#[message_body] text: String`
- **THEN** the handler receives the trimmed text

### Requirement: Global enhancers run before the gateway's and the handler's
Gateway handlers SHALL order enhancers as HTTP routes do: global ones first (provider-registered ahead of
`use_global_*`), then the gateway's, then the handler's. Global pipes MUST run on every `#[message_body]` argument
before the gateway's, handler's and parameter's pipes.

#### Scenario: Global guard runs before the gateway's guards
- **WHEN** the application calls `use_global_guards(A)` and the gateway declares `#[use_guards(B)]`, both returning
  `true` and recording their call
- **THEN** `A` is recorded before `B` for every message

### Requirement: Global exception filters do not apply to gateway handlers
The framework SHALL NOT apply filters registered with `use_global_filters` or `Provider::app_filter` to gateway
handlers; only handler and gateway filters and the built-in mapping take part, as in the original framework.

#### Scenario: HTTP-only global filter
- **WHEN** the application registers `Provider::app_filter::<AllFilter>()` and a gateway handler fails
- **THEN** `AllFilter` is not called and the client receives the built-in `exception` frame

### Requirement: Applications can keep global enhancers HTTP-only
The framework SHALL provide `CorvisApplication::apply_global_enhancers_to_gateways(bool)`, `true` by default. When set
to `false` before the application is initialized, gateway handlers MUST run only their gateway's and their own
enhancers, as before this change.

#### Scenario: Switch turned off
- **WHEN** the application registers `Provider::app_guard::<DenyGuard>()`, calls
  `app.apply_global_enhancers_to_gateways(false)` before `init()`, and a client sends a message
- **THEN** the handler runs and HTTP routes still answer 403

### Requirement: The switch has no effect after initialization
Calling `apply_global_enhancers_to_gateways` after the application was initialized SHALL NOT change the gateways
already built and MUST log a warning saying the call has no effect.

#### Scenario: Switch called too late
- **WHEN** `apply_global_enhancers_to_gateways(false)` is called after `init()`
- **THEN** a warning is logged and global guards keep applying to gateway handlers

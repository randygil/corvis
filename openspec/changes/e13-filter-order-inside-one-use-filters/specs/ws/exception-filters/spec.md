# Spec Delta

## MODIFIED Requirements

### Requirement: Gateway filters are tried handler first, then gateway
When a guard, interceptor, pipe or handler of a message fails, including a guard that returns `false` and a handler
that panics, the framework SHALL try the handler's filters and then the gateway's filters, and use the first one whose
`catches` returns `true`. Within one level, filters MUST be tried in the same order HTTP routes use for filters declared
at one level: the filters of one `#[use_filters(...)]` attribute from the last listed to the first, and the filters of
an upper `#[use_filters]` attribute before those of the attributes below it. This is the order of the original
framework, verified with the gateway differential fixture.

#### Scenario: Handler filter wins over gateway filter
- **WHEN** a gateway declares `#[use_filters(GatewayFilter)]`, a handler declares `#[use_filters(HandlerFilter)]`,
  both catch every exception, and the handler returns an error
- **THEN** `HandlerFilter` builds the frame and `GatewayFilter` is not called

#### Scenario: Guard refusal goes through the filters
- **WHEN** a handler guard returns `false` and the handler has a filter that catches every exception
- **THEN** the filter receives the `Forbidden resource` exception and builds the frame

#### Scenario: Two filters in one handler attribute
- **WHEN** a `#[subscribe_message]` method declares `#[use_filters(First, Second)]`, both filters catch every
  exception, and the handler returns an error
- **THEN** `Second` builds the frame and `First` is not called

#### Scenario: Two filters in one gateway attribute
- **WHEN** a gateway declares `#[use_filters(First, Second)]` below `#[websocket_gateway]`, the handler declares no
  filter, both filters catch every exception, and the handler returns an error
- **THEN** `Second` builds the frame

#### Scenario: Stacked attributes on a handler
- **WHEN** a handler declares `#[use_filters(A, B)]` and, on the next line, `#[use_filters(C)]`, and only `A` and `C`
  catch the raised exception
- **THEN** `A` builds the frame

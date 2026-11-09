# Spec Delta

## ADDED Requirements

### Requirement: Route and controller filters are tried in the original framework's order
For an HTTP route, the framework SHALL try exception filters in this order and use the first one whose `catches`
returns `true`: the method's filters, then the controller's filters, then the global filters (in the order of the
requirement "Global exception filters are tried last registered first"). Within one level, the filters of one
`#[use_filters(...)]` attribute MUST be tried from the last listed to the first, and when several `#[use_filters]`
attributes are stacked on the same method or controller, the filters of the upper attribute MUST be tried before
those of the attributes below it. The array form `#[use_filters([A, B])]` MUST behave as `#[use_filters(A, B)]`. This
is the order of the original framework, verified with a differential fixture.

#### Scenario: Two filters in one method attribute
- **WHEN** a route declares `#[use_filters(First, Second)]`, both filters catch every exception, and the handler fails
- **THEN** `Second` answers the request and `First` is not called

#### Scenario: Two filters in one controller attribute
- **WHEN** a controller declares `#[use_filters(First, Second)]` below `#[controller]`, its route declares no filter,
  both filters catch every exception, and the handler fails
- **THEN** `Second` answers the request

#### Scenario: Stacked attributes
- **WHEN** a route declares `#[use_filters(A, B)]` and, on the next line, `#[use_filters(C)]`, and the three filters
  catch the exception
- **THEN** the filters are tried in the order `B`, `A`, `C`, so `B` answers

#### Scenario: Method level before controller level
- **WHEN** a controller declares `#[use_filters(C1, C2)]`, its route declares `#[use_filters(M1, M2)]`, and only `M1`
  and `C2` catch the raised exception
- **THEN** `M1` answers the request

#### Scenario: Array form
- **WHEN** a route declares `#[use_filters([First, Second])]`, both filters catch every exception, and the handler
  fails
- **THEN** `Second` answers the request

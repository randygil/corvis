# http/trust-proxy Specification

## Purpose
Guarantees that resolving the client address behind proxies (`X-Forwarded-For` with a trust setting) cannot be
crashed or inflated by client-controlled headers.

## Requirements

### Requirement: Trust settings parse without crashing
Parsing a trust proxy setting (address lists, subnets, named ranges such as `loopback`) SHALL return a setting or an
error for any string, and MUST NOT crash.

#### Scenario: Malformed trust list
- **WHEN** a trust setting such as `10.0.0.0/99, ::1/, zz` is parsed
- **THEN** parsing returns an error or a setting, without a panic

### Requirement: Forwarded addresses only add hops
The resolved address chain SHALL start with the socket address and contain at most one entry per address listed in
the `X-Forwarded-For` headers, for any trust setting and any header contents.

#### Scenario: Garbage in X-Forwarded-For
- **WHEN** a request carries several `X-Forwarded-For` headers with malformed and valid addresses
- **THEN** the chain starts with the socket address and has no more entries than forwarded addresses plus one

# infra/auth-dependencies Specification

## Purpose
Policy for the cryptographic crates the authentication features pull into an application: which they are, that there
is one generation of each, that known advisories reach the build, and where randomness comes from.

## Requirements

### Requirement: Crypto crate inventory
The threat model SHALL list, per auth cargo feature, every crate that implements a cryptographic primitive, a
password hash, a token format, TLS or randomness: its version, what it is used for, the features enabled, and its
maintenance status at the time of the review.

#### Scenario: Reader checking what a feature adds
- **WHEN** a reader looks up `auth-local` in the inventory
- **THEN** it lists the password hashing crates and their primitive crates with versions and enabled features

#### Scenario: Inventory out of date
- **WHEN** a listed crate changes its major version in `Cargo.lock`
- **THEN** the source check test fails and names the crate

### Requirement: One generation of each primitive
`cargo deny check` SHALL fail when the dependency graph with all features has two versions of a crate in the
single-generation list of `deny.toml`. The list MUST contain every inventoried crate that has one version when this
change is implemented; a crate that already has two is recorded in the threat model with the reason.

#### Scenario: Second version of a comparison crate
- **WHEN** a dependency upgrade brings a second version of the constant-time comparison crate
- **THEN** the `deny` job fails

### Requirement: No native TLS or foreign crypto stack
`deny.toml` SHALL ban `openssl`, `openssl-sys` and `native-tls`, so the provider client, TLS termination and token
verification keep one reviewed stack.

#### Scenario: Dependency that pulls a native TLS stack
- **WHEN** a new dependency enables a native TLS backend
- **THEN** the `deny` job fails and names the banned crate

### Requirement: Advisories cover the auth features
The advisory checks (`cargo deny check` and `cargo audit`) SHALL evaluate the graph with every auth feature enabled.
An ignored advisory that affects an inventoried crate MUST have an entry in the threat model with the reason and the
issue that removes it.

#### Scenario: Advisory for a password hashing crate
- **WHEN** an advisory is published for a crate of the inventory
- **THEN** the `deny` and `audit` jobs fail until the crate is upgraded or the exception is written with its
  threat-model entry

### Requirement: Secrets come from the operating system generator
Session identifiers, OAuth2 state values, PKCE verifiers and password salts SHALL be generated from the operating
system's random generator. A source check test MUST fail when a file of the authentication modules uses another
generator.

#### Scenario: Thread-local generator introduced
- **WHEN** a change makes the session module generate identifiers with a thread-local or seeded generator
- **THEN** the source check test fails and names the file and line

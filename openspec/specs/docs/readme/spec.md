# docs/readme Specification

## Purpose
Defines what the repository's root `README.md` must contain and guarantee, so that the first page a reader sees on
GitHub or crates.io is in English, accurate, and leads into the documentation site.

## Requirements

### Requirement: The README is in English
The root `README.md` SHALL be written in English: prose, headings, tables, and the comments and user-facing strings in
its code examples. Code identifiers, attribute names, crate names and commands stay as in the code.

#### Scenario: Reading the README
- **WHEN** a reader opens `README.md` on GitHub
- **THEN** every heading and paragraph is in English

### Requirement: The README leads to both languages of the documentation site
The README SHALL link to the documentation site's English root (`https://corvis.randygil.com`) and its Spanish
translation (`https://corvis.randygil.com/es/`), and every topic it summarizes SHALL link to the docs page that covers
it in detail.

#### Scenario: Looking for Spanish documentation
- **WHEN** a Spanish-speaking reader opens the README
- **THEN** near the top there is a link to `https://corvis.randygil.com/es/`

#### Scenario: Following a summarized topic
- **WHEN** a reader reaches the security summary in the README
- **THEN** it links to the docs pages that list the security behavior in full

### Requirement: README Rust examples compile or are marked as fragments
Every Rust code block in the README SHALL either compile as a doctest of the `corvis` crate or be marked `ignore`
because it is a fragment. A complete example (the hello-world application) MUST be compiled, not ignored.

#### Scenario: Running the doctests
- **WHEN** `cargo test --doc -p corvis` runs
- **THEN** the README's non-ignored Rust blocks are compiled and pass

#### Scenario: A README example drifts from the API
- **WHEN** an attribute or function used by a non-ignored README block is renamed or removed
- **THEN** `cargo test --doc -p corvis` fails

### Requirement: README examples match the API
The README examples and tables SHALL describe the API as the code implements it. In particular:
`#[global]` MUST be shown below `#[module(...)]` (or as `#[module(global, ...)]`); namespaced configuration MUST use
`corvis::config::register_as` with `ConfigModule::for_feature` or `ConfigModuleOptions::load`;
`ServeStaticModule::for_root` MUST receive a `Vec` of `ServeStaticOptions::new(root)`; and any `#[on_event]` pattern
with `*` or `**` MUST be accompanied by `EventEmitterOptions { wildcard: true, .. }`.

#### Scenario: Making a module global
- **WHEN** a reader copies the global module example from the README
- **THEN** `#[global]` is placed below `#[module(...)]` and the code compiles

#### Scenario: Copying the wildcard event listener
- **WHEN** a reader copies the events example with `#[on_event("cat.*")]`
- **THEN** the example registers `EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() })`
  and the listener receives `cat.created`

### Requirement: The README follows the public naming rule
The README SHALL NOT contain the words "Nest", "NestJS" or "Prisma", except in nominative phrases such as "compatible
with schema.prisma" or the `schema.prisma` and `_prisma_migrations` names themselves.

#### Scenario: Naming check
- **WHEN** the README is searched for `\b(Nest|NestJS|Prisma)\b` and `@nestjs/`
- **THEN** there are no matches

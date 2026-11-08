# repo/naming Specification

## Purpose
Defines the public naming rule for the repository's published surfaces outside the documentation site, and the check
that enforces it.

## Requirements

### Requirement: Published surfaces do not use the banned brand words
The published surfaces of the repository SHALL NOT contain the words "Nest", "NestJS" or "Prisma", identifiers built on
them (such as `NestFactory` or `PrismaClient...`), or `@nestjs/` package names. Published surfaces are: crate sources
under `crates/*/src` (including rustdoc and code comments), `Cargo.toml` metadata, `examples/`, the root `README.md`,
the benchmark report and scripts (`bench/report.js`, `bench/results.md`, `bench/run.sh`, `bench/corvis-app/`) and the
documentation site. Nominative uses MUST be allowlisted with a reason: the `schema.prisma` file name and format, the
`_prisma_migrations` table, the `prisma/` folder of existing projects, error texts copied verbatim for parity, and
tests or fixtures that run against or record the output of an original Node library.

#### Scenario: A comment cites the original framework
- **WHEN** a source file under `crates/corvis/src` contains `As in @nestjs/jwt, ...`
- **THEN** the naming check fails and reports the file and line

#### Scenario: A nominative file name
- **WHEN** a source file mentions `schema.prisma` or `_prisma_migrations`
- **THEN** the naming check passes

#### Scenario: A differential test fixture
- **WHEN** a test under `crates/corvis-orm/tests/` maps errors to the original client's error class names
- **THEN** the naming check passes because the file is in the allowlist with its reason

### Requirement: The crate descriptions are Corvis's own
The `description` of every workspace crate and the opening paragraph of each crate's top-level rustdoc SHALL describe
Corvis in its own words and MUST NOT reuse the tagline of the original framework.

#### Scenario: Reading the corvis crate description
- **WHEN** a reader looks at `crates/corvis/Cargo.toml` or the docs.rs front page of `corvis`
- **THEN** the description does not contain "A progressive" and "efficient and scalable server-side applications"

### Requirement: The naming check is runnable
The repository SHALL provide a check script that scans the published surfaces, applies the allowlist, prints every
violation with file and line, and exits with a non-zero status when there is at least one.

#### Scenario: Clean tree
- **WHEN** `node scripts/check-naming.mjs` runs on the tree after this change
- **THEN** it prints no violation and exits with status 0

#### Scenario: Allowlist entry without a reason
- **WHEN** an allowlist entry has an empty reason
- **THEN** the check fails

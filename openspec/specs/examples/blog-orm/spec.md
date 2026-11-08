# examples/blog-orm Specification

## Purpose
TBD - created by archiving change e13-fix-the-examples. Update Purpose after archive.

## Requirements

### Requirement: Boolean query flags are parsed as text flags
The `blog-orm` example SHALL parse the `published` query parameter of `GET /posts` with `z::stringbool()`, so the
textual values `"true"`, `"1"`, `"yes"`, `"on"` select published posts and `"false"`, `"0"`, `"no"`, `"off"` select
unpublished posts. Any other value MUST be rejected with status `400`. Omitting the parameter MUST NOT filter by
`published`.

#### Scenario: published=false
- **WHEN** the database holds one published and one unpublished post and a client requests `GET /posts?published=false`
- **THEN** the response contains only the unpublished post

#### Scenario: published=true
- **WHEN** a client requests `GET /posts?published=true`
- **THEN** the response contains only the published post

#### Scenario: Invalid value
- **WHEN** a client requests `GET /posts?published=maybe`
- **THEN** the response status is `400`

#### Scenario: No filter
- **WHEN** a client requests `GET /posts`
- **THEN** both posts are returned

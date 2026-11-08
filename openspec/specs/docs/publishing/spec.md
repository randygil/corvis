# docs/publishing Specification

## Purpose
Defines where the documentation site is published and how a deployment is gated by the docs checks, so the public
site always matches a commit of `main` that passed them.

## Requirements

### Requirement: The site is published from main
The documentation site SHALL be published at `https://corvis.randygil.com` from the `main` branch, and a deployment
MUST fail without replacing the published site when `npm run docs:check` fails.

#### Scenario: Push to main
- **WHEN** a commit that passes the docs checks lands on `main`
- **THEN** the published site is rebuilt from that commit

#### Scenario: Broken docs
- **WHEN** a commit on `main` breaks a docs check (for example a dead link)
- **THEN** the image build fails and the previous site stays online

### Requirement: Published URLs match the local site
The published site SHALL serve the same clean URLs as the dev server, in both languages, and answer unknown paths with
status 404 and the site's 404 page.

#### Scenario: Deep link
- **WHEN** a reader opens `https://corvis.randygil.com/es/controllers`
- **THEN** the Spanish controllers page is served with status 200

#### Scenario: Unknown path
- **WHEN** a reader opens a path that is not a page
- **THEN** the response status is 404 and the body is the site's 404 page

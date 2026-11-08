# Spec Delta

## Purpose

Defines how the Corvis documentation site serves its content in English (primary) and Spanish (translation), so that
readers of either language get the same pages, navigation and working links.

## ADDED Requirements

### Requirement: English is served at the site root
The documentation site SHALL serve its English pages at the root path and declare the page language as English.

#### Scenario: Opening the home page
- **WHEN** a reader opens `/`
- **THEN** the home page is shown in English and the document language is `en`

#### Scenario: Opening a guide page
- **WHEN** a reader opens `/controllers`
- **THEN** the controllers page is shown in English

### Requirement: Spanish is served under /es/
The site SHALL serve the Spanish translation of every page under the `/es/` prefix, using the same slug as the English
page, and declare the page language as Spanish.

#### Scenario: Opening a Spanish page
- **WHEN** a reader opens `/es/controllers`
- **THEN** the controllers page is shown in Spanish and the document language is `es`

### Requirement: Every page exists in both languages
For every English page there SHALL be a Spanish page with the same slug, and for every Spanish page an English one, and
each pair MUST carry the same availability status.

#### Scenario: Page parity check
- **WHEN** the docs checks run
- **THEN** they fail if a page exists in only one language or if a pair declares different statuses

### Requirement: Language switcher keeps the page
Every page SHALL offer a language switcher that leads to the same page in the other language.

#### Scenario: Switching from English to Spanish
- **WHEN** a reader on `/fundamentals/custom-providers` picks Spanish in the language switcher
- **THEN** the browser opens `/es/fundamentals/custom-providers`

#### Scenario: Switching from Spanish to English
- **WHEN** a reader on `/es/openapi/operations` picks English in the language switcher
- **THEN** the browser opens `/openapi/operations`

### Requirement: Navigation matches in both languages
The navigation bar and the sidebar SHALL list the same pages, in the same order and grouping, in both languages, with
labels in the page's language, and links that stay within that language.

#### Scenario: Sidebar on a Spanish page
- **WHEN** a reader opens any page under `/es/`
- **THEN** every sidebar and navigation link points under `/es/` and its label is in Spanish

#### Scenario: Sidebar on an English page
- **WHEN** a reader opens any English page
- **THEN** no sidebar or navigation link points under `/es/`, and labels are in English

### Requirement: Links stay in the reader's language and resolve
Internal links in page content SHALL point to a page of the same language, and every internal link and anchor MUST
resolve to an existing page and heading.

#### Scenario: Link inside a Spanish page
- **WHEN** a Spanish page links to another documentation page
- **THEN** the link points under `/es/` to an existing Spanish page

#### Scenario: Link with an anchor
- **WHEN** a page links to `/modules#<anchor>` (or its `/es/` equivalent)
- **THEN** the target page in that language has a heading whose anchor is `<anchor>`

#### Scenario: Broken link
- **WHEN** a page links to a missing page or anchor
- **THEN** the docs build or checks fail and name the page and the link

### Requirement: Site chrome is localized
The site interface (navigation, sidebar headings, outline title, search box and its messages, previous/next labels,
theme switch, 404 page, footer and the page status badge) SHALL be shown in the language of the current page.

#### Scenario: Status badge on an unavailable feature
- **WHEN** a reader opens a page marked as not yet available
- **THEN** the badge reads "Not available yet" on the English page and "No disponible todavía" on the Spanish page

#### Scenario: Searching in Spanish
- **WHEN** a reader opens the search box on a Spanish page
- **THEN** the search interface is in Spanish and results come from Spanish pages

### Requirement: English pages are fully in English
English pages SHALL contain no Spanish prose: headings, text, tables, admonition titles, and the comments, example
strings and placeholder names in code examples MUST be in English. API names, commands and crate names stay unchanged.

#### Scenario: Code example comments
- **WHEN** an English page shows a code example that has comments or example strings
- **THEN** they are written in English, placeholder names such as `MiGuard` or `mi-app` read `MyGuard` or `my-app`,
  and the code is otherwise identical to the Spanish page's example

### Requirement: Public naming rule holds in both languages
Pages in both languages MUST NOT use the words "Nest", "NestJS" or "Prisma", except in nominative phrases such as
"compatible with schema.prisma".

#### Scenario: Naming check
- **WHEN** the docs checks run
- **THEN** they fail if a page uses a forbidden name outside the allowed phrases, naming the page and line

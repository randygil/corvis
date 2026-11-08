# Proposal

Implements [randygil/corvis#4](https://github.com/randygil/corvis/issues/4) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

The documentation site in `docs/` (VitePress, 173 pages, about 62,000 words) exists only in Spanish and has never been
committed. Corvis targets teams migrating Node.js backends worldwide, and every other public surface of the repository
(README, roadmap, issues, specs) is in English, so the docs must be readable in English first. The Spanish text is
complete and reviewed, so it should stay available as a translation instead of being discarded. This has to land before
the site is published (#5) and before every later change starts adding docs pages in both languages, as the definition
of done requires.

## What Changes

- Commit the current Spanish site as the baseline, so the move and the translation are reviewable as history.
- Make English the primary locale, served at the site root (`/`), and move the Spanish pages under `/es/`, with
  VitePress i18n and its language switcher. Page slugs stay identical in both locales (they are already English), so the
  switcher lands on the same page in the other language.
- Translate all 173 pages to English: prose, headings, admonition titles, tables, and the comments and user-facing
  strings inside code examples. Code identifiers, crate names, attributes and commands stay as they are.
- Localize the site chrome per locale: navigation bar, sidebar labels, outline label, search UI, footer, 404 page, theme
  switch labels and the page status badge.
- One sidebar definition with labels in both languages, so both trees always list the same pages in the same order.
- Spanish absolute links point inside `/es/`; anchors in links match the headings of the target page in each language.
- A check script (page parity, links and anchors, naming rule) that runs with the docs build.
- Short contributor notes on writing docs in both languages (where pages go, terminology, how to run the checks).

Out of scope:
- Publishing the site and its domain or hosting (#5).
- The English README (#3).
- Correcting or extending the technical content of pages; translation keeps the meaning of the Spanish text. Factual
  errors found while translating are reported as issues, not fixed here.
- Renaming the repository folder (#7) and other locales beyond English and Spanish.

## Capabilities

### New Capabilities

- `docs/localization`: the documentation site serves English at the root and Spanish under `/es/`, with the same pages,
  navigation and working links in both, a language switcher, and localized site chrome.

### Modified Capabilities

None (no specs exist yet).

## Impact

- `docs/`: all pages (English at the root, Spanish moved to `docs/es/`), `.vitepress/config.mts` (locales),
  `.vitepress/sidebar.mts` (bilingual labels), `.vitepress/theme/StatusBadge.vue` (localized labels, status keys in
  English), `package.json` (description, check script).
- New `docs/scripts/check.mjs` (Node, no new dependencies).
- Contributor notes on bilingual docs (in `docs/` or `CONTRIBUTING.md`, which #2 creates).
- No Rust code changes; the parity of the port apps is unaffected.

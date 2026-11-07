# Design

## Context

See proposal.md for motivation; requirements are in `specs/docs/localization/spec.md`. Current state of `docs/`
(untracked in git):

- VitePress 1.6.4, `cleanUrls`, local search, custom theme (`StatusBadge.vue` in the `doc-before` slot, `custom.css`).
- 173 Markdown pages, about 62,000 words, in 18 section folders plus 17 top-level pages. Slugs are already English
  (`/fundamentals/custom-providers`).
- `config.mts` sets `lang: 'es'` and every UI string in Spanish; `sidebar.mts` (236 lines) holds one tree with Spanish
  labels; the nav bar has five Spanish entries.
- Links: 448 absolute Markdown links (`](/modules)`), 4 HTML `href="/..."` links in `index.md`, no relative links,
  12 external links. 13 links carry an anchor built from a Spanish heading (`/modules#encapsulacion`).
- Every page has a `status` frontmatter key with Spanish values: `documentado` (65), `no-disponible` (94),
  `parcial` (14). `StatusBadge.vue` maps the last two to Spanish labels.
- Code examples contain Spanish comments and strings (58 `//` comment lines plus string literals).
- VitePress already fails the build on links to missing pages, but it does not check anchors.

## Goals / Non-Goals

**Goals:**
- One source of truth for the navigation tree, so the two languages cannot drift apart.
- Mechanical, scriptable checks for everything the spec requires that a build does not already enforce.
- A move of the Spanish pages that keeps their git history.

**Non-Goals:**
- Visual redesign of the theme or the home page.
- Automatic (machine) translation pipelines or a translation-memory tool; the translation is written once, by hand
  or by agents, and reviewed.

## Decisions

1. **English at the root, Spanish in `docs/es/`.** This is VitePress's default i18n layout: `locales.root` (English)
   and `locales.es` with `link: '/es/'`. Alternative considered: `docs/en/` + `docs/es/` with `rewrites` mapping `en/`
   to the root; symmetric folders, but every new page needs a rewrite-aware path and the dead-link checker reports
   rewritten paths, which is harder to read. Rejected for the simpler layout.
2. **History first, then translation.** Three steps in git: commit the Spanish site where it is; `git mv` every page
   into `docs/es/` in a commit of its own (only renames plus the link prefix rewrite, so `git log --follow` keeps
   working); then add the English pages at the root. Reviewers can read the translation as additions only.
3. **Spanish link rewrite by script.** A one-off Node script rewrites `](/x)` and `href="/x"` to `/es/x` in the moved
   pages, leaving external links and `/logo.svg`-style assets alone. Anchors in Spanish pages stay as they are (they
   already match the Spanish headings).
4. **Bilingual sidebar and nav in one tree.** `sidebar.mts` holds entries as `{ en, es, link }` and exports
   `sidebar(locale)` and `nav(locale)`, which pick the label and prefix links with `/es` for Spanish. Alternative: two
   copies of the tree; rejected because the spec requires identical order and grouping and copies drift.
5. **Locale config.** `config.mts` keeps shared settings at the top level and moves `lang`, `description` and all
   `themeConfig` strings into `locales.root` (English) and `locales.es` (the current Spanish strings). Local search
   gets `options.locales.es.translations` with the existing Spanish texts, so each locale searches only its pages.
6. **Status keys in English.** Frontmatter values become `documented`, `partial` and `unavailable` in both languages;
   `StatusBadge.vue` keeps a label table per language and picks it with `useData().lang`. Alternative: keep the Spanish
   keys; rejected because English sources would carry Spanish metadata and the parity check compares the keys.
7. **Translation rules.** English pages mirror the Spanish ones block by block: same headings in the same order and
   level, same admonitions, tables and code blocks. Code is copied verbatim except comments and example strings, which
   are translated. Meaning is kept; errors found are listed for new issues instead of fixed. Terminology: keep the
   framework terms the API uses (module, provider, controller, guard, pipe, interceptor, middleware, exception filter,
   gateway) and the Spanish site's existing choices (it already keeps "providers", "guards", "pipes", "interceptors"
   in English). The translation is split by section into batches of about 25–45 pages, delegated to the `docs-writer`
   agent, up to four batches in parallel (no Cargo builds involved, so no shared-target or disk concern). English
   anchors are fixed by translating the 13 anchored links to the English slugs of their target headings.
8. **One check script, run after the build.** `docs/scripts/check.mjs` (Node, no new dependencies), wired as
   `npm run docs:check` = `vitepress build && node scripts/check.mjs`:
   - page parity: the set of slugs under the root equals the set under `es/`, and each pair has the same `status`;
   - structure parity: each pair has the same number of headings per level and of fenced code blocks (catches dropped
     sections without judging prose);
   - links and anchors, read from the built HTML in `.vitepress/dist`: every internal `href` resolves to a built page,
     every `#anchor` to an `id` on that page, links in `/es/` pages stay under `/es/` and links in English pages never
     enter it (nav and sidebar included, since they are part of the HTML);
   - language: English sources contain no `¿`, `¡`, `ñ` or accented vowels outside an allowlist, and none of a short
     list of Spanish function words outside code identifiers; Spanish sources contain no voseo forms;
   - naming: case-sensitive whole-word `Nest`, `NestJS` and `Prisma` in any source fail, with `schema.prisma` allowed.
   Each failure prints the file, line and offending text; the script exits non-zero if any check fails.
9. **Contributor notes in `docs/README.md`.** Kept out of the site with `srcExclude: ['README.md', 'scripts/**']`.
   `CONTRIBUTING.md` is created by #2 on another branch; it can link to this file later instead of both changes editing
   the same new file.

## Risks / Trade-offs

- [Translation quality varies between batches] → one terminology list in the agent brief, structure-parity check, and a
  read-through of a sample of pages per batch before accepting it.
- [Huge diff, hard to review] → history in three commits (baseline, move, translation) and the translation split by
  section, so each batch is reviewable alone.
- [Heuristic language check gives false positives (names like "Señor" in sample data, accented words in quoted
  output)] → a small explicit allowlist in the script, each entry with the page it belongs to.
- [Pages edited in only one language later] → the parity checks fail the docs check; the definition of done already
  requires both languages.
- [Search index doubles in size] → acceptable for 346 pages; VitePress builds one index per locale.

## Migration Plan

The site is not published yet (#5), so there are no existing URLs to redirect: Spanish pages move from `/x` to `/es/x`
before anyone links to them. Rollback is reverting the branch.

# Tasks

## 1. Baseline and move

- [ ] 1.1 Commit the current Spanish site as is (sources, `.vitepress/` config and theme, `public/`, `package.json`, `package-lock.json`; `node_modules`, `cache` and `dist` stay ignored); verify `git status` shows nothing untracked under `docs/` except ignored folders and `npm run docs:build` passes on the committed tree
- [ ] 1.2 `git mv` every page into `docs/es/` (same relative paths) and rewrite internal absolute links `](/x)` and `href="/x"` in the moved pages to `/es/x` with a one-off script (external links and assets untouched); commit the renames and the rewrite together; verify `git log --follow docs/es/controllers.md` reaches the baseline commit and no Spanish page links outside `/es/`

## 2. Site configuration

- [ ] 2.1 Rework `sidebar.mts` into one bilingual tree (`{ en, es, link }` entries) exporting `sidebar(locale)` and `nav(locale)`, with English labels for every entry; verify both locales produce the same number of entries in the same order (a short assertion in `scripts/check.mjs` or a one-off Node run)
- [ ] 2.2 Move locale-specific settings in `config.mts` into `locales.root` (English, `lang: 'en'`) and `locales.es` (`lang: 'es'`, `link: '/es/'`, the current Spanish strings), including nav, sidebar, outline, doc footer, theme switch, 404 page, footer and local-search translations; add `srcExclude: ['README.md', 'scripts/**']`; verify `npm run docs:dev` (bound to 127.0.0.1) shows the language switcher and Spanish chrome on `/es/` pages
- [ ] 2.3 Rename the `status` values to `documented`, `partial`, `unavailable` in every page and make `StatusBadge.vue` pick labels by language ("Partial support" / "Not available yet" and the current Spanish labels); verify a Spanish `unavailable` page shows "No disponible todavía"
- [ ] 2.4 Write `docs/scripts/check.mjs` with the checks of design decision 8 (page parity and status, structure parity, links and anchors from `dist`, language heuristics with an allowlist, naming rule) and add `docs:check` to `package.json` (also translate its `description`); verify each check fails on a deliberately broken temporary page and passes once it is removed

## 3. English translation

Each batch: translate the Spanish pages of the listed sections into English at the root with the same paths, following design decision 7 (mirror structure, code verbatim except comments and strings, existing terminology, anchors pointing to English headings); list any factual error found instead of fixing it; verify with `npm run docs:check` that the batch's pages pass parity, structure, links and language checks, and read through at least three of its pages.

- [ ] 3.1 Batch A, core guide (35 pages): the 17 top-level pages (including `index.md` and its HTML links), `fundamentals/`, `websockets/`
- [ ] 3.2 Batch B, HTTP and application (36 pages): `http/`, `application/`, `security/`, `openapi/`
- [ ] 3.3 Batch C, GraphQL, microservices and MCP (36 pages): `graphql/`, `microservices/`, `mcp/`
- [ ] 3.4 Batch D, data and operations (35 pages): `data/`, `ai/`, `observability/`, `reliability/`, `cli/`
- [ ] 3.5 Batch E, recipes and help (31 pages): `recipes/`, `faq/`, `devtools/`, `discover/`
- [ ] 3.6 Open one issue in epic #1 (or the relevant epic) per factual error the batches listed, or record that none were found

## 4. Contributor notes

- [ ] 4.1 Write `docs/README.md`: layout (English at the root, Spanish in `es/`), the bilingual sidebar, translation rules and terminology, status values, and how to run `npm run docs:dev` and `npm run docs:check`; verify the page is not part of the built site and the documented commands run as written

## 5. Definition of done

- [ ] 5.1 `npm run docs:check` passes with all checks enabled and no allowlist entry without a reason
- [ ] 5.2 Manual pass in the dev server: home, one page per section in both languages, the language switcher both ways, search in both languages, the 404 page in both languages, light and dark themes
- [ ] 5.3 No Rust code changed: record that `cargo build`, clippy, tests and the apidiff of the port apps are unaffected, so they are not rerun for this change
- [ ] 5.4 Docs EN+ES: this change is the docs; confirm both locales are complete (173 pages each)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #4, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

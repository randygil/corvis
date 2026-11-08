# Corvis documentation

The documentation site is built with [VitePress](https://vitepress.dev). English is the primary language; Spanish is a
full translation. This file is not part of the site.

The site is published at https://corvis.randygil.com from `main` (Coolify builds `docs/Dockerfile`, which runs
`npm run docs:check`, so a failing check stops the deployment).

## Running it

```bash
cd docs
npm install
npm run docs:dev     # http://127.0.0.1:5173
npm run docs:check   # build + all checks; must pass before a change is done
```

## Layout

| Path | Content |
| --- | --- |
| `*.md`, `<section>/*.md` | English pages, served at `/` |
| `es/*.md`, `es/<section>/*.md` | Spanish pages, served at `/es/`, same paths as the English ones |
| `.vitepress/config.mts` | Site settings; per-language interface strings under `locales` |
| `.vitepress/sidebar.mts` | One navigation tree with labels in both languages |
| `.vitepress/theme/` | Theme, including the page status badge |
| `scripts/check.mjs` | The checks run by `npm run docs:check` |

## Adding or changing a page

1. Write the English page, then its Spanish translation at the same path under `es/`. Both are required.
2. Add new pages to the tree in `.vitepress/sidebar.mts` with both labels (`['English', 'Español', '/path']`); links
   there have no language prefix.
3. Links inside pages use absolute paths: `/modules` in English pages, `/es/modules` in Spanish pages. Anchors follow
   each page's own headings (`/modules#encapsulation`, `/es/modules#encapsulacion`).
4. Set `status` in the frontmatter, identical in both languages:
   - `documented`: the feature exists and the page documents it (default);
   - `partial`: part of the feature exists; the page says what is missing;
   - `unavailable`: the feature does not exist yet; the page points to the closest real alternative.
5. Run `npm run docs:check`.

## Translation rules

- The two versions mirror each other block by block: same headings in the same order, same admonitions, tables and
  code blocks. The check compares headings per level and the number of code blocks.
- Code is identical in both languages except comments, example strings (messages, sample values) and placeholder
  names (`MyGuard` / `MiGuard`, `my-app` / `mi-app`). API names and route strings stay the same.
- Keep the framework terms as the API names them: module, provider, controller, guard, pipe, interceptor, middleware,
  exception filter, gateway. The Spanish pages keep "providers", "guards", "pipes" and "interceptors" in English.
- Spanish is neutral Latin American Spanish (no voseo). English is plain technical English, US spelling, addressing
  the reader as "you".
- Do not use the words "Nest", "NestJS" or "Prisma"; nominative phrases such as "compatible with schema.prisma" are
  fine.

## What the checks cover

- every page exists in both languages with the same `status`;
- each pair has the same headings per level and the same number of code blocks;
- both sidebars list the same pages, and every listed page exists;
- every internal link and anchor in the built site resolves, and links stay in the page's language (the language
  switcher is the only exception);
- no Spanish text in English pages, no voseo in Spanish pages, and the naming rule.

Real exceptions to the language checks go in the `ALLOW` list at the top of `scripts/check.mjs`, each with a reason.

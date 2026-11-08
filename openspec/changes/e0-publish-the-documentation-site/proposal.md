# Publish the documentation site

Implements [randygil/corvis#5](https://github.com/randygil/corvis/issues/5).

## Why

The bilingual site (#4) only runs locally. Readers need a public address, and every deployment must pass the same checks
as a local `npm run docs:check`.

## What Changes

- `docs/Dockerfile`: builds the site with `npm run docs:check` (build plus all checks) and serves `.vitepress/dist`
  with nginx.
- `docs/nginx.conf`: clean URLs (`/controllers` → `controllers.html`), the site's own 404 page, long cache for hashed
  assets, gzip.
- A Coolify application deploys `main` of `randygil/corvis` (base directory `docs/`) at https://corvis.randygil.com,
  redeploying on every push to `main`.

## Capabilities

### New Capabilities
- `docs/publishing`: where and how the documentation site is published.

## Impact

- New files under `docs/`; no Rust code changes.
- Out of scope: GitHub Pages and CI-driven deployment (CI is #6; it can run `docs:check` on PRs), search engine
  settings, analytics, a custom domain other than `corvis.randygil.com`, and auditing every page against the code
  (pages already carry a `status` badge saying whether the feature exists).

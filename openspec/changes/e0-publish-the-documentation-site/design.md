# Design

## Decisions

1. **Docker image, built by Coolify.** A two-stage Dockerfile (Node 22 to build, nginx to serve) keeps the deployment
   independent of Coolify's build packs and can be tested locally with `docker build docs`. The build stage runs
   `docs:check`, not only `docs:build`, so a broken link or a missing translation stops the deployment.
2. **nginx `try_files $uri $uri.html $uri/`** maps VitePress `cleanUrls` output; `error_page 404 /404.html` serves the
   site's 404 page (VitePress picks the language from the path when it hydrates).
3. **Coolify application** in its own project, source `randygil/corvis` through the existing GitHub App, branch `main`,
   base directory `/docs`, automatic deployment on push, HTTPS through Coolify's proxy.

## Risks / Trade-offs

- [The repository is private while the site is public] → the docs describe the public API only; nothing in `docs/`
  is secret. Making the repo public is a separate decision.

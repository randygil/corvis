# Tasks

## 1. Image

- [x] 1.1 Add `docs/Dockerfile`, `docs/nginx.conf` and `docs/.dockerignore`; verify locally that `docker build docs`
  passes and the container answers 200 for `/`, `/es/`, `/controllers`, `/es/controllers`, 301 for `/es` and 404 for
  unknown paths in both languages

## 2. Deployment

- [ ] 2.1 Create the Coolify application (source `randygil/corvis`, branch `main`, base directory `/docs`, Dockerfile
  build, domain `https://corvis.randygil.com`, automatic deployment on push)
- [ ] 2.2 Deploy and verify the same URLs as 1.1 on `https://corvis.randygil.com`, with a valid certificate

## 3. Definition of done

- [x] 3.1 No Rust code changed: cargo build, clippy, tests and the apidiff of the ports are unaffected
- [x] 3.2 Docs EN+ES: mention the published address in `docs/README.md`

# Proposal

Implements [randygil/corvis#3](https://github.com/randygil/corvis/issues/3) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

The root `README.md` is the first page anyone sees on GitHub and, through `readme = "../../README.md"` in
`crates/corvis/Cargo.toml`, the page crates.io will show for `corvis`. It is 1,518 lines of Spanish while every other
public surface (docs site at the root, roadmap, issues, specs) is English first. The docs review also found examples in
it that do not compile or do not do what they say:

- the module table says `#[global]` goes **above** `#[module]`, but above it Rust fails with `cannot find attribute
  'global'`; it must go below;
- it lists `ConfigModule::register_as`, which does not exist: `register_as` is the free function `corvis::config::register_as`;
- it shows `ServeStaticModule::for_root(ServeStaticOptions {...})`, but `for_root` takes a `Vec<ServeStaticOptions>`
  built with `ServeStaticOptions::new(root)`;
- the events example uses `#[on_event("cat.*")]`, which only matches with `EventEmitterOptions { wildcard: true, .. }`
  (wildcards are off by default);
- it still names the original framework in several places.

## What Changes

- Rewrite `README.md` in English as a shorter landing page: what Corvis is, a complete hello-world, install and CLI
  quick start, repository layout, the "coming from Node.js" mapping tables, short examples of the main features,
  performance summary, security summary, known limitations, license, and links into the documentation site
  (`https://corvis.randygil.com`, Spanish at `/es/`) for everything detailed.
- Long reference material that the docs site already covers (Express body parser and `@Res()` details, multer tables,
  zod equivalence table, the full security list) is replaced by links to the matching docs pages. Anything that is in
  the README but not in the docs is moved into the right docs page (English and Spanish) before it leaves the README.
- Fix the four broken examples above and remove the brand mentions (lines 919, 1369, 1396, 1429 today).
- Add a doctest harness so the README's Rust blocks are compiled by `cargo test --doc -p corvis`; fragments that cannot
  compile alone are marked `ignore` explicitly.

## Capabilities

### New Capabilities

- `docs/readme`: the repository README is in English, follows the public naming rule, links to both languages of the
  documentation site, and its Rust examples compile or are explicitly marked as fragments.

### Modified Capabilities

None.

## Impact

- `README.md` (rewritten).
- `crates/corvis/src/lib.rs`: one `#[cfg(doctest)]` item that includes the README as doc comments (no runtime or
  public API change).
- Possibly a few docs pages (EN in `docs/`, ES in `docs/es/`) that receive README content not yet documented.
- No framework runtime change; the port apps' parity is unaffected.

## Out of scope

- The CLI default of `corvis build` (debug unless `--release`, while today's README says it runs `cargo build
  --release`): fixed by #20. This README describes the behavior the code has when it lands.
- Brand mentions outside the root README (code comments, crate descriptions, benchmark, `corvis-orm` defaults): #22.
- Translating `crates/corvis-orm/README.md` (still Spanish): belongs to the corvis-orm publishing epic ("README and
  compatibility table"); noted as a possible new issue if that task does not cover the translation.
- Absolute links for crates.io rendering of the README and other packaging metadata: #10.
- A Spanish copy of the README (`README.es.md`): not created; Spanish readers are sent to `/es/` of the docs site.

# Design: corvis builds without default features

## Context

`application.rs:290-294` builds the gateway list for the route posture check with
`crate::websockets::posture_gateways(..)` under `#[cfg(feature = "websockets")]` and `Vec::new()` otherwise. Without
the feature nothing constrains the element type, so type inference fails (E0282). With the type fixed, clippy reports
`ErasedHooks::gateway` (`di/provider.rs:66`) as never read: it is set for every provider but only read in
`websockets.rs:557`.

## Decisions

1. Annotate the type rather than restructure the posture code.
2. `#[cfg_attr(not(feature = "websockets"), allow(dead_code))]` on the field, rather than compiling the field out: the
   provider builder sets it unconditionally, and compiling it out would spread `cfg` through the builder.
3. CI checks the library only (no `--all-targets`), with no default features and each feature alone, in the existing
   clippy job on Linux and Windows. Feature unions beyond that are covered by the default and all-features builds.

## Risks / Trade-offs

- Tests are not checked per feature (see Out of scope in the proposal).

## Verification (2026-11-09)

`cargo clippy -p corvis --no-default-features [--features jwt|websockets|health-tls] -- -D warnings`: 0 errors and
0 warnings in all four sets (they failed before the fix).

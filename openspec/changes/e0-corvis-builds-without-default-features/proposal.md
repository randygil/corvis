# Proposal: corvis builds without default features

Issue: https://github.com/randygil/corvis/issues/153

## Why

`cargo check -p corvis --no-default-features` failed with E0282 at `crates/corvis/src/application.rs:294`: the
`Vec::new()` that replaces the gateway list when the `websockets` feature is off had no element type. Every feature set
without `websockets` failed, so an application depending on `corvis` with `default-features = false` could not build.
CI only built the default and all-features sets. Found by the #120 security review.

## What Changes

- Give the empty gateway list its type (`Vec<crate::posture::GatewayPosture>`).
- Allow the `ErasedHooks::gateway` field to be unread when `websockets` is off (it is only read by the WebSocket
  runtime), so clippy with `-D warnings` passes for every feature set.
- CI: the clippy job also checks the `corvis` library with no default features and with each feature alone.

## Capabilities

- Modified: `infra/ci`

## Impact

`crates/corvis/src/application.rs`, `crates/corvis/src/di/provider.rs`, `.github/workflows/ci.yml`. No runtime change for
the default feature set.

## Out of scope

- Test targets that use WebSocket or JWT APIs do not declare `required-features`, so `--all-targets` without default
  features still fails to compile. Gating the tests is separate work.

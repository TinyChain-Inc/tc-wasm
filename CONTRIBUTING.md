# Contributing to `tc-wasm`

`tc-wasm` hosts the shared TinyChain WASM helper crate. It keeps the ABI surface,
library manifests, and attested-time helpers aligned with the Rust host so
WASM-published libraries behave the same way as their native counterparts.

## How this crate fits into TinyChain

- Provides the macros and helpers that compile WASM libraries down to the same
  manifest format `tc-server` expects during `/lib` installs.
- Exercises minimal example handlers so downstream library authors can validate
  typed inputs/outputs and transaction wiring before shipping their own crates.
- Acts as the reference spot for attested time handling inside WASM modules,
  preventing adapter-specific drift.

## Contribution workflow

1. Follow the top-level TinyChain contributing guide for discussion channels,
   pull request expectations, and environment setup.
2. Apply the repo-wide style rules in `/CODE_STYLE.md` (grouped imports, `cargo fmt`,
   `cargo clippy --all-targets --all-features -D warnings`) before sending patches.
3. Keep dependencies lean; this crate must stay small so every adapter can embed
   it without bloating WASM builds.
4. Run `cargo test -p tc-wasm` (and any relevant downstream tests) before
   opening a PR so manifest generation and example flows stay healthy.
5. Document observable behavior changes in `README.md` so library authors know
  how host/runtime contracts evolved.

## Rights and licensing

By contributing to this crate you represent that (a) the work is authored by
you (or you have the necessary rights to contribute it) and (b) you transfer and
assign all right, title, and interest in the contribution to the TinyChain
Open-Source Project for distribution under the TinyChain open-source license
(Apache 2.0, see the root `LICENSE`). No other restrictions or encumbrances may
attach to your contribution.

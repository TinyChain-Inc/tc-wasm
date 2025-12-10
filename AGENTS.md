# tc-wasm Agent Notes

`tc-wasm` hosts the WASM helper utilities and examples. Keep artifacts minimal and
compatible with the manifest format expected by Python clients and `tc-server`.

## Development guidelines

- Maintain parity with the Python manifest schema (`Library.__json__`). WASM libraries
  must emit identical immutable attributes and `wasm_export` annotations or
  installation will fail. Update the README if the manifest shape changes.
- Avoid expanding dependencies; prefer small utilities that can be shared by other
  WASM-targeted crates without bloating the binary size.
- Keep examples short and focused on the minimal kernel wiring rather than exhaustive
  surface coverage.

## Testing

- Run `cargo test -p tc-wasm` after changes to example handlers, macros, or manifest
  generation. Add regression tests instead of leaving compatibility fallbacks in place.

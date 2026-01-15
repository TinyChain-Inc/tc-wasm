# tc-wasm

TinyChain WASM helper crate. It currently exports utilities used by WASM-targeted TinyChain libraries.

The `example` module in `src/lib.rs` shows how to:

1. Implement a minimal transaction type (`ExampleTxn`).
2. Write a handler (`HelloHandler`) with typed inputs/outputs.
3. Use `tc_ir::StaticLibrary` and the `tc_library_routes!` macro to build a runnable library via `hello_library()`.

When emitting a manifest for installation (e.g., via `/lib`), follow the existing TinyChain format: the JSON map returned by Python’s `Library.__json__` (defined in the TinyChain Python client) remains canonical. WASM libraries must produce the same structure (attributes serialized via `to_json`, immutable values only) and annotate each exported method with a `wasm_export` field pointing to the corresponding WASM export. `tc-server` will reject manifests that diverge from this format to preserve compatibility with existing clients.

Run `cargo test -p tc-wasm` to see the example exercised end-to-end.

## Hello WASM example

A concrete WASM-ready library lives under `examples/hello_wasm.rs`. It reuses the
same `ExampleTxn` + `HelloHandler` definitions from `tc-ir/examples/hello_library.rs`
and only adds a tiny ABI shim provided by `tc_wasm::abi` (`tc_library_entry`,
`alloc/free`, and the exported `hello` function). Build it with:

```bash
cargo build -p tc-wasm --example hello_wasm --target wasm32-unknown-unknown --release
```

This produces `target/wasm32-unknown-unknown/release/examples/hello_wasm.wasm`, which
exports:

- `tc_library_entry` – uses `manifest_bytes` + `RouteExport` to generate the manifest JSON
  describing `/lib/example` with a single `/hello` route.
- `alloc` / `free` – provided by `tc_wasm::abi` so every library shares the same
  host-memory helpers.
- `hello` – the actual TinyChain handler implemented via `HelloHandler`. It decodes the
  JSON body into a Rust `String`, invokes the same `HandleGet` logic shown in the
  `tc-ir` example, and serializes the response back to JSON.

Install the resulting module via `/lib` (or `client/py/bin/install_wasm.py`) the same as
any other TinyChain WASM artifact.

To wrap your own library crate, expose a `StaticLibrary`, implement `WasmTransaction`
for your transaction type (i.e., rebuild it from a `TxnHeader`), and export each route
via `dispatch_get/dispatch_put/...` helpers. The ABI module takes care of decoding the
request, awaiting the `Handle*` future, and encoding the response back to TinyChain so
your WASM entry point stays as small as the native example. Keep imports grouped and
formatted per the repo-wide `CODE_STYLE.md` whenever you add new modules or adapters.

### Future portability: WASI

Today TinyChain loads WASM libraries via Wasmtime in the default single-threaded profile.
In the future we plan to support WASI-based distribution so the same library binary can
run in other hosts (e.g., browser runtimes or edge environments) without bespoke glue.
Track the control-plane roadmap for the WASI deliverable; the ABI helpers defined here
will be kept portable so migrating to a WASI-aware TinyChain host only requires enabling
the new target triple during your `cargo build`.

## Attested time inside WASM

WASM libraries never talk to `/service/std/time` directly. Instead, the host
retrieves the signed time blob (timestamp, nonce, signature, `kid`) from the
control plane and passes it into the TinyChain WASM ABI (e.g., via
`tc_host::attested_time()`). Every library must:

1. Embed (or fetch from manifest metadata) the control-plane public key
   corresponding to `kid`.
2. Verify the signature locally before trusting the timestamp.
3. Reject the request if the signature fails, the nonce repeats, or the host
   omits the blob entirely. This guarantees that an honest host (running the
   published binary as-is) cannot accidentally feed unverified time into the
   library; only a host that fully compromises the runtime (e.g., patches the
   module) could bypass the check.

This protects honest deployments from partially implemented hosts: even if a
host forgets to forward the blob, the WASM library will fail closed. It does
not defend against a fully compromised host (which can fabricate blobs), so
runtimes must still be attested and monitored via MetricsChain.

## Tamper resistance expectations

Do **not** rely on binary obfuscation, anti-debugging tricks, or self-modifying
code to secure a TinyChain WASM library. A malicious host can always attach a
debugger, patch linear memory, or substitute a different module. Instead:

- Publish deterministic builds and sign them.
- Require hosts to prove they’re running the signed module via remote attestation
  (TPM/TEE) or measured boot.
- Let the control plane revoke hosts quickly when tampering is detected.

Keep the WASM code minimal and portable; the ambient environment (attestation +
ledger auditing) is responsible for tamper resistance.

# large-upload — bounded-memory streaming demonstration

One minimal OpenAPI 3.1.0 document, two operations, one question: **does
memory stay flat when the payload is a gigabyte?** Both generated halves are
streaming-first, so the answer is yes — and this example measures it live on
the client AND the server.

```yaml
PUT /blobs/{id}    # application/octet-stream → 201 {bytes_received, sha256}
PUT /audio/{id}    # audio/wav                → 201 {bytes_received, sha256}
```

The generator classifies `audio/wav` exactly like `application/octet-stream`
(raw byte stream), so the pair demonstrates the equivalence at the codegen
level while keeping the scenario concrete: raw blobs and WAV audio tracks.
Like kitchen-sink, the example is split into THREE production crates — shared
models, Reqwest client, Axum server — plus ONE shared measurement source
module (`memmon/mod.rs`, included by both transports via `#[path]`), so every
schema type has exactly ONE Rust identity and neither transport compiles the
other.

## Layout

| Path | Contents |
| --- | --- |
| `openapi.yaml` | The two-operation document. |
| `models/` | Shared schema surface crate (`large-upload-models`, `--generate types`). `generated/models.rs` + `generated/views.rs` are committed generator output; `src/lib.rs` wires them in unmodified as `#[path]` file modules. Deps: serde ONLY — the generated models need nothing else here. |
| `memmon/mod.rs` | NOT a crate: one copy of demo-only memory instrumentation (sampled RSS + getrusage high-water mark, progress printers), included by BOTH transport crates via `#[path = "../../memmon/mod.rs"]` (relative to each crate's `src/lib.rs`) without coupling them to each other. No axum, no reqwest; its three dependencies live in both host manifests. |
| `client/` | Client crate (`large-upload-client`, `--generate client --types-path large_upload_models`). `generated/client.rs` is committed generator output; `src/lib.rs` wires it in unmodified as a `#[path]` file module; `src/transfers.rs` synthesizes + streams the WAV file; `src/main.rs` the binary; `tests/smoke.rs` the ignored real-TCP smoke tests. |
| `server/` | Server crate (`large-upload-server`, `--generate server --types-path large_upload_models`). `generated/server.rs` is committed generator output; `src/lib.rs` wires it in unmodified as a `#[path]` file module; `src/app.rs` is the demo application (`LargeUploadApp`, disk + proxy modes); `src/main.rs` the binary. |

Normal generation writes NO Cargo.toml: every manifest above is
hand-maintained, carrying only the dependencies its generated code genuinely
uses. The server's DEFAULT build never pulls reqwest — proxy-mode forwarding
opts in explicitly through the `proxy` Cargo feature (`proxy =
["dep:reqwest"]`); the GENERATED server module itself needs no client stack,
as proven in CI by `scripts/check-transport-isolation.sh` (see also the
split-workspace compile proofs in `crates/generator/tests/split_workspace.rs`).

## What runs

| Piece | Behavior |
|---|---|
| `large-upload-client` | Synthesizes a deterministic WAV file (`--size-mib`, default **1024**) written chunk-wise, streams it through BOTH media types via `reqwest::Body::wrap_stream(ReaderStream…)`, verifies both receipts against the locally computed SHA-256. |
| server, DISK mode (default) | Consumes `body.into_data_stream()` chunk-by-chunk into a temp-dir file while an incremental SHA-256 digests the same chunks. Nothing aggregates. |
| server, PROXY mode (`--proxy-url <base>`, requires `--features proxy`) | Wraps the INBOUND stream directly as the outbound body and forwards to `{base}/blobs\|audio/{id}` — zero buffering; point it at a second disk-mode instance to stream through three processes. A default build rejects the flag instead of compiling reqwest. |

The JSON receipts prove the full payload was handled without ever
downloading it back.

## Memory measurement

The `memmon` module combines two complementary sources:

* **Sampled RSS** — `memory-stats` reads the process resident set size every
  50 ms from a background task and tracks the running maximum (shows the
  *shape* of the run).
* **Kernel high-water mark** — getrusage(2) `ru_maxrss` at exit; maintained
  by the kernel, so even a spike shorter than one sampling interval cannot
  hide (catches *sub-interval* peaks).

Both are compared against the RSS baseline captured right before the
transfers start. Progress lines during each transfer pair bytes moved with
the CURRENT rss, making flatness observable live:

```
[send blob demo-blob] 512/1024 MiB sent, rss=4.9 MiB      # client
[recv blob demo-blob] 512 MiB received, rss=4.8 MiB       # server
```

Each process prints a final report and exits non-zero if the peak delta
breached the budget:

```
=== memory report [client] ===
baseline RSS       :        3.4 MiB
sampled peak RSS   :        5.0 MiB   (sampler @ 50ms)
kernel high-water  :        5.0 MiB   (getrusage ru_maxrss)
peak delta vs base :        1.5 MiB   (limit 32 MiB -> PASS)
elapsed            :        6.5 s
```

Threshold: 32 MiB by default (`LARGE_UPLOAD_MAX_RSS_DELTA_MIB` overrides).
Measured on macOS over loopback: 2 GiB per full run, peak deltas ≈ 1.5 MiB
(client) and ≈ 3–4 MiB (server/proxy) — three orders of magnitude below the
payload, independent of `--size-mib`.

## Run it

The full 1 GiB demonstration is a **`--release` workflow**: correctness is
identical in debug builds, but gigabyte-scale synthesis, hashing, and SHA-256
verification burn far less wall-clock time optimized. Pass `--release` to
BOTH sides (the client does the synthesizing + hashing, the server the
digesting).

```sh
# terminal 1 — server in disk mode (default port 8097)
cargo run --release -p large-upload-server

# terminal 2 — client with the full 1 GiB demonstration
cargo run --release -p large-upload-client
# smaller/faster: add -- --size-mib 64 · keep the synthesized file: --keep

# proxy chain: frontend forwards to a backend, both stay flat
# (proxy forwarding is an explicit Cargo-feature opt-in)
cargo run --release -p large-upload-server --features proxy -- --port 8098
cargo run --release -p large-upload-server --features proxy -- --port 8097 \
    --proxy-url http://127.0.0.1:8098
cargo run --release -p large-upload-client -- --size-mib 1024
```

Stop the servers with Ctrl-C: they print their final memory report and exit
non-zero if the threshold was breached.

## Tests

```sh
cargo test -p large-upload-models                 # determinism gate (byte-stable regeneration)
cargo test -p large-upload-client -- --ignored    # real-TCP smoke: disk round trip + proxy chain
LARGE_UPLOAD_GENERATED_UPDATE=1 cargo test -p large-upload-models --test determinism   # refresh generated/
```

The smoke tests run an 8 MiB sweep and assert the memory margin with a
generous 64 MiB bound (gross-aggregation detection, not noise-level proof —
that is what the binaries' reports at gigabyte scale are for).

## Regeneration

Generated artifacts are committed and MUST stay byte-stable (main spec §50
tests 38–39). Refresh snapshots with the update switch shown above, or
regenerate exactly as committed with the normal CLI (source-only; manifests
stay hand-maintained). The three commands below are REPLAYED VERBATIM by
`crates/generator/tests/example_regeneration.rs` in CI — if one of them
stops reproducing the committed bytes, that test fails:

```sh
oapi-to-rust examples/large-upload/openapi.yaml \
  --generate types \
  --output-dir examples/large-upload/models/generated

oapi-to-rust examples/large-upload/openapi.yaml \
  --generate client --types-path large_upload_models \
  --output-dir examples/large-upload/client/generated

oapi-to-rust examples/large-upload/openapi.yaml \
  --generate server --types-path large_upload_models \
  --output-dir examples/large-upload/server/generated
```

Note: reqwest is built without TLS features in this workspace, which keeps
the demo loopback-only by construction.

# kitchen-sink example

One OpenAPI 3.1.0 document (`openapi.yaml`) exercising every feature class of
the openapi-to-rust generator — each operation copied from a proven fixture
shape under `crates/generator/fixtures/`, cross-referenced to main-spec
sections in its header comment (feature classes a–m). The example is split
into THREE crates — shared models, Reqwest client, Axum server — so every
schema type has exactly ONE Rust identity and neither transport compiles the
other. The generated artifacts are COMMITTED under each crate's `generated/`
directory and compiled UNMODIFIED via `include!`. Two runnable demos prove
end-to-end behavior over real TCP: an Axum server implementing all 22
operations and a reqwest client driving a full-operation sweep against it.

## Layout

| Path | Contents |
| --- | --- |
| `openapi.yaml` | The 22-operation union document; its header comment indexes feature classes a–m. |
| `models/` | Shared schema surface crate (`kitchen-sink-models`, `--generate types`). `generated/models.rs` + `generated/views.rs` are committed generator output; `src/lib.rs` include!s them unmodified. Deps: serde, serde_json, dependency-light `openapi-support` — no transport stacks. |
| `client/` | Client crate (`kitchen-sink-client`, `--generate client --types-path kitchen_sink_models`). `generated/client.rs` is committed generator output; `src/lib.rs` include!s it; `src/sweep.rs` is the hand-written sweep driver; `src/main.rs` the binary; `tests/smoke.rs` the ignored real-TCP smoke test. Deps: models crate + reqwest-side support features ONLY. |
| `server/` | Server crate (`kitchen-sink-server`, `--generate server --types-path kitchen_sink_models`). `generated/server.rs` is committed generator output; `src/lib.rs` include!s it; `src/app.rs` is the hand-written demo application (`KitchenSinkApp`) + router wiring; `src/main.rs` the binary. Deps: models crate + axum-side support features ONLY (reqwest absent). |
| `models/tests/determinism.rs` | Regeneration contract for ALL committed artifacts across the three crates (see below). |

Normal generation writes NO Cargo.toml: every manifest above is
hand-maintained, carrying only the dependencies its generated code genuinely
uses.

## Run it manually

Terminal 1 (server):

```sh
cargo run -p kitchen-sink-server            # or: -- --port 8123
```

Terminal 2 (client):

```sh
cargo run -p kitchen-sink-client            # or: -- --base-url http://127.0.0.1:8123
```

The client prints one line per sweep step (`op   ok   summary`, `FAIL`
instead of `ok` when an in-step assertion broke) and ends with
`all N steps passed against <base-url>`. It runs EVERY step before deciding:
the process exits non-zero if any step failed.

## Operation → feature-class map

Lettering follows the `openapi.yaml` header comment.

| Class | operationId | Method + path | Demos |
| --- | --- | --- | --- |
| a | `createWidget` | POST `/widgets` | JSON request/response + `application/problem+json` error (fixture 01, §8). |
| b | `createSession` | POST `/sessions` | Urlencoded form body answering 201 JSON with required Location + optional ETag headers (fixture 10, §15/§16). |
| c | `putNote` | PUT `/notes/{id}` | Bounded `text/plain` body, 204 no-body reply (§5.2/§44). |
| d | `putObject` / `getObject` | PUT/GET `/objects/{id}` | Octet-stream chunk-wise upload/download with typed ETag + int64 Content-Length headers (§9/§10/§15). |
| e | `getThumbnail` | GET `/thumbnails/{id}` | Exact `application/json` beside `image/*` wildcard on ONE status; exact entry wins at runtime (§5.10, §22). Demo ids: `meta-json` serves JSON metadata, `png-blob` the wildcard PNG branch. |
| f | `uploadDocument` | POST `/documents` | Multipart: bounded JSON metadata + optional repeated textual `tags` + EXACTLY ONE streaming binary part (§17/§17.1). |
| g | `streamEvents` | GET `/events` | Server-sent events decoded item-by-item (§18–§20). |
| g | `exportRecords` | GET `/records/export` | NDJSON response stream (§19). |
| g | `pushMetrics` | POST `/metrics` | json-seq as REQUEST body, drained item-by-item into a 202 ack (§18.1). |
| g | `exportMetrics` | GET `/metrics/export` | json-seq (RFC 7464) as RESPONSE stream (§20). |
| h | `postVendorDocument` | POST `/vendor-documents` | Unknown vendor media type kept as raw streaming fallback in BOTH directions; codecs all OFF (§21/§5.9/§45). Demo echoes the body verbatim. |
| i | `probeStatus` | GET `/status-probes/{id}` | Literal `'200'` beats `'2XX'`; `'4XX'` and `default` close the space (§23/§24). Demo dispatch by id prefix: `ok*` → 200, `2xx*` → 202 envelope, `4xx*` → 409 problem, anything else → 599 through `default`. |
| j | `deleteTask` | DELETE `/tasks/{id}` | 204 No Content (demo: only id `t-1` succeeds, else 404 problem) (§14/§35). |
| j | `getWidget` | GET `/widgets/{id}` | Plain JSON fetch with bare 404 (no content documented). |
| j | `headWidget` | HEAD `/widgets/{id}` | Header-only probe: required ETag + Content-Length, NEVER a body (§35). |
| k | `echoNote` | POST `/echo-note` | Optional body ≠ null: absent (`None`) vs JSON `null` (`Some(None)`) vs value (`Some(Some(..))`). Demo mirrors injectively: absent → `[absent]`, null → null, value → itself. Nullable properties also on `AuditEntry.metadata` and `MatrixRecord`'s cells (§26/§27). |
| l | `createAccount` | POST `/accounts` | Request targets the `Write` view, response the `Read` view (writeOnly `password` structurally cannot surface) (companion §5). |
| l | `listAuditEntries` | GET `/audit/{id}` | Read view of `AuditEntry`: readOnly `createdAt` served, writeOnly `draftNote` absent at compile time. |
| l | `syncRecord` | PUT `/synced` | Mixed readOnly AND writeOnly fields — neither direction lossless; trait takes the view. |
| m | `createPet` | POST `/pets` | `oneOf` WITH explicit discriminator mapping (contradictory const tags) answered by an `allOf` field-wise merge (`FullWidget`) (companion §4). |
| m | `createRecord` | POST `/records` | Presence/nullability matrix (all four cells) + `StringStatus` enum on a plain JSON roundtrip (companion §2.1). |

## Smoke test

```sh
cargo test -p kitchen-sink-client -- --ignored
```

Spawns the SAME demo router (from the server crate, wired in as a
dev-dependency of the smoke test only) on an ephemeral loopback port over
real TCP, drives the identical sweep the client binary runs, and asserts:
every step passes AND every documented operation ran exactly as often as
intended (`getThumbnail` twice, `probeStatus` four times, `echoNote` three
times for absent/null/value, everything else once — a fixed 28-step
itinerary). Gated behind `#[ignore]` so plain `cargo test` stays hermetic.

## Regeneration

Generated artifacts are committed and MUST stay byte-stable: CI's determinism
job re-runs the full pipeline repeatedly and byte-compares every run against
each other AND against every committed file across the three crates (main
spec §50 tests 38–39); ANY drift fails. Any diagnostic — even a Warning —
fails too, since none are expected for this document.

After editing `openapi.yaml`, either refresh the snapshots:

```sh
KITCHEN_SINK_GENERATED_UPDATE=1 cargo test -p kitchen-sink-models --test determinism
```

or regenerate them exactly as committed, with the normal CLI (source-only;
manifests stay hand-maintained):

```sh
openapi-to-rust examples/kitchen-sink/openapi.yaml \
  --generate types \
  --output-dir examples/kitchen-sink/models/generated

openapi-to-rust examples/kitchen-sink/openapi.yaml \
  --generate client --types-path kitchen_sink_models \
  --output-dir examples/kitchen-sink/client/generated

openapi-to-rust examples/kitchen-sink/openapi.yaml \
  --generate server --types-path kitchen_sink_models \
  --output-dir examples/kitchen-sink/server/generated
```

Generated files must NEVER be hand-edited: problems in generator behavior get
reported and fixed in the generator, never patched around in the artifacts.

## Streaming and boundedness guarantees on display

Nothing in the demos ever aggregates a potentially unbounded payload: the
octet-stream upload/download, the multipart binary part, and the vendor echo
all move strictly chunk-wise (main spec §32); SSE, NDJSON, and json-seq
streams are consumed item-by-item (§18–§20). Structured JSON/form bodies
encode and decode under finite bounds — overflow discards partial output,
fires the hook, and emits a fixed empty 500 (§34/§34.1) — while pre-handler
protocol rejections stay outside the documented enums (§39). HEAD probes are
header-only unit responses (§35), and the readOnly/writeOnly views show how
directional schemas keep each direction honest. These are precisely the
patterns §49 forbids violating: no `bytes()`, no `to_bytes(.., usize::MAX)`,
no unbounded serialization anywhere on the wire paths.

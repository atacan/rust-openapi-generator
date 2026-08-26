# Changelog

## Unreleased

### Selective artifact generation (`--generate` / `--types-path`, D-impl-selective-artifacts)

- New user-facing generation mode in the `openapi-to-rust` CLI:
  `openapi-to-rust <document> [--generate …] [--types-path …] [--out-dir …]`
  writes source artifacts into a directory without any configuration file.
  `--generate` accepts `types`, `client`, `server` (repeated and
  comma-separated forms are equivalent; `all` is shorthand for the three);
  omitting it preserves the historical all-in-one output byte-for-byte.
  The existing `--dump` mode is unchanged.
- `types` is the WHOLE shared schema surface: both `models.rs` and the
  directional read/write views of `views.rs` (companion §5) — there are no
  separate public selections for the two modules.
- `--types-path <RUST_PATH>` names an externally generated shared-types base
  namespace (its `models`/`views` modules sit under it), enabling the split
  workspace layout: one types crate, one Reqwest client crate, one Axum
  server crate, all referencing a single Rust identity per schema type. It
  is a Rust path, not a Cargo package name (`api-types` → `api_types`);
  obvious mistakes (kebab-case, `/`, dangling `::`) are rejected up front.
  Validation: client/server without local `types` require it; combining it
  with a same-invocation `types` selection is rejected as ambiguous;
  repeated selections deduplicate deterministically and argument order never
  affects emitted bytes.
- Library: new `codegen::config::{TypesLocation, CodegenConfig}` plus
  `generate_client_with_config`/`generate_server_with_config`; the emitters
  render the configured import prefixes directly (no post-generation string
  replacement). The sibling default keeps every existing caller and snapshot
  byte-identical; the import sort key now mirrors rustfmt's full ordering
  (keyword paths, then `::`-prefixed extern paths, then plain paths).

### `examples/large-upload` — bounded-memory streaming demonstration

- New workspace-member example crate around a minimal two-path OpenAPI 3.1.0 document
  (`PUT /blobs/{id}` with `application/octet-stream`, `PUT /audio/{id}` with `audio/wav` —
  the generator classifies both identically as raw streaming bodies): each answers a small
  JSON `UploadReceipt` ({bytes_received, sha256}) so full-payload handling is proven
  without re-downloading.
- Committed deterministic generated artifacts + determinism test
  (`LARGE_UPLOAD_GENERATED_UPDATE=1` refreshes), mirroring the kitchen-sink conventions;
  the CI determinism job now also regenerates `-p large-upload`.
- Runnable demos with TWO server modes: DISK mode (request body persisted chunk-by-chunk
  under a temp dir while an incremental SHA-256 digests it) and PROXY mode (`--proxy-url`
  forwards the inbound stream verbatim via `reqwest::Body::wrap_stream` — zero buffering);
  the client synthesizes a deterministic WAV file (default 1024 MiB) and streams it through
  BOTH media types, verifying both receipts. An `#[ignore]`-gated smoke test covers the
  disk round trip and a two-hop proxy chain over real TCP.
- In-process memory evidence: a shared monitor module samples process RSS every 50 ms
  (`memory-stats`) and reads the kernel high-water mark at exit (getrusage `ru_maxrss`);
  both binaries print paired progress/RSS lines during transfers and fail non-zero when
  peak − baseline exceeds 32 MiB (override via `LARGE_UPLOAD_MAX_RSS_DELTA_MIB`).
  Measured on macOS: 2 GiB transferred per run with peak deltas of ~1.5 MiB (client) and
  ~3–4 MiB (server/proxy).

### `examples/kitchen-sink` — end-to-end example package

- New workspace-member example crate built around ONE OpenAPI 3.1.0 union document
  (`examples/kitchen-sink/openapi.yaml`, 22 operations): every feature class of the fixture
  corpus in a single API — JSON/problem+json, urlencoded forms, bounded `text/plain`,
  chunk-wise octet-stream upload+download with typed ETag/Content-Length, the `image/*`
  wildcard beside JSON on one status, multipart with exactly one streaming binary part,
  SSE/NDJSON/json-seq in both directions, unknown-vendor raw fallback, status-range
  precedence + `default`, 204/HEAD no-body probes, optional-body-vs-null, readOnly/writeOnly
  views in matching positions, and oneOf-discriminator/allOf/string-enum composition.
- Committed deterministic output: the four generated artifacts plus the emitted §3.1
  manifest artifact live under `generated/` and are compiled UNMODIFIED via `include!`;
  `tests/determinism.rs` double-generates and byte-compares against them
  (`KITCHEN_SINK_GENERATED_UPDATE=1` refreshes, mirroring the golden-harness convention).
- Runnable demos: thin server/client binaries sharing a 28-step full-operation sweep over
  real TCP, with the same sweep asserted by an `#[ignore]`-gated smoke test
  (`cargo test -p kitchen-sink -- --ignored`).
- CI guard: the determinism job now regenerates `-p kitchen-sink` and fails on any drift in
  `examples/kitchen-sink`.
- Recorded `D-header-field-shape` in DECISIONS.md: optional-header structs use plain domain
  fields (§48 option 2); conversion failures follow the §34.1 fallback path (hook + fixed
  empty 500); checked constructors stay reserved for required-fallible headers.

## v0.1.0-alpha.1 — 2026-08-24

All four implementation milestones of `rust-openapi-axum-reqwest-codegen-spec.md` §52,
plus post-milestone hardening. Tagged on the generator + `openapi-support` runtime with
the embedded toolchain tuple: Axum 0.8 / Reqwest 0.12 / http 1 / bytes 1, MSRV 1.85.

### Phase 0 — invariants, IR, foundations

- Support runtime: `BodyLimits` (§33), fail-fast bounded collection (`collect_limited`,
  decoded-byte accounting contract per §30.2), bounded serializers
  (`serialize_json_limited`, `serialize_form_limited` via purpose-built streaming form
  encoder; counting-writer contract), `EncodeTooLarge`, encode-overflow and
  stream-failure hook traits (§34.1/§40), `ProtocolRejection` + canonical 400/413/415/422
  mapping outside documented enums (§39), truncated-stream decode error types (§40),
  `OptionalField<T>` + presence-aware adapters implementing all four companion §2.1
  presence/nullability cells, identity-only request content-coding guard (§30.4).
- Document loader: OpenAPI 3.0/3.1/3.2 detection, version-aware `$ref` sibling rules,
  external relative-file refs with memoization, cycle policy (property→Boxed via
  SCC-precise pass, container→direct, unbroken self-containment rejected), inline depth
  cap, 1xx rejection at parse time.
- Normalizer: allOf intersection-first (object merges with conflict errors, scalar
  constraint intersection), strict oneOf / conservative anyOf with static mutual-
  exclusivity proofs (discriminator mapping requires branch-side tag constants),
  raw/value fallback default, servers precedence, deterministic naming pipeline.
- All companion §11 Open items closed in DECISIONS.md before codegen began.
- Deterministic golden harness: fixtures → byte-exact snapshots, double-generation gates.

### Phase 1 — core protocol shapes

- Shared model generation: companion §2.1 matrix cell-for-cell, string/integer/mixed
  enums (manual numeric discriminants), proven-oneOf choice enums made decode-safe by
  structural single-variant const enums, RawValue fallback newtypes, deny/flatten
  policies, cycle-precise Box edges.
- Directional view types (companion §5) generated for readOnly/writeOnly models with
  asymmetric conversions that never fabricate values.
- Full companion §6 parameter matrix (7 styles × explode × locations, deepObject,
  allowReserved, cookie-as-header).
- Reqwest client generation: bounded encode with `BodyTooLarge` before send (§34.2),
  purpose-split collect limits, §28 Content-Type dispatch incl. malformed/duplicate
  headers and charset policy, empty-JSON rejection (§28.3), streaming wrappers, redirects
  off by default with opt-in hook, per-operation server bases with variable substitution.
- Axum server/router generation: Mode A traits, §38 pre-handler pipeline (identity-only
  content coding → parameter decoding → Content-Type state machine → body acquisition),
  rejections outside documented enums, peek-and-preserve optional-body detection (§28.2),
  DefaultBodyLimit wiring, §34.1 fallback 500 + hook, checked range constructors.
- Conformance crate: differential client↔server round trips over real TCP, including an
  8 MiB lazy-chunked upload proof; compile-conformance for every fixture's emitted crate.

### Phase 2 — body-rich APIs

- URL-encoded forms end-to-end (strict pairs parser + serde map deserializer;
  413-before-parse gate).
- Typed response headers with required-header protocol errors and §48 checked
  constructors.
- Multipart: incremental RFC 2045/7578 framing engine enforcing §17.1 cardinality limits
  incrementally; client `from_file` streaming builders; server single-pass collector with
  live streaming parts; wire-arrival-based requiredness enforcement.
- Bucket-2 runtime validation (companion §9): ReDoS-safe bounded pattern matcher,
  hand-rolled format checks, emitted `validate_request()` wired post-decode →
  `SchemaViolation` 422; clients lenient by default.
- Wildcard negotiation completion incl. `x-rust-body: stream` override.

### Phase 3 — streaming structured protocols

- SSE (WHATWG line endings + strict §18.2 data validation), NDJSON, JSON Text Sequences:
  incremental bounded decoders/encoders, per-record limits, both directions.
- §40 committed-stream contract: hook fires then body terminates abruptly; client-side
  premature-end classification surfaces `Truncated` distinct from clean EOF.
- Cancellation/backpressure/truncation conformance proofs over real TCP.

### Phase 4 — extensibility

- Media-type codec plugin interface (compile-time registry, default off, raw fallback):
  XML, CBOR, MessagePack built-ins with bounded collect-then-parse decode and fail-fast
  encode; codec deps emitted only when enabled.
- Representation overrides (`ForceStreaming`) with override > plugin > default precedence.
- Replayable uploads: `<op>_replaying(body_factory, policy)` twins; pre-response-only
  retryable classification; no implicit retries anywhere (§31).
- Real 10 GiB synthetic upload/download passthrough proofs (ignore-gated), backpressure
  bound, prompt cancellation.

### Hardening

- GitHub Actions CI: fmt/clippy (incl. feature matrix)/test/determinism/MSRV jobs;
  SHA-pinned actions.
- Emitted-crate manifest generation (§3.1): caret-only reqs, pinned rust-version, feature
  graph mirroring support, codec fragments only when enabled.
- cargo-fuzz targets for every parser surface (ASan-clean local campaign) with curated
  seed corpus.
- trybuild negative fixtures proving §49 misuse fails to compile against generated crates.
- §50 test 52 pinned both halves after the §39 Codec exception amendment (50081c8).

### Post-alpha hardening (F1–F4)

Closed after the `v0.1.0-alpha.1` review, one gated work package each:

- **Directional views consumed by operation codecs** (`f81fac0`): request codecs
  encode/decode `<M>Write` views and response codecs `<M>Read` views wherever models carry
  readOnly/writeOnly fields; the router auto-converts to shared models only when lossless
  and never fabricates values. Fixture 08 gained real operations; t50a/b/c conformance
  proofs pin wire-level directionality and per-direction requiredness (companion §5,
  §50 test 50 runtime half). Zero churn on view-less fixtures.
- **Opt-in response decompression with decoded-byte accounting** (`790feff`, F2): gzip /
  Brotli / Zstandard features forward to Reqwest's codings; limits count DECODED bytes,
  proven at support level and end-to-end with a hostile gzipped origin whose decoded body
  exceeds the lowered limit (§30.2, §50 test 32). Default stays all-OFF with byte-stable
  manifests.
- **HEAD header-only round-trip proof** (`790feff`, F3): fixture 17 pins typed-header-only
  variants on both sides; zero body bytes are ever read or exposed (§35, §50 test 34).
  Emission fixes surfaced by the new fixtures: rustfmt-canonical send chains, variant
  literal layouts, and struct-pattern breaking.
- **MSRV clippy gate restored** (`44eed98`, F4): elided-lifetime sites fixed and a
  generated-code `let_and_return` eliminated, so rustc 1.85's clippy passes `-D warnings`;
  the CI MSRV job now lints alongside tests.

Standing decision (closed by design): record-framed request streams have no `_replaying`
twins — see DECISIONS.md `D-impl-retry`.

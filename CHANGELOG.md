# Changelog

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

### Known gaps at this tag (tracked for v0.1.0)

- Operation codecs do not yet consume directional views: shared models ride both wire
  directions, so `readOnly` fields would be sent on requests today (companion §5 /
  §50 test 50 runtime half). Scheduled.
- Transparent response decompression (§30.2) is not yet implemented; limits count wire
  bytes when content coding is present. Scheduled together with a HEAD round-trip
  conformance test (§35 / §50 tests 32+34).
- MSRV CI job is test-only until seven elided-lifetime sites are cleaned up for
  rustc-1.85 clippy. Scheduled.
- Record-framed request streams have no `_replaying` twins (recorded decision,
  `D-impl-retry`). Closed without action by design.

# DECISIONS

Implementation decisions recorded during phased implementation. Format: section reference,
decision, rationale. Companion "Open" items affecting the IR or generated public types are closed
here during Phase 0 per companion §11. The two specification documents remain authoritative;
nothing below may contradict them.

---

## A. Companion Open items closed in Phase 0 (companion §11)

### D-§2 JSON Schema 3.x keyword coverage matrix (companion §2)

**Decision.** Keywords are classified into three buckets:

1. **Type-shaping → generated code:** `type`, `properties` + `required`, `additionalProperties`
   (all §4.4 forms), `items`, `prefixItems` (tuple types), `enum`, `const`, composition keywords
   (`allOf`/`oneOf`/`anyOf`), `$ref`, `nullable` (3.0 normalization), `readOnly`/`writeOnly`
   (directional views, companion §5), `default` (serde default), `discriminator`,
   `title`/`description` (doc comments), `deprecated` (doc attribute only).
2. **Validation-only metadata** (retained in IR; enforced on server requests per companion §9;
   lenient on client decode): `pattern`, `minLength`, `maxLength`, `minimum`, `maximum`,
   `exclusiveMinimum`/`exclusiveMaximum`, `multipleOf`, `minItems`, `maxItems`, `uniqueItems`,
   `minProperties`, `maxProperties`, `contains` + `minContains` + `maxContains`,
   `patternProperties`, `format` (unless the typed-format feature is enabled), `contentEncoding`,
   `contentMediaType`, `example`/`examples`.
3. **Recorded with a generation diagnostic when they alter semantics beyond the v1 model:**
   `unevaluatedProperties`, `unevaluatedItems`, `if`/`then`/`else`, `not`, `dependentSchemas`,
   `dependentRequired`. A composed type containing any of these falls back to the raw/value
   representation or is a generation error listing schema paths (same philosophy as companion
   §4.1/§4.2). Standalone (non-composed) object schemas using only `unevaluatedProperties: false`
   behave exactly like `additionalProperties: false`.

**Rationale.** Conservative fallback over silent ignoring; matches the lossless-representation
philosophy already Decided in companion §4.1/§4.2.

### D-§2.1 Tri-state PATCH mode shape (companion §2.1 note 2)

**Decision.** v1 ships exactly the four-cell presence/nullability matrix. No tri-state mode.
Optional+nullable stays conflated (`Option<T>` for absence and explicit null).

**Rationale.** The matrix covers the contract meanings stated in the spec; a tri-state wrapper is
additive later behind configuration without changing existing generated types. Revisit when a
concrete fixture requires absent-vs-null distinction for an optional+nullable property.

### D-§3 Anchor-style refs (companion §3)

**Decision.** v1 resolves only RFC 6901 JSON Pointer fragments against the local document or
relative external files. `$anchor`, `$dynamicAnchor`, `$dynamicRef`, and base-URI rebasing via a
non-empty `$id` are unsupported: encountering one is a generation diagnostic error naming the
schema path — never a silent ignore.

Also adopted from Proposed: external file references via relative paths are supported in v1;
remote URL resolution requires an explicit opt-in fetcher (default off → diagnostic if
encountered), keeping generation deterministic.

**Rationale.** Deterministic resolution needs no fetch state or URL normalization; anchor support
can be added later without IR changes.

### D-§4.4 Free-form object representation (companion §4.4)

**Decision.**

| Schema | Generated representation |
|---|---|
| free-form `type: object` (no properties, no constraints) | `serde_json::Map<String, serde_json::Value>` |
| `{}` / no `type` / scalar-typed unconstrained | `serde_json::Value` |

`serde_json::Map` is used **without** `preserve_order`: iteration order is key-sorted, which keeps
generated output byte-deterministic (JSON member order is not semantically meaningful).

**Rationale.** `Map` preserves the "must be an object" contract that `Value` would erase; sorting
keys satisfies main-spec §50 determinism requirements.

### D-§6 Header parameter name normalization collisions (companion §6)

**Decision.** Header parameters and typed response headers run through the standard naming
pipeline: snake_case fields preserving `-`/`_` word boundaries. Distinct wire headers normalizing
to identical field names receive deterministic numeric suffixes ordered by first document position
(companion §10 rule). Wire header names are always carried separately (header literal / serde
rename) — renaming never changes protocol bytes. Names normalizing to Rust keywords get `_`
suffix per companion §10.

**Rationale.** One collision rule everywhere; protocol bytes stay faithful to the document.

### D-§10 Reserved-name collisions against framework types (companion §10)

**Decision.** Adopted as proposed: user-facing operation/schema names are never renamed because of
collisions with framework/support names (`Client`, `Response`, `Body`, …). Generated operation
code lives in tag modules and references framework/support items through fully qualified paths
(`axum::body::Body`, `::openapi_support::…`) so shadowing cannot occur. If a schema name still
collides with another item in the same module scope after module placement, numeric suffixes
ordered by document position apply (companion §10).

**Rationale.** Keeps public API names faithful to the OpenAPI document.

---

## B. Main-spec §51 recommendations adopted

### D-§51.1 Binary client response wrapper
Generated status wrapper structs carry typed documented headers plus the owned raw
`reqwest::Response` (Example 3 / §32 shape). Raw-response-only output is not generated.

### D-§51.2 Always enum
Every operation generates a response enum even when exactly one status is documented.

### D-§51.3 Owning request content enums
Operation request content enums own their values; convenience methods taking `&T` are provided
only for single-content JSON operations.

### D-§51.4 Multipart server API direction
Operation-specific streaming wrapper (§17 Output B); sequential streaming semantics outrank
struct-like ergonomics. Detailed design is a Phase 2 deliverable.

### D-§51.5 Initial codec scope
Exactly the §51.5 list (JSON, plain text, form, raw/binary, multipart/form-data, SSE, NDJSON,
JSON Sequence). All other media types fall back to raw streaming until Phase 4 codec plugins.

---

## C. Implementation choices required before Phase 0 code

### D-impl-forms Form bounded serialization implementation
`serialize_form_limited` is implemented in `openapi-support` by a purpose-built streaming form
serializer driving the counting writer (fail-fast per §34). `serde_urlencoded` is never used on
encode paths (its `to_string` buffers unboundedly). Server-side decode uses axum's `Form`
extractor only under §16 conditions; client-side form decode is deferred until needed.

### D-impl-limits Default body limits (main spec §33)
Generator defaults (all overridable through generator configuration):

| Limit | Default |
|---|---|
| `structured_request_bytes` | 8 MiB |
| `structured_response_bytes` | 8 MiB |
| `error_response_bytes` | 1 MiB |
| `structured_encode_bytes` | 8 MiB |
| `text_body_bytes` | 8 MiB |
| `multipart_scalar_part_bytes` | 1 MiB |
| `max_stream_record_bytes` | 1 MiB |
| `max_multipart_parts` | 1000 |
| `max_part_header_bytes` | 64 KiB |
| `max_field_name_bytes` | 256 |
| `max_file_name_bytes` | 1024 |
| `peek_buffer_bytes` | 8 KiB |
| `max_multipart_depth` | 4 |

### D-impl-crate Runtime crate identity
Package `openapi-support`, lib `openapi_support`. Emitted generated crates depend on it with caret
version requirements matching the release's embedded tuple (main spec §3.1); path dependencies are
never emitted.

### D-impl-hooks Hook traits
Object-safe traits in `openapi-support` with no-op default implementations:

```rust
pub trait EncodeOverflowHook: Send + Sync {
    fn on_encode_overflow(&self, operation_id: &str, variant: &str, limit: usize);
}

pub trait StreamFailureHook: Send + Sync {
    fn on_stream_failure(&self, operation_id: &str, error: &(dyn std::error::Error + Send + Sync));
}
```

The stream-failure hook takes a `dyn Error` reference rather than a concrete error type so
Phase 3 codec error enums plug in without changing the support-crate public API.

Default installation is the silent no-op; client/server builders accept custom hooks (main spec
§34.1 step 3, §40 step 3).

### D-impl-oneoffallback Default fallback for unprovable `oneOf`/`anyOf` disjointness
Companion §4.2 offers raw/value representation or generation error as configuration alternatives.
Default = **raw/value representation carrying validation metadata** (generation succeeds, verdicts
stay exact). Configuration may switch to hard error.

### D-impl-servers-empty Empty `servers` arrays at operation/path level
Companion §8 defines the absent-or-empty rule explicitly only for the root-level array. Decision:
a present-but-empty `servers: []` at operation or path level also falls through to the next level
(op empty → path array → root array → `/`). Rationale: mirrors the root-level precedent; an empty
override that disables all bases has no coherent meaning for client generation.

### D-impl-allof-additional Schema-valued `additionalProperties` across `allOf` members
Companion §4.1 requires lossless intersection or fallback/error. Intersecting two distinct
schema-valued `additionalProperties` maps is not representable losslessly in the v1 typed model,
so mixed/conflicting schema-valued `additionalProperties` across `allOf` members falls back to
the raw/value representation with a Warning diagnostic (same default philosophy as
D-impl-oneoffallback). Property-level constraint conflicts remain generation Errors exactly as
§4.1 states.

### D-impl-boxing Property-edge heap indirection is cycle-precise
Companion §3: "Recursion through properties generates heap-indirected types (`Box<T>`)". Decision:
`Indirection::Boxed` is recorded **only** on property edges whose target can reach back to the
source through property/composition-only paths (i.e., edges that close a property-recursion
cycle, computed by an SCC pass after loading); acyclic property edges stay direct, and container
edges are always direct. Boxing every property edge would satisfy the letter of the recursion
rule but pollutes every generated struct; precision here keeps generated public types faithful.

### D-impl-msrv-pins MSRV 1.85 transitive dependency pins
The MSRV job against the pinned §3.1 tuple surfaced that current `url` → `idna` → `idna_adapter`
1.2.1 resolves `icu_*` 2.x, which requires rustc 1.88 — above the pinned MSRV 1.85. Decision: the
workspace lockfile pins `url = 2.5.4`, `idna = 1.0.3`, `idna_adapter = 1.2.0` (icu 1.x family) so
the pinned tuple builds cleanly; when Phase 1 emits generated-crate manifests, the generator ships
a documented transitive-pin table (or equivalent guidance) so MSRV consumers resolve compatible
versions deterministically rather than discovering breakage at lock time.

---

## D. Phase 1 implementation choices

### D-impl-clienterror-location Single authoritative `ClientError`
`ClientError` and `BodyLimitDirection` (main spec §36, "the single authoritative definition") live
in `openapi-support` behind the `client` feature and generated clients re-export them. Generated
code never invents variants outside §36.

### D-impl-views-phase1 Directional views land in Phase 1
Companion §5 is Decided and shapes generated public types, so read/write view generation ships
with model codegen in Phase 1 rather than being retrofitted in Phase 2. Models without
`readOnly`/`writeOnly` fields get no view types (identity). Addendum: §5 counts "has a default"
as lossless for view→shared reconstruction; because Phase 1 models carry schema defaults as doc
comments only (see D-impl-runtime-validation-timing), a declared default does not yet enable a
reconstruction conversion — only genuinely optional fields do. This is strictly conservative:
it can suppress a conversion, never fabricate a value; when defaults are materialized the
conversions widen automatically.

### D-impl-async-trait Generated server traits use `async-trait`
Spec examples annotate API traits with `#[async_trait::async_trait]`; dyn-dispatchable traits are
required for router construction over an application-supplied trait object. Dependency authorized:
`async-trait = "0.1"`.

### D-impl-codegen-emission String-template emission verified by rustfmt
Codegen emits source as deterministic strings (no syn/quote dependency); the verification suite
asserts emitted output is `rustfmt`-clean (main spec §50 test 40) by invoking the installed
`rustfmt` binary in tests.

### D-impl-server-mode-a Mode A is the generated server default
Trait methods return the documented response enum directly (main spec §37 Mode A). Mode B
(`AppError` hook) is deferred until a phase that scopes it.

### D-impl-charset-rejection Unsupported charsets map to `MalformedBody`
Main spec §28.4: textual bodies with a `charset` outside the UTF-8 family yield decode errors.
Server-side this is a `ProtocolRejection` with kind `MalformedBody` (400); client-side it surfaces
as `ClientError::Decode`.

### D-impl-relative-servers Relative server URLs stay relative in generated defaults
Resolving relative `servers` entries against the declaring document's file location would embed
machine-specific paths into generated output and violate byte-for-byte reproducibility (main spec
§50 test 39). Decision: absolute server URLs are baked as the default base; relative entries are
emitted verbatim and the generated `ClientBuilder` requires an explicit base URL when no absolute
default exists. Server-variable substitution follows companion §8 (builder parameters, declared
defaults, enum validation).

### D-impl-runtime-validation-timing Bucket-2 runtime validators ship in Phase 2
Phase 1 enforces structural validation only (types, required fields — via Serde decode failures,
translated per §39: syntax → `MalformedBody` 400, data errors → `SchemaViolation` 422).
Constraint metadata (pattern/min/max/format, companion §9 bucket 2) is carried in the IR and
doc-comments but enforced at runtime starting Phase 2.

### D-impl-flatten-map-deterministic Schema-valued additionalProperties uses `BTreeMap`
`#[serde(flatten)]` maps serialize in iteration order; `BTreeMap<String, T>` keeps runtime wire
bytes deterministic (sorted keys) consistent with the D-§4.4 sorted-key choice.

### D-impl-typed-headers-phase2 Typed response-header fields ship in Phase 2
Phase 1 streaming/raw client variants own the `reqwest::Response` through a generated status
wrapper (`into_bytes_stream()` convenience, main spec §32/§51.1) without typed documented-header
fields; typed header structs (`GetArtifact200Headers`, §4 naming table) land in Phase 2 together
with header parameters' collision rules (companion §6/D-§6).

### D-impl-singlefile-layout Single-file generated modules until multi-tag fixtures exist
Phase 1 emits one `models.rs` / `views.rs` / `client.rs` / `server.rs` per generated crate. The
per-tag module layout of spec §3 (`client/<tag>.rs`) arrives when fixture coverage exercises tags;
the naming pipeline's tag sanitation is already in place.

### D-impl-param-matrix-phase1 Full companion §6 matrix ships in Phase 1
The complete style × explode × location matrix (incl. deepObject, allowReserved, cookie-as-header,
no jar) lives in `openapi_support::params` from Phase 1 because both generated clients (encode)
and generated routers (pre-handler syntax validation, §38) consume it; generation-time rejection
of invalid style/location combinations happens in the generator per companion §6.

### D-impl-forms-phase2 Forms and multipart are Phase 2 deliverables
Per main spec §52, URL-encoded forms, multipart, typed response headers, wildcard negotiation,
and status-range/default handling beyond the Phase 1 enum shapes arrive in Phase 2. Phase 1
server bodies: JSON family (bounded decode), plain text (bounded String), binary/raw/unknown
(streaming passthrough).

### D-impl-multipart-order Wire-arrival enforcement for multipart requiredness
§17.1 defines missing-required as "the part stream ends without them", but sequential streaming
(§51.4) means parts behind a live binary part cannot be judged before the application drains it.
Decision: multipart validation is wire-arrival-based — scalar/JSON parts arriving before the
first binary part validate pre-handler (missing → 422, handler never invoked); required parts
still outstanding at binary handoff become `pending_required` on the emitted live part and
surface as a terminal `SchemaViolation` from `next_chunk` at clean end-of-message (§38's
"application-owned tail"). Trailing declared scalar/JSON parts are decoded onto a generated
`<Op>TrailingParts` carrier rather than drained.

### D-impl-multipart-single-binary At most one binary part per multipart body (v1)
Sequential streaming gives exactly one live-part slot; a queue of parked unbounded streams would
either buffer payloads or complicate the public API beyond v1 scope. Documents declaring more
than one binary field get a generation Error diagnostic (`multipart_schema_unsupported`).
Repeated (array) binary fields are likewise rejected, symmetrically with the client, which
cannot clone streaming bodies. Revisit when a concrete fixture needs multi-file uploads.

### D-impl-x-rust-body-stream `x-rust-body: stream` forces raw streaming for plain-text entries
Main spec §44 says a `text/plain` + `schema: {type: string}` entry maps to bounded `String` by
default and that unbounded plain-text streaming SHOULD be available through an extension such as
`x-rust-body: stream`. Decision: the extension is honored on media-type entries whose class is the
§5.2 plain-text family (text/plain, text/html, text/csv, text/markdown, application/sql); planning
re-classes such an entry into the streaming family for BOTH directions, while the media-type
literal — and therefore runtime Content-Type matching, the operation's Accept contribution, and
the `TextPlain` variant name — stays verbatim. Any other `x-rust-body` value remains an ignored
vendor extension. JSON-family entries are deliberately NOT overridable this way: their bounded

---

## E. Phase 3 implementation choices

### D-impl-sse-framing SSE framing follows WHATWG with strict data validation
Line terminators: CRLF, LF, and CR are all accepted between lines (WHATWG event-stream
convention), while the §18.2 contract holds otherwise — `data:` joined with `\n` before one JSON
parse, `id:`/`event:` ignored by default, `retry:` surfaced only through configuration, comment
lines ignored, BOM stripped once at stream start. An event without `data:` is skipped per WHATWG;
malformed JSON yields `SseDecodeError::MalformedJson` and TERMINATES the stream (fail-fast, never
skip-and-continue), per §18.2 "without collecting the rest".

### D-impl-ndjson-lines NDJSON blank-line policy
A single trailing line terminator is part of the format; interior empty lines are
`MalformedJson`. EOF exactly at a record boundary is clean end-of-stream; EOF mid-record is
`Truncated` (distinct from clean EOF per §40).

### D-impl-jsonseq-eof JSON Text Sequences: no truncated-record recovery
RFC 7464 allows parsers to recover a truncated final record ("MAY"). Decision: v1 does NOT
recover — EOF after RS without a terminating LF yields `JsonSeqDecodeError::Truncated`, keeping
truncation observable rather than guessed away. A record whose first byte is not RS yields
`MissingRecordSeparator`.

### D-impl-stream-item-bounds Per-item bounds for streamed encodes and decodes
Decoded records are bounded by `max_stream_record_bytes` on decode; server-side per-item ENCODE
also enforces `max_stream_record_bytes` (not `structured_encode_bytes`) since each item is an
independently bounded document. Overflow on a committed stream follows §40 (terminate + hook),
while pre-commit overflow on request encoding follows §34.2.

### D-impl-request-direction-streams Streaming structured media run in both directions
The §6 summary table admits SSE/NDJSON/JSON-seq REQUEST bodies; generated clients send them via
chunk-wrapped encoded item streams (per-item bound enforced pre-send) and generated routers hand
handlers a typed item-stream wrapper whose decode errors surface as rejections before/during
consumption under the same wire-arrival philosophy as multipart (D-impl-multipart-order).
full-document decode is what produces the typed representation, and a raw JSON stream would erase
the type contract instead of relaxing its memory bound.

**Rationale.** Minimal-diff realization of §44: flipping the planned media class reuses the
existing streaming emitters byte-for-byte on both sides instead of threading a third representation
through every textual arm; boundedness is recovered by the application draining the stream.

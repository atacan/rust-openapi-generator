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

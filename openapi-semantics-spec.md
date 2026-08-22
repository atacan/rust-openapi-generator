# Rust OpenAPI Code Generation Specification
## Companion: OpenAPI Document Semantics

**Status:** Draft scaffold (Phase 0 dependency)
**Companion to:** [`rust-openapi-axum-reqwest-codegen-spec.md`](./rust-openapi-axum-reqwest-codegen-spec.md)

---

## 1. Purpose and scope

The main specification defines generated request/response bodies, status/content enums, streaming semantics, and contract-boundary rules. This companion defines everything below that layer which the parser/normalizer must decide **before** code generation:

- `$ref` resolution including external and cyclic references;
- OpenAPI 3.0 vs 3.1/3.2 normalization;
- schema composition (`allOf`, `oneOf`, `anyOf`) and discriminators;
- `additionalProperties`, free-form objects, string formats;
- read-only/write-only behavior;
- parameter serialization (`style`, `explode`, `deepObject`, cookies);
- security schemes;
- server/base-URL handling;
- naming collisions and Rust keywords.

Each subsection is marked **Decided**, **Proposed**, or **Open**. Nothing here may contradict the main specification; where interaction occurs, the main document's contract-boundary sections (28, 34, 39, 40) win.

---

## 2. Document versions and normalization

**Decided:** Inputs may be OpenAPI 3.0.x, 3.1.x, or 3.2.x. The parser normalizes everything into a version-agnostic internal IR whose semantics follow 3.1+.

Normalization table (initial):

| 3.0 construct | IR representation |
|---|---|
| `nullable: true` on a schema | null-capable union (`Option<T>` at property level) |
| boolean `exclusiveMinimum`/`exclusiveMaximum` | numeric comparison metadata |
| `example` (singular) | example metadata |
| `type: string, format: binary` | binary payload marker (main spec section 5.3) |
| deprecated `binary`/`file` type forms | normalized to string+binary |

**Open:** 3.1 JSON Schema keyword coverage matrix (`contains`, `unevaluatedProperties`, `prefixItems`, ...): which keywords affect generated types versus being validation-only metadata. Initial position: type-shaping keywords map to code; pure validation keywords attach as runtime-validation metadata per section 9.

---

## 3. `$ref` resolution

**Decided:**

- Reference targets: any component type the generator consumes (schemas, parameters, request bodies, responses, security schemes).
- JSON Pointer escaping (`~0`, `~1`) handled per RFC 6901.
- Sibling keys next to `$ref`: ignored with a warning (3.0 behavior); never merged.
- Cycle policy: the IR is graph-aware. Recursion through properties generates heap-indirected types (`Box<T>`); recursion broken by arrays/maps is direct. A schema requiring unbroken self-containment (a value must contain itself) is a generation error.
- Inline expansion depth cap (configurable) guards pathological nesting; hitting it is an error, not silent truncation.

**Proposed:** External file references via relative paths are supported in v1; remote URL resolution requires an explicit opt-in fetcher so generation stays deterministic.

**Open:** Anchor-style refs (`$id`/`$anchor`, 3.1) support scope.

---

## 4. Schema composition

### 4.1 `allOf`

**Proposed:** Members that are plain objects merge field-wise into one struct. Merge conflicts (same property, incompatible schemas) are generation errors; identical constraints collapse; `required` unions. Non-object members fall back to composition-by-nesting with `serde(flatten)`.

### 4.2 `oneOf` / `anyOf`

**Proposed:** Generated as Rust enums.

- With a `discriminator` (property name or mapping): internally tagged enum (`#[serde(tag = "...")]`), mapping entries become explicit variant names.
- Without: untagged enum (`#[serde(untagged)]`); if two variants can decode the same document, ordering follows declaration order and a warning is emitted.
- `anyOf` additionally permits intersection documents; initial position treats it identically to `oneOf` for typing purposes, documented as a lossy approximation.

### 4.3 Enumerations

**Proposed:** `enum` of constants with `string` values mapped through serde renames; integer enums use numeric discriminants; mixed-type enums generate a fallback raw-value variant rather than failing.

### 4.4 `additionalProperties`

**Decided:**

- `false` → `#[serde(deny_unknown_fields)]`;
- absent/`true` → unknown keys ignored;
- schema-valued → `#[serde(flatten)] additional: HashMap<String, T>` alongside named properties.

**Open:** Free-form object (`type: object` with no properties, no constraint) defaults to `serde_json::Value` vs `Map<String, Value>`.

### 4.5 String formats

**Proposed:** Default keeps `String`/primitives to stay dependency-light. Optional typed-format feature maps `date-time`/`date` to a chrono/time type and `uuid` to `uuid::Uuid` when enabled.

---

## 5. Read-only and write-only

**Proposed:** Shared models include all fields. `readOnly` fields carry `#[serde(skip_deserializing)]`-style directionality only in generated operation codecs, not in the shared model, preserving model reuse (main spec section 2.6). Strict directional views are a future opt-in.

---

## 6. Parameters

**Decided:** Full `style` × `explode` matrix implemented per OAS defaults (form+explode for query/cookie, simple for path/header). `deepObject` supported for query. Unknown style/location combinations are generation errors.

**Proposed:** Cookie parameters: client sends them as `Cookie` headers built from values; server extracts from the `Cookie` header without requiring a cookie jar. No automatic cookie store in v1.

**Open:** Header parameter name normalization (`X-Request-Id` vs generated field casing) collision rules with typed response headers.

---

## 7. Security schemes

**Proposed (minimal v1):**

- Client: credentials applied through a generated interceptor trait (`AuthProvider`) covering `apiKey` (header/query/cookie), HTTP bearer/basic, and OAuth2/OIDC token providers.
- Server: no automatic enforcement; generated extractors expose resolved credential material, leaving policy to application middleware.
- Security requirement unions ("OR") select the first satisfiable provider configured; "AND" groups require all.

---

## 8. Servers and base URLs

**Decided:** The first `servers` entry is the default base URL; all entries generate constructors. Server variables (`{region}`) become builder parameters with declared defaults and `enum` validation. Path parameters percent-encode using RFC 3986 unreserved set; query parameter serialization order is declaration order for deterministic output.

---

## 9. Runtime validation policy

**Proposed:** Schema constraints (patterns, min/max, format checks) attach to the IR as validation metadata. Defaults: server-side requests validate and reject via the main spec's `SchemaViolation` → `422` path; server responses trust application construction; client decoding is lenient by default with a strict mode behind configuration.

---

## 10. Naming

**Decided:**

- Rust keywords and empty identifiers get a `_` suffix (not raw identifiers) deterministically;
- casing: types `PascalCase`, methods/fields `snake_case`, preserving `operationId` word boundaries;
- collisions resolve with deterministic numeric suffixes ordered by document position (never hash-based, feeding the reproducibility requirements of main spec section 50);
- tags map to modules with the same sanitization rules.

**Open:** Reserved-name collisions against framework types (`Client`, `Response`, `Body`) — proposed prefixing with the tag/module namespace rather than renaming user-facing operation types.

---

## 11. Open question tracking

All **Open** items above block Phase 1 completion, not Phase 0 start. They should be promoted to Decided/Proposed with rationale as implementation feedback arrives, mirroring the milestone structure of main spec section 52.

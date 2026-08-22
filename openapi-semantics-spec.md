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

Callbacks, links, and webhooks are out of scope for both documents (main spec section 1). Each subsection below is marked **Decided**, **Proposed**, or **Open**. Nothing here may contradict the main specification; where interaction occurs, the main document's contract-boundary sections (28, 34, 39, 40) win.

---

## 2. Document versions and normalization

**Decided:** Inputs may be OpenAPI 3.0.x, 3.1.x, or 3.2.x. The parser normalizes everything into a version-agnostic internal IR whose semantics follow 3.1+.

Normalization table (initial):

| 3.0 construct | IR representation |
|---|---|
| `nullable: true` on a schema | nullability dimension recorded alongside the presence dimension (section 2.1) |
| boolean `exclusiveMinimum`/`exclusiveMaximum` | numeric comparison metadata |
| `example` (singular) | example metadata |
| `type: string, format: binary` | binary payload marker (main spec section 5.3) |
| deprecated `binary`/`file` type forms | normalized to string+binary |

### 2.1 Property presence versus value nullability

Serde's plain `Option<T>` conflates a missing property with an explicit JSON `null`. Because required-but-nullable and optional-but-non-nullable properties have different contract meanings, the IR records **two independent dimensions** per property — presence and nullability — and the generator maps them explicitly:

| Presence | Nullability | Generated representation |
|---|---|---|
| required | non-nullable | `T` — missing key and explicit `null` both fail deserialization |
| required | nullable | `Option<T>` decoded through a presence-aware adapter — explicit `null` yields `None`; a missing key is a schema violation |
| optional | non-nullable | support wrapper `OptionalField<T>` (`Absent` / `Present(T)`) — missing key yields `Absent`; explicit `null` is a decode error |
| optional | nullable | `Option<T>` — absence and `null` both yield `None` |

Notes:

- generated structs therefore cannot rely on bare derived `Option` behavior where the contract distinguishes the cases; presence-aware adapters and wrapper types live in the generated support module;
- the optional+nullable conflation matches most contracts; PATCH-style semantics that must distinguish "absent" from "explicit null" can enable a tri-state mode (**Open**: exact shape TBD);
- this is the schema-level counterpart of the absent-body-versus-JSON-null rule in main spec sections 26–27.

**Open:** 3.1 JSON Schema keyword coverage matrix (`contains`, `unevaluatedProperties`, `prefixItems`, ...): which keywords affect generated types versus being validation-only metadata. Initial position: type-shaping keywords map to code; pure validation keywords attach as runtime-validation metadata per section 9.

---

## 3. `$ref` resolution

**Decided:**

- Reference targets: any component type the generator consumes (schemas, parameters, request bodies, responses, security schemes).
- JSON Pointer escaping (`~0`, `~1`) handled per RFC 6901.
- Sibling semantics are version- and context-dependent, matching the OAS distinction between Reference Objects and Schema Objects that merely contain a `$ref` keyword:
  - OAS 3.0 **Reference Object**: only `$ref` exists; every sibling key is ignored with a warning (`summary`/`description` were added to Reference Objects in 3.1);
  - OAS 3.1+/3.2 **Reference Object**: `$ref`, `summary`, `description`; these siblings are recognized and carried into the IR as reference metadata;
  - OAS 3.1+/3.2 **Schema Object containing a `$ref` keyword**: this is not a Reference Object; sibling schema keywords are preserved and evaluated together with the referenced schema per JSON Schema 2020-12 conjunction semantics.
- The parser classifies each node by document version and context before normalization so these three cases never collapse into a single rule.
- Cycle policy: the IR is graph-aware. Recursion through properties generates heap-indirected types (`Box<T>`); recursion broken by arrays/maps is direct. A schema requiring unbroken self-containment (a value must contain itself) is a generation error.
- Inline expansion depth cap (configurable) guards pathological nesting; hitting it is an error, not silent truncation.

**Proposed:** External file references via relative paths are supported in v1; remote URL resolution requires an explicit opt-in fetcher so generation stays deterministic.

**Open:** Anchor-style refs (`$id`/`$anchor`, 3.1) support scope.

---

## 4. Schema composition

### 4.1 `allOf`

**Decided:** The IR defines `allOf` as **schema intersection first**; object merging is merely a codegen optimization applied when proven safe.

- Object members merge field-wise into one struct: identical constraints collapse, `required` unions, and conflicting constraints for the same property are generation errors.
- Scalar/array members intersect compatible type and validation constraints into a single type (for example two string schemas with `minLength: 3` and a `pattern` become one validated string carrying both checks). `serde(flatten)` is object/map-oriented and MUST NOT be used as a fallback for non-object members.
- Intersections that cannot be represented losslessly in generated types fall back to a raw/value representation with retained validation metadata, or are generation errors listing schema paths — the same conservative philosophy as unrepresentable `oneOf`/`anyOf`.

### 4.2 `oneOf` / `anyOf`

**Decided:**

- `oneOf` means **exactly-one validation**. Generated decoders attempt every branch and fail when zero or more than one branch validates. A document matching multiple branches is a data-level schema violation (main spec `SchemaViolation` → `422` path), never silently resolved by declaration order.
- Branch sets whose disjointness cannot be proven statically fall back per configuration: either a raw/value representation carrying validation metadata, or a generation error listing the offending schema paths. Silent choose-one enums are forbidden for ambiguous `oneOf`.
- `anyOf` ("at least one") generates a Rust enum ONLY when the generator can prove the branches are mutually exclusive, using the same proof standard as above. Otherwise it falls back to a raw/value representation with retained validation metadata. The generator MUST NOT emit a choose-one enum for `anyOf`.
- The discriminator is a selection hint only (per OAS): it may route decoding but never changes a `oneOf` validation verdict.

**Decided (discriminator architecture):** the default is **inspect-select-validate** — read the discriminator property from the raw document, select the candidate branch schema, then deserialize that candidate. Discriminator selection may optimize **deserialization only**; it never changes the validation verdict. Unless mutual exclusivity of all branches has been proven statically, exactly-one validation must still demonstrate that every other branch fails for the document; when exclusivity IS proven, validating only the selected branch is safe. Collapsing the pipeline into native Serde internally-tagged enums is permitted only when static analysis proves wire-shape equivalence AND exclusivity: every branch carries the constant tag property with its expected value(s), no branch has conflicting tag constraints. Otherwise the two-phase codec is emitted even though it is less idiomatic; correctness outranks ergonomics.

With an explicit `mapping`, mapping entries select their targeted schemas directly during the inspect phase; the validation rule above is unchanged.

### 4.3 Enumerations

**Proposed:** `enum` of constants with `string` values mapped through serde renames; integer enums use numeric discriminants; mixed-type enums generate a fallback raw-value variant rather than failing.

### 4.4 `additionalProperties`

**Decided:**

- `false` → `#[serde(deny_unknown_fields)]`;
- absent/`true` → unknown keys ignored. This is an explicit **lossy model** policy choice, not a claim about OpenAPI semantics: valid extension properties cannot be preserved by the typed model and disappear in round trips. Applications that must preserve them use the schema-valued form or the raw/value fallback;
- schema-valued → `#[serde(flatten)] additional: HashMap<String, T>` alongside named properties.

**Open:** Free-form object (`type: object` with no properties, no constraint) defaults to `serde_json::Value` vs `Map<String, Value>`.

### 4.5 String formats

**Proposed:** Default keeps `String`/primitives to stay dependency-light. Optional typed-format feature maps `date-time`/`date` to a chrono/time type and `uuid` to `uuid::Uuid` when enabled.

---

## 5. Read-only and write-only

**Decided:** Shared models keep every field; directionality is enforced by generated **directional view types** used by operation codecs.

Transport directions are explicit:

- client request encode and server request decode use the **write view** (`readOnly` fields omitted from the wire);
- server response encode and client response decode use the **read view** (`writeOnly` fields treated as absent on the wire).

Requiredness is directional, matching the schema's meaning in each direction:

- required + `readOnly` is required only in the read/response direction;
- required + `writeOnly` is required only in the write/request direction (for example a required write-only `password` exists in `WidgetWrite` but not in `WidgetRead`);

Conversion between shared models and views is intentionally **asymmetric**:

- projecting **shared model → directional view** is always generated where fields are borrowable;
- converting a **decoded directional view → shared model** is generated only when it can be lossless (every missing-in-view field has a default or is `Option`); otherwise it is either a `TryFrom` conversion requiring an application-supplied completion step for the absent fields, or it is not generated at all. A read view of `Widget { password: String }` cannot fabricate the password; the generator never invents values to force a lossless-looking API.

Derived Serde impls on the shared model remain untouched — the view boundary makes directionality a compile-time-visible property of generated codecs instead of a runtime filter or validation afterthought.

---

## 6. Parameters

**Decided:** Full `style` × `explode` matrix implemented per OAS defaults (form+explode for query/cookie, simple for path/header). `deepObject` supported for query. Query `allowReserved` is honored per OAS. Unknown style/location combinations are generation errors. OAS 3.2's `in: querystring` parameter location is **rejected in v1** with a clear generation diagnostic, consistent with the conservative fallback philosophy of section 4.2; support MAY be added later.

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

**Decided:** Server selection follows OAS precedence:

1. an operation-level `servers` array overrides the path-level array;
2. a path-level array overrides the root-level array;
3. an absent or empty root-level array implies `/`, resolved against the document's own location;
4. within the effective array, the first entry is the default base URL and all entries generate constructors.

Relative server URLs resolve deterministically against the URL of the document that declared them. Server variables (`{region}`) become builder parameters with declared defaults and `enum` validation. Path parameters percent-encode using RFC 3986 unreserved set; query parameter serialization order is declaration order for deterministic output.

---

## 9. Runtime validation policy

**Proposed:** Schema constraints (patterns, min/max, format checks) attach to the IR as validation metadata. Defaults: server-side requests validate and reject via the main spec's `SchemaViolation` → `422` path; server responses trust application construction; client decoding is lenient by default with a strict mode behind configuration.

**Decided:** Leniency applies **only to validation metadata** — pattern/min/max/format-style checks. Structural, type-shaping validation is always enforced in every mode and cannot be weakened by any configuration: `oneOf` exactly-one cardinality and exclusivity proofs (section 4.2), discriminator routing verdicts, and the presence/nullability matrix (section 2.1). A "lenient client mode" therefore never degrades composition correctness.

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

All **Open** items that affect the IR or generated public types MUST be resolved by the end of Phase 0 and therefore block Phase 1 start — for example the JSON Schema keyword coverage matrix (section 2), free-form object representation (section 4.4), anchor refs (section 3), and framework-type naming collisions (section 10). Implementation-detail Open items may remain open until the relevant implementation phase.

Open items should be promoted to Decided/Proposed with rationale as implementation feedback arrives, mirroring the milestone structure of main spec section 52.

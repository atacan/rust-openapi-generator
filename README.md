# openapi-to-rust

Deterministic OpenAPI 3.1 → Rust generator: shared schema models, directional
read/write views, a bounded Reqwest client, and an Axum server interface —
byte-for-byte reproducible output (no timestamps, no paths, no environment
sensitivity).

Two CLI modes share one binary (`openapi-to-rust`):

```bash
# Print the deterministic normalized dump to stdout.
openapi-to-rust --dump api.yaml

# Generate Rust source artifacts for the document.
openapi-to-rust <path-to-yaml> [OPTIONS]
```

Run `openapi-to-rust --help` for the full option reference.

## Selective generation

By default the generator writes all four source files of one crate:
`models.rs`, `views.rs`, `client.rs`, and `server.rs`. The extensible
`--generate` namespace selects individual artifacts:

| Selection            | Files written                |
| -------------------- | ---------------------------- |
| `types`              | `models.rs` + `views.rs`     |
| `client`             | `client.rs`                  |
| `server`             | `server.rs`                  |
| `all` (or omitted)   | all four, as historically    |

Repeated flags and comma-separated lists are equivalent
(`--generate types --generate client` ≡ `--generate types,client`); selections
deduplicate deterministically and argument order never affects emitted bytes.

## The split-crate workflow

The motivating layout gives every schema type ONE Rust identity: a workspace
with the shared OpenAPI types in their own crate and each transport in its
own crate. No conversion layer between duplicated structs is needed because
client and server consume the exact same model types.

```text
workspace/
├── api-types/      src/models.rs  src/views.rs   (--generate types)
├── api-client/     src/client.rs                 (--generate client …)
└── api-server/     src/server.rs                 (--generate server …)
```

Generate each surface:

```bash
openapi-to-rust api.yaml \
  --generate types \
  --output-dir crates/api-types/src

openapi-to-rust api.yaml \
  --generate client \
  --types-path api_types \
  --output-dir crates/api-client/src

openapi-to-rust api.yaml \
  --generate server \
  --types-path api_types \
  --output-dir crates/api-server/src
```

`--types-path` names where the shared types live when they are NOT generated
in the same invocation; its value must be a Rust module/crate path (validated
before anything is written). It supports external crate paths (`api_types`,
`::company_api::v2`), nested in-crate modules (`crate::generated::types`),
relative qualifiers (`crate::types`, `self::types`, `super::shared`,
`super::super::shared`, `self::super::shared`), and raw identifiers
(`r#type`). Passing it together with a `types` selection in one invocation is
rejected as ambiguous.

### Wiring the crates by hand

Generation is SOURCE-ONLY: normal generation intentionally does not edit,
create, or merge any `Cargo.toml`. You own your manifests. The types crate
exposes the generated modules with ordinary wiring:

```rust
// crates/api-types/src/lib.rs
pub mod models;
pub mod views;
```

Each transport crate depends on the local types package and only the
dependencies its generated code genuinely uses:

```toml
# crates/api-server/Cargo.toml (api-client mirrors it with reqwest et al.)
[dependencies]
api-types = { path = "../api-types" }
openapi-support = { version = "0.1", features = ["server"] }
axum = "0.8"
async-trait = "0.1"
bytes = "1"
http = "1"
mime = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### Cargo package name vs Rust crate path

The `--types-path` value is a RUST PATH, not a Cargo package name:

```text
Cargo package: api-types
Rust crate path: api_types
```

That is why the commands above pass `--types-path api_types` while the
manifest dependency reads `api-types = { path = "../api-types" }`.

### Enable only the transport you need

Enable ONLY the transport-side dependencies/features each crate needs so the
client does not compile the server stack and vice versa: give the support
dependency `features = ["client"]` in the client crate and
`features = ["server"]` in the server crate, never both. The repository's own
integration suite compiles exactly this three-crate layout and asserts the
isolation (`reqwest` absent from the server graph, `axum` absent from the
client graph, and matching single-feature support builds), so the workflow
stays honest as the generator evolves.

## Output stability

All output is deterministic: repeated generation of the same document yields
byte-identical files, argument order cannot affect bytes, and the default
all-in-one mode remains byte-compatible with earlier releases.

# Rust OpenAPI Code Generation Specification
## Direct Axum Server + Reqwest Client with Streaming-First HTTP Bodies

**Status:** Draft design specification  
**Target ecosystem:** Tokio, Axum, Reqwest, `http`, `bytes`, Serde  
**OpenAPI target:** OpenAPI 3.1+ semantics, with 3.0-compatible behavior where practical  
**Primary design goal:** Generate idiomatic framework-native Rust while preserving streaming and bounded-memory behavior for HTTP request and response bodies.

---

## 1. Goals

This specification defines how an OpenAPI operation is translated into two generated Rust surfaces:

1. a **Reqwest client**, and
2. an **Axum server interface/router**.

The generator intentionally does **not** introduce a transport-neutral HTTP runtime abstraction. Generated client code uses Reqwest types directly; generated server code uses Axum types directly. Shared generated schema types use ordinary Rust/Serde types.

The generator MUST make the memory behavior of every body representation predictable:

- structured finite bodies such as ordinary JSON are decoded/encoded with explicit configurable limits;
- binary, unknown, file-like, event-stream, and other potentially unbounded bodies remain streaming;
- no generated large-body path may silently call an unbounded whole-body collector;
- request and response variants are represented as Rust enums wherever OpenAPI describes multiple alternatives, forcing developers to handle documented alternatives explicitly.

This document focuses on request/response body and response-status generation, together with the contract-boundary semantics that keep those invariants airtight: bounded serialization (section 34), pre-handler protocol rejections (section 39), post-commit stream failures (section 40), transport policies (section 30), and the body-presence state machine (section 28). Parameter serialization, authentication, and schema-generation details are specified separately in the companion document [`openapi-semantics-spec.md`](./openapi-semantics-spec.md) and only referenced here where needed by examples. Callbacks, links, and webhooks remain outside the scope of both documents.

---

## 2. Core principles

### 2.1 Framework-native generated code

The generator targets a fixed stack:

```text
Client:  Tokio -> Reqwest -> Hyper/http-body internally
Server:  Tokio -> Hyper -> Tower -> Axum
Shared:  http + bytes + serde
```

There is no generated conversion through a custom `RuntimeBody`, `ClientTransport`, or `ServerTransport` abstraction.

Typical native body types are:

```rust
// Client request streaming body
reqwest::Body

// Client response streaming body
reqwest::Response

// Server request streaming body
axum::body::Body

// Server response streaming body
axum::body::Body
```

### 2.2 Streaming is the default for potentially unbounded payloads

A media type that represents arbitrary bytes, files, continuous events, or an unknown/custom representation MUST NOT be converted into `Vec<u8>`, `String`, or `bytes::Bytes` for the complete body.

Examples:

```text
application/octet-stream     -> stream
image/png                    -> stream
video/mp4                    -> stream
application/pdf              -> stream
application/zip              -> stream
text/event-stream            -> typed stream
application/x-ndjson         -> typed item stream
application/json-seq         -> typed item stream
unknown vendor media type    -> stream by default
```

### 2.3 Finite structured bodies are bounded

Ordinary document-oriented structured media types are allowed to be buffered, but only through an explicit generated limit.

Examples:

```text
application/json
application/problem+json
application/vnd.example+json
application/x-www-form-urlencoded
small text/plain values represented as String
```

Generated code MUST use a configurable per-operation or client/server default limit, for example:

```rust
const DEFAULT_STRUCTURED_BODY_LIMIT: usize = 8 * 1024 * 1024;
```

The exact default is a generator configuration decision. The important invariant is that generated code never performs an unbounded `response.bytes().await?`, `response.text().await?`, or Axum whole-body aggregation for a structured body.

### 2.4 Status codes are exhaustive enums

If an operation documents multiple response statuses, the generated operation result is an enum.

OpenAPI:

```yaml
responses:
  '200': ...
  '201': ...
  '400': ...
  '404': ...
```

Generated shape:

```rust
pub enum CreateWidgetResponse {
    Ok200(...),
    Created201(...),
    BadRequest400(...),
    NotFound404(...),
}
```

Consumers therefore use exhaustive Rust matching:

```rust
match client.create_widget(input).await? {
    CreateWidgetResponse::Ok200(value) => { /* ... */ }
    CreateWidgetResponse::Created201(value) => { /* ... */ }
    CreateWidgetResponse::BadRequest400(problem) => { /* ... */ }
    CreateWidgetResponse::NotFound404(problem) => { /* ... */ }
}
```

### 2.5 Media-type alternatives are nested enums

If one request body or one response status allows several media types, the generated operation contains a second enum for that content choice.

OpenAPI:

```yaml
'200':
  content:
    application/json: ...
    application/octet-stream: ...
```

Generated conceptual shape:

```rust
pub enum GetArtifact200Content {
    Json(Artifact),
    OctetStream(reqwest::Response),
}

pub enum GetArtifactResponse {
    Ok200(GetArtifact200Content),
    NotFound404(ProblemDetails),
}
```

The Axum server gets the analogous server-native enum, using `axum::body::Body` for streaming content.

### 2.6 Schema types are shared; operation transport types are framework-specific

Schema-derived models should be reusable by client and server:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: String,
    pub name: String,
}
```

Operation request/response wrappers are generated separately for client and server because they may contain framework-native body types:

```text
client::UploadBody::OctetStream(reqwest::Body)
server::UploadBody::OctetStream(axum::body::Body)
```

This avoids a transport-neutral runtime while still sharing all ordinary data models.

---

## 3. Dependencies and generated module layout

A generated crate may use the following shape:

```text
src/
  lib.rs
  models.rs
  support.rs
  client/
    mod.rs
    widgets.rs
  server/
    mod.rs
    widgets.rs
```

Suggested Cargo features:

```toml
[features]
default = ["client", "server"]
client = ["dep:reqwest", "dep:tokio-util", "dep:futures-util"]
server = ["dep:axum", "dep:tower", "dep:futures-util"]
```

Representative dependencies:

```toml
bytes = "1"
http = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
mime = "0.3"

reqwest = { version = "...", optional = true, features = ["json", "multipart", "stream"] }
axum = { version = "...", optional = true, features = ["json", "form", "multipart", "tokio"] }
tokio = { version = "1", features = ["fs", "io-util"] }
tokio-util = { version = "...", optional = true, features = ["io", "codec"] }
futures-util = { version = "...", optional = true }
```

The generated `support` module SHOULD be small and contain only reusable helpers such as bounded collection (`collect_limited`), bounded serialization (`serialize_json_limited`, `serialize_form_limited`, section 34), content-type matching, percent encoding, structured decode errors, protocol rejection types (section 39), stream-failure/encode-overflow hooks, and event-stream codecs. It MUST NOT define a parallel HTTP transport abstraction.

### 3.1 Toolchain and dependency version policy

Each released generator version embeds its supported framework tuple and toolchain floor as versioned metadata — for example: *generator 0.x targets Axum `0.8.x`, Reqwest `0.12.x`, `http` `1.x`, `bytes` `1.x`, MSRV 1.85*. Supported versions are a property of the generator release, never computed from ecosystem state at generation time; this keeps output deterministic across machines and points in time.

Generated crates declare:

- caret version requirements matching the generator release's embedded tuple, overridable through generator configuration;
- the pinned MSRV in `rust-version`;
- no floating pre-release or path dependencies.

Generating against an unsupported framework combination is an error, not best-effort output, so emitted code can rely on concrete framework APIs.

---

## 4. Naming rules

Assume `operationId: getArtifact`.

| OpenAPI concept | Generated Rust name |
|---|---|
| Operation client method | `get_artifact` |
| Operation server trait method | `get_artifact` |
| Client response enum | `GetArtifactResponse` |
| Server response enum | `GetArtifactResponse` within server module |
| Request content enum | `GetArtifactRequestBody` |
| Status-specific content enum | `GetArtifact200Content` |
| Status-specific headers | `GetArtifact200Headers` |
| Multipart typed part enum | `UploadDocumentPart` or generated field wrapper |

Status variants SHOULD contain both semantic and numeric information in the identifier where practical:

```rust
Ok200
Created201
NoContent204
BadRequest400
Unauthorized401
Forbidden403
NotFound404
Conflict409
UnprocessableEntity422
TooManyRequests429
InternalServerError500
```

For nonstandard codes:

```rust
Status299
Status599
```

The numeric suffix prevents collisions and makes the OpenAPI mapping obvious.

---

## 5. Media-type classification

There are infinitely many valid registered and vendor media types, so the generator cannot maintain an exhaustive string list. Instead it MUST classify every media type into a semantic category.

### 5.1 JSON family

Matches:

```text
application/json
application/problem+json
application/*+json
vendor media types ending in +json
```

Default representation:

```text
OpenAPI schema -> generated Serde model
```

Memory behavior: **bounded full-document buffering**.

### 5.2 Plain textual family

Matches media types whose schema semantically maps to a string, including commonly:

```text
text/plain
text/html
text/csv
text/markdown
application/sql
```

Two modes are possible:

- **typed finite string** when the schema explicitly describes a finite string and the generator is configured to materialize text;
- **streaming bytes** for large/unknown text, wildcard `text/*`, or explicitly streaming operations.

Default for `text/plain` + `schema: { type: string }`: bounded `String`.

### 5.3 Binary/raw family

Includes:

```text
application/octet-stream
image/*
audio/*
video/*
application/pdf
application/zip
application/gzip
font/*
and custom media types with no selected structured codec
```

Representation: framework-native streaming body.

### 5.4 URL encoded forms

```text
application/x-www-form-urlencoded
```

Representation: generated Serde struct.

Memory behavior: bounded because form decoding requires the finite form representation.

### 5.5 Multipart

Includes:

```text
multipart/form-data
multipart/mixed
multipart/related
other multipart/*
```

Multipart MUST be parsed incrementally. Individual binary/file parts MUST remain streams.

Small scalar or JSON parts MAY be bounded and decoded into generated values.

### 5.6 Server-Sent Events

```text
text/event-stream
```

Representation: typed asynchronous event stream.

The operation's schema describes the **type of each streamed item** (section 18.1); `x-rust-stream-item` overrides when the schema instead describes an envelope. SSE framing semantics — JSON-only `data:` fields, `id`/`event`/`retry`, comments, multi-line data, malformed events — are fixed in section 18.2.

### 5.7 Newline-delimited JSON

Recognized aliases MAY include:

```text
application/x-ndjson
application/ndjson
application/jsonl
```

Representation: asynchronous stream of decoded items.

Each logical JSON record is bounded independently by `max_stream_record_bytes`; the entire body is not. The schema describes the type of each streamed item (section 18.1).

### 5.8 JSON Text Sequences

```text
application/json-seq
```

Representation: asynchronous stream of decoded items framed according to JSON Text Sequences.

The schema describes the type of each streamed item (section 18.1).

### 5.9 Optional codec families

Formats such as XML, CBOR, MessagePack, Protobuf, Avro, or custom vendor formats require an explicit codec policy/plugin.

Without a configured codec they MUST fall back to raw streaming bodies rather than being guessed into an eager representation.

Examples:

```text
application/xml               -> raw stream by default, typed if XML codec enabled
application/cbor              -> raw stream by default, typed if CBOR codec enabled
application/protobuf          -> raw stream by default, typed if Protobuf mapping enabled
application/vnd.foo+protobuf  -> same policy
```

### 5.10 Media type ranges and wildcards

OpenAPI permits media-type ranges such as:

```text
text/*
application/*
*/*
```

Wildcard representations MUST be raw streaming bodies unless a more specific content entry matched first.

The runtime parser follows HTTP specificity:

```text
application/problem+json > application/* > */*
```

---

## 6. Summary mapping: request bodies

| OpenAPI request content | Reqwest generated parameter | Axum generated handler input | Memory model |
|---|---|---|---|
| `application/json`, `*+json` | generated `T` or `&T` | generated `T` | bounded full document |
| `text/plain` string | `String` / `&str` | `String` | bounded full document |
| `application/x-www-form-urlencoded` | generated form struct | generated form struct | bounded full document |
| `application/octet-stream` | `reqwest::Body` | `axum::body::Body` | streaming |
| `image/*`, `audio/*`, `video/*`, PDF, ZIP, etc. | `reqwest::Body` | `axum::body::Body` | streaming |
| `multipart/form-data` | generated multipart input builder/value | generated typed streaming multipart input | streaming per part |
| `multipart/*` generic | generated/raw multipart stream | generated/raw multipart stream | streaming |
| `text/event-stream` request | stream/body, when supported by operation | typed event stream/body | streaming |
| NDJSON | `Stream<Item = Result<T, E>>` or body wrapper | `Stream<Item = Result<T, DecodeError>>` | streaming by record |
| JSON Sequence | item stream | item stream | streaming by record |
| unknown/custom | `reqwest::Body` | `axum::body::Body` | streaming |
| multiple media types | client content enum | server content enum | variant-dependent |

---

## 7. Summary mapping: response bodies

| OpenAPI response content | Reqwest generated response variant | Axum generated response field | Memory model |
|---|---|---|---|
| `application/json`, `*+json` | generated `T` | generated `T` encoded as JSON | bounded full document |
| `text/plain` string | `String` | `String` | bounded full document |
| no body | `()` / unit variant | `()` | none |
| `application/octet-stream` | owns `reqwest::Response` | `axum::body::Body` | streaming |
| image/audio/video/PDF/ZIP/etc. | owns `reqwest::Response` | `axum::body::Body` | streaming |
| multipart response | streaming multipart response wrapper/body | generated multipart streaming encoder/body | streaming |
| `text/event-stream` | typed event stream backed by Reqwest response | Axum `Sse`-compatible stream | streaming |
| NDJSON | typed item stream | typed item stream encoded incrementally | streaming by record |
| JSON Sequence | typed item stream | typed item stream encoded incrementally | streaming by record |
| unknown/custom | owns `reqwest::Response` | `axum::body::Body` | streaming |
| multiple media types | nested status-content enum | nested status-content enum | variant-dependent |

---

## 8. Example 1 — JSON request and JSON response

### OpenAPI input

```yaml
paths:
  /widgets:
    post:
      operationId: createWidget
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/CreateWidget'
      responses:
        '201':
          description: Created
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Widget'
        '400':
          description: Invalid request
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Generated shared schema types

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateWidget {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Widget {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProblemDetails {
    pub title: String,
    pub detail: Option<String>,
}
```

### Output A — Reqwest client

```rust
pub enum CreateWidgetResponse {
    Created201(Widget),
    BadRequest400(ProblemDetails),
}

impl Client {
    pub async fn create_widget(
        &self,
        body: &CreateWidget,
    ) -> Result<CreateWidgetResponse, ClientError> {
        // Bounded request serialization (section 34). Reqwest's `.json(body)`
        // convenience is deliberately not used because it buffers without a limit.
        let payload = serialize_json_limited(
            body,
            self.limits.structured_encode_bytes,
        )?;

        let response = self.http
            .post(self.url("/widgets")?)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await?;

        match response.status() {
            http::StatusCode::CREATED => {
                let bytes = collect_reqwest_limited(
                    response,
                    self.limits.structured_response_bytes,
                ).await?;
                Ok(CreateWidgetResponse::Created201(
                    serde_json::from_slice(&bytes)?,
                ))
            }
            http::StatusCode::BAD_REQUEST => {
                let bytes = collect_reqwest_limited(
                    response,
                    self.limits.error_response_bytes,
                ).await?;
                Ok(CreateWidgetResponse::BadRequest400(
                    serde_json::from_slice(&bytes)?,
                ))
            }
            status => Err(ClientError::UndocumentedStatus { status }),
        }
    }
}
```

### Output B — Axum server

```rust
pub enum CreateWidgetResponse {
    Created201(Widget),
    BadRequest400(ProblemDetails),
}

#[async_trait::async_trait]
pub trait WidgetsApi: Send + Sync + 'static {
    async fn create_widget(
        &self,
        body: CreateWidget,
    ) -> CreateWidgetResponse;
}
```

Generated router logic performs bounded JSON extraction (with the route's `DefaultBodyLimit` wired to `structured_request_bytes`) and then dispatches to the trait. Response encoding uses the generated limited serializers, never `axum::Json` directly:

```rust
impl CreateWidgetResponse {
    /// Infallible path used by the generated router; applies the
    /// section 34 fallback on encode overflow.
    pub fn into_response_with_limits(
        self,
        limits: &BodyLimits,
    ) -> axum::response::Response {
        match self {
            Self::Created201(body) => {
                match serialize_json_limited(&body, limits.structured_encode_bytes) {
                    Ok(bytes) => (
                        http::StatusCode::CREATED,
                        [(http::header::CONTENT_TYPE, "application/json")],
                        bytes,
                    ).into_response(),
                    Err(_) => fallback_internal_error(), // 500, empty body, hook fires
                }
            }

            Self::BadRequest400(body) => {
                // Content-Type is application/problem+json, not generic application/json.
                encode_problem_json_limited(http::StatusCode::BAD_REQUEST, &body, limits)
            }
        }
    }
}

impl axum::response::IntoResponse for CreateWidgetResponse {
    fn into_response(self) -> axum::response::Response {
        self.into_response_with_limits(&BodyLimits::process_default())
    }
}
```

The router invokes `into_response_with_limits` with its configured limits so that application-level composition through plain `IntoResponse` remains possible without losing the bound entirely.

**Memory behavior:** bounded complete JSON document on both request and response paths.

---

## 9. Example 2 — streaming binary upload and response status enum

### OpenAPI input

```yaml
paths:
  /objects/{id}:
    put:
      operationId: putObject
      parameters:
        - in: path
          name: id
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/octet-stream:
            schema:
              type: string
              format: binary
      responses:
        '201':
          description: Stored
        '400':
          description: Invalid object
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
        '409':
          description: Version conflict
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum PutObjectResponse {
    Created201,
    BadRequest400(ProblemDetails),
    Conflict409(ProblemDetails),
}

impl Client {
    pub async fn put_object(
        &self,
        id: &str,
        body: reqwest::Body,
    ) -> Result<PutObjectResponse, ClientError> {
        let response = self.http
            .put(self.object_url(id)?)
            .header(http::header::CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .send()
            .await?;

        // status decoding omitted
        todo!()
    }
}
```

Caller streams a file without loading it into memory:

```rust
let file = tokio::fs::File::open(path).await?;
let chunks = tokio_util::io::ReaderStream::new(file);
let body = reqwest::Body::wrap_stream(chunks);

match client.put_object("abc", body).await? {
    PutObjectResponse::Created201 => {}
    PutObjectResponse::BadRequest400(problem) => {}
    PutObjectResponse::Conflict409(problem) => {}
}
```

### Output B — Axum server

```rust
pub enum PutObjectResponse {
    Created201,
    BadRequest400(ProblemDetails),
    Conflict409(ProblemDetails),
}

#[async_trait::async_trait]
pub trait ObjectsApi: Send + Sync + 'static {
    async fn put_object(
        &self,
        id: String,
        body: axum::body::Body,
    ) -> PutObjectResponse;
}
```

Application implementation can process chunks incrementally:

```rust
async fn put_object(
    &self,
    id: String,
    body: axum::body::Body,
) -> PutObjectResponse {
    use futures_util::StreamExt;

    let mut chunks = body.into_data_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk: bytes::Bytes = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return PutObjectResponse::BadRequest400(/* ... */),
        };

        // Write/upload/process `chunk` and release it.
    }

    PutObjectResponse::Created201
}
```

**Memory behavior:** request body is streaming and total payload size is not tied to process memory.

---

## 10. Example 3 — streaming binary download

### OpenAPI input

```yaml
paths:
  /objects/{id}:
    get:
      operationId: getObject
      responses:
        '200':
          description: Object bytes
          headers:
            ETag:
              schema:
                type: string
            Content-Length:
              schema:
                type: integer
                format: int64
          content:
            application/octet-stream:
              schema:
                type: string
                format: binary
        '404':
          description: Missing
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub struct GetObject200 {
    // Parsed documented headers may be copied out before `response` is moved.
    pub etag: Option<String>,
    pub content_length: Option<u64>,
    pub response: reqwest::Response,
}

impl GetObject200 {
    pub fn into_bytes_stream(
        self,
    ) -> impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> {
        self.response.bytes_stream()
    }
}

pub enum GetObjectResponse {
    Ok200(GetObject200),
    NotFound404(ProblemDetails),
}
```

Usage:

```rust
match client.get_object("abc").await? {
    GetObjectResponse::Ok200(download) => {
        let mut chunks = download.into_bytes_stream();
        while let Some(chunk) = chunks.next().await {
            file.write_all(&chunk?).await?;
        }
    }
    GetObjectResponse::NotFound404(problem) => {
        // Explicitly handled.
    }
}
```

### Output B — Axum server

```rust
pub struct GetObject200 {
    pub etag: Option<String>,
    pub content_length: Option<u64>,
    pub body: axum::body::Body,
}

pub enum GetObjectResponse {
    Ok200(GetObject200),
    NotFound404(ProblemDetails),
}
```

An implementation can return a file stream:

```rust
let file = tokio::fs::File::open(path).await?;
let stream = tokio_util::io::ReaderStream::new(file);
let body = axum::body::Body::from_stream(stream);

GetObjectResponse::Ok200(GetObject200 {
    etag: Some(etag),
    content_length: Some(size),
    body,
})
```

**Memory behavior:** successful object response streams end-to-end. The 404 JSON body is bounded and decoded.

---

## 11. Example 4 — one status with JSON OR binary body

This is the important nested-enum case.

### OpenAPI input

```yaml
paths:
  /artifacts/{id}:
    get:
      operationId: getArtifact
      responses:
        '200':
          description: Artifact representation
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/ArtifactMetadata'
            application/octet-stream:
              schema:
                type: string
                format: binary
        '404':
          description: Missing artifact
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum GetArtifact200Content {
    Json(ArtifactMetadata),
    OctetStream(reqwest::Response),
}

pub enum GetArtifactResponse {
    Ok200(GetArtifact200Content),
    NotFound404(ProblemDetails),
}
```

Generated decoding logic:

```rust
match response.status() {
    http::StatusCode::OK => {
        match classify_content_type(response.headers())? {
            MediaType::Json => {
                let bytes = collect_reqwest_limited(
                    response,
                    limits.structured_response_bytes,
                ).await?;

                Ok(GetArtifactResponse::Ok200(
                    GetArtifact200Content::Json(
                        serde_json::from_slice(&bytes)?,
                    ),
                ))
            }
            MediaType::Exact("application/octet-stream") => {
                Ok(GetArtifactResponse::Ok200(
                    GetArtifact200Content::OctetStream(response),
                ))
            }
            other => Err(ClientError::UnexpectedContentType { other }),
        }
    }
    // ...
}
```

Caller MUST handle both representations:

```rust
match client.get_artifact("abc").await? {
    GetArtifactResponse::Ok200(content) => match content {
        GetArtifact200Content::Json(metadata) => { /* ... */ }
        GetArtifact200Content::OctetStream(response) => {
            let mut stream = response.bytes_stream();
            // ...
        }
    },
    GetArtifactResponse::NotFound404(problem) => { /* ... */ }
}
```

### Output B — Axum server

```rust
pub enum GetArtifact200Content {
    Json(ArtifactMetadata),
    OctetStream(axum::body::Body),
}

pub enum GetArtifactResponse {
    Ok200(GetArtifact200Content),
    NotFound404(ProblemDetails),
}
```

Generated `IntoResponse` sets the correct content type based on the nested variant.

```rust
match response {
    GetArtifactResponse::Ok200(GetArtifact200Content::Json(value)) => {
        // status 200, Content-Type application/json
    }
    GetArtifactResponse::Ok200(GetArtifact200Content::OctetStream(body)) => {
        // status 200, Content-Type application/octet-stream
    }
    GetArtifactResponse::NotFound404(problem) => {
        // status 404, Content-Type application/problem+json
    }
}
```

---

## 12. Example 5 — request body with JSON OR binary

### OpenAPI input

```yaml
paths:
  /imports:
    post:
      operationId: createImport
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/ImportDescriptor'
          application/octet-stream:
            schema:
              type: string
              format: binary
      responses:
        '202':
          description: Accepted
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/ImportJob'
```

### Output A — Reqwest client

```rust
pub enum CreateImportRequestBody {
    Json(ImportDescriptor),
    OctetStream(reqwest::Body),
}

pub enum CreateImportResponse {
    Accepted202(ImportJob),
}

impl Client {
    pub async fn create_import(
        &self,
        body: CreateImportRequestBody,
    ) -> Result<CreateImportResponse, ClientError> {
        let request = self.http.post(self.url("/imports")?);

        let request = match body {
            CreateImportRequestBody::Json(value) => {
                // Bounded request serialization (section 34); Reqwest's `.json()`
                // convenience is deliberately not used.
                let payload =
                    serialize_json_limited(&value, self.limits.structured_encode_bytes)?;
                request
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(payload)
            }
            CreateImportRequestBody::OctetStream(body) => request
                .header(http::header::CONTENT_TYPE, "application/octet-stream")
                .body(body),
        };

        // send and decode...
        todo!()
    }
}
```

### Output B — Axum server

```rust
pub enum CreateImportRequestBody {
    Json(ImportDescriptor),
    OctetStream(axum::body::Body),
}

#[async_trait::async_trait]
pub trait ImportsApi: Send + Sync + 'static {
    async fn create_import(
        &self,
        body: CreateImportRequestBody,
    ) -> CreateImportResponse;
}
```

The generated Axum route cannot use `Json<T>` directly because the same operation admits multiple media types. Instead it receives the raw request body, matches the `Content-Type`, and then either:

- performs bounded JSON decoding, or
- passes the Axum body through unchanged.

---

## 13. Example 6 — many status codes with distinct bodies

### OpenAPI input

```yaml
paths:
  /widgets/{id}:
    put:
      operationId: updateWidget
      responses:
        '200':
          description: Updated
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Widget'
        '201':
          description: Created because it did not exist
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Widget'
        '400':
          description: Invalid request
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
        '404':
          description: Related resource not found
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
        '409':
          description: Conflict
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum UpdateWidgetResponse {
    Ok200(Widget),
    Created201(Widget),
    BadRequest400(ProblemDetails),
    NotFound404(ProblemDetails),
    Conflict409(ProblemDetails),
}
```

The return type is NOT `Result<Widget, ApiError>` because that collapses documented protocol outcomes into an application-level success/error convention that OpenAPI did not specify.

`Result` remains reserved for transport/protocol/decoding failures:

```rust
pub async fn update_widget(...)
    -> Result<UpdateWidgetResponse, ClientError>;
```

Therefore:

```text
Rust Result error = request could not be represented/executed/decoded as documented
Response enum     = HTTP outcome documented by the API
```

### Output B — Axum server

```rust
pub enum UpdateWidgetResponse {
    Ok200(Widget),
    Created201(Widget),
    BadRequest400(ProblemDetails),
    NotFound404(ProblemDetails),
    Conflict409(ProblemDetails),
}

#[async_trait::async_trait]
pub trait WidgetsApi {
    async fn update_widget(...) -> UpdateWidgetResponse;
}
```

The server implementation is forced by Rust's enum construction to choose one documented response shape.

---

## 14. Example 7 — 204 No Content

### OpenAPI input

```yaml
paths:
  /widgets/{id}:
    delete:
      operationId: deleteWidget
      responses:
        '204':
          description: Deleted
        '404':
          description: Missing
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum DeleteWidgetResponse {
    NoContent204,
    NotFound404(ProblemDetails),
}
```

### Output B — Axum server

```rust
pub enum DeleteWidgetResponse {
    NoContent204,
    NotFound404(ProblemDetails),
}
```

The generated response serializer MUST ensure the 204 variant does not emit a body. This is a special case of the general no-body rules in section 35, which also cover `205`, `304`, `HEAD`, and informational statuses.

---

## 15. Example 8 — response headers become typed fields

### OpenAPI input

```yaml
responses:
  '201':
    description: Created
    headers:
      Location:
        required: true
        schema:
          type: string
          format: uri
      ETag:
        schema:
          type: string
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Widget'
```

### Output A — Reqwest client

```rust
pub struct CreateWidget201 {
    pub location: String,
    pub etag: Option<String>,
    pub body: Widget,
}

pub enum CreateWidgetResponse {
    Created201(CreateWidget201),
}
```

### Output B — Axum server

```rust
pub struct CreateWidget201 {
    pub location: String,
    pub etag: Option<String>,
    pub body: Widget,
}

pub enum CreateWidgetResponse {
    Created201(CreateWidget201),
}
```

The generated server `IntoResponse` writes the typed fields to headers. The client parses required response headers and treats malformed/missing required headers as protocol decoding errors.

---

## 16. Example 9 — URL encoded form

### OpenAPI input

```yaml
paths:
  /sessions:
    post:
      operationId: createSession
      requestBody:
        required: true
        content:
          application/x-www-form-urlencoded:
            schema:
              type: object
              required: [username, password]
              properties:
                username:
                  type: string
                password:
                  type: string
                remember_me:
                  type: boolean
      responses:
        '204':
          description: Session created
        '401':
          description: Invalid credentials
```

### Generated schema type

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CreateSessionForm {
    pub username: String,
    pub password: String,
    pub remember_me: Option<bool>,
}
```

### Output A — Reqwest client

```rust
pub async fn create_session(
    &self,
    form: &CreateSessionForm,
) -> Result<CreateSessionResponse, ClientError> {
    // Bounded form serialization (section 34); Reqwest's `.form(form)` is not used.
    let payload = serialize_form_limited(form, self.limits.structured_encode_bytes)?;

    let response = self.http
        .post(self.url("/sessions")?)
        .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(payload)
        .send()
        .await?;
    // ...
}
```

### Output B — Axum server

```rust
#[async_trait::async_trait]
pub trait SessionsApi {
    async fn create_session(
        &self,
        form: CreateSessionForm,
    ) -> CreateSessionResponse;
}
```

The generated route MAY use Axum `Form<T>` only when the operation accepts just this one request media type, the route's `DefaultBodyLimit` has been wired to `structured_request_bytes` (section 38), and the extractor's rejection is translated into the section 39 rejection mapping. If content negotiation is required, the route decodes from the raw body after matching `Content-Type`, using the same bounded collection path.

---

## 17. Example 10 — multipart with scalar metadata and streaming file

### OpenAPI input

```yaml
paths:
  /documents:
    post:
      operationId: uploadDocument
      requestBody:
        required: true
        content:
          multipart/form-data:
            schema:
              type: object
              required: [metadata, file]
              properties:
                metadata:
                  $ref: '#/components/schemas/DocumentMetadata'
                file:
                  type: string
                  format: binary
            encoding:
              metadata:
                contentType: application/json
              file:
                contentType: application/octet-stream
      responses:
        '201':
          description: Uploaded
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Document'
```

### Output A — Reqwest client

A generated client-side multipart input SHOULD allow a file part to remain streaming:

```rust
pub struct UploadDocumentRequest {
    pub metadata: DocumentMetadata,
    pub file: reqwest::Body,
    pub file_name: Option<String>,
    pub file_content_type: Option<mime::Mime>,
}
```

The generated request builder constructs a Reqwest multipart form without collecting the file.

An ergonomic file-path constructor may also be generated:

```rust
impl UploadDocumentRequest {
    pub async fn from_file(
        metadata: DocumentMetadata,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, std::io::Error> {
        let file = tokio::fs::File::open(path.as_ref()).await?;
        let stream = tokio_util::io::ReaderStream::new(file);
        Ok(Self {
            metadata,
            file: reqwest::Body::wrap_stream(stream),
            file_name: path.as_ref().file_name()
                .map(|v| v.to_string_lossy().into_owned()),
            file_content_type: None,
        })
    }
}
```

### Output B — Axum server

The generated server-side multipart abstraction MUST preserve sequential streaming semantics.

A preferred API is an operation-specific streaming multipart wrapper rather than a struct containing `Bytes` for each field:

```rust
pub struct UploadDocumentMultipart {
    inner: axum::extract::Multipart,
}

pub enum UploadDocumentPart {
    Metadata(DocumentMetadata),
    File(UploadDocumentFilePart),
}

pub struct UploadDocumentFilePart {
    pub file_name: Option<String>,
    pub content_type: Option<mime::Mime>,
    // wrapper around the current field's chunk stream
    pub field: StreamingMultipartField,
}
```

The exact wrapper API requires care because multipart parsers often borrow the parent parser while a field is active. The generator MUST NOT produce:

```rust
pub struct UploadDocumentRequest {
    pub file: Vec<u8>, // forbidden for a potentially unbounded file
}
```

**Memory behavior:** bounded metadata field, streaming file field.

### 17.1 Multipart resource and cardinality limits

`multipart_scalar_part_bytes` bounds individual parts but not the number of parts. An attacker must not be able to send millions of tiny parts without violating any configured limit, so the generated parser enforces the full cardinality set from section 33:

- `max_multipart_parts`: total fields per request; exceeding it is a protocol rejection;
- `max_part_header_bytes`: per-part header block (including `Content-Disposition`, `Content-Type`, and extension headers); oversized part headers are a rejection before any part payload is read;
- `max_field_name_bytes` / `max_file_name_bytes`: `name=` and `filename=` lengths are checked during header parsing.

Field-matching semantics:

- **Missing required fields** (per the OpenAPI schema) are detected when the part stream ends without them and produce a `422` protocol rejection; the application handler is not invoked.
- **Duplicate scalar/JSON-typed fields**: a repeated `name` that maps to a single-valued schema property is a `422` protocol rejection. Repeated fields whose schema is an array collect in wire order.
- **Unknown fields**: by default ignored for forward compatibility, matching Serde's default; generator configuration MAY switch to strict rejection.
- **Nesting depth**: nested multipart bodies beyond depth 1 are treated as opaque streaming parts unless a codec plugin handles them; a configurable `max_multipart_depth` guards pathological recursion in framing parsers.

All cardinality checks run incrementally while streaming; none requires buffering part payloads.

---

## 18. Example 11 — Server-Sent Events

### OpenAPI input

```yaml
paths:
  /events:
    get:
      operationId: streamEvents
      responses:
        '200':
          description: Event stream
          content:
            text/event-stream:
              schema:
                $ref: '#/components/schemas/Event'
        '401':
          description: Unauthorized
          content:
            application/problem+json:
              schema:
                $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub struct StreamEvents200 {
    // Owns the response-backed SSE decoder.
    inner: SseJsonStream<Event>,
}

impl futures_core::Stream for StreamEvents200 {
    type Item = Result<Event, SseDecodeError>;
    // poll_next delegated to incremental decoder
}

pub enum StreamEventsResponse {
    Ok200(StreamEvents200),
    Unauthorized401(ProblemDetails),
}
```

The client MUST NOT collect the successful `text/event-stream` response. Error JSON remains bounded.

### Output B — Axum server

```rust
pub enum StreamEventsResponse<S> {
    Ok200(S),
    Unauthorized401(ProblemDetails),
}
```

A concrete generated type may erase the stream to reduce generic spread:

```rust
pub type EventStream = std::pin::Pin<
    Box<dyn futures_core::Stream<Item = Result<Event, ServerStreamError>> + Send>
>;

pub enum StreamEventsResponse {
    Ok200(EventStream),
    Unauthorized401(ProblemDetails),
}
```

Generated `IntoResponse` maps `Ok200` to Axum SSE events and `Content-Type: text/event-stream`.

### 18.1 Stream item typing convention

The schema under a stream-typed media type (`text/event-stream`, NDJSON aliases, `application/json-seq`) is interpreted as **the type of each streamed item**, which is what the `$ref: Event` in this example means. Documents that instead describe an envelope may override the interpretation with:

```yaml
x-rust-stream-item:
  $ref: '#/components/schemas/Event'
```

The extension wins when present; otherwise the item-schema convention applies. This convention is normative for the generator and documented in section 5.6.

### 18.2 SSE framing semantics

For SSE, the generated decoder defines precisely how the wire format maps to items:

- `data:` payloads MUST be JSON (per RFC/WHATWG plus this specification's item-schema convention); each event's data is parsed as one item of type `Event`;
- multi-line `data:` fields are joined with `\n` before parsing;
- `id:` and `event:` are ignored by default because the generated item is the bare `Event` value; generator configuration MAY switch to an envelope representation carrying them;
- `retry:` is surfaced through configuration only; automatic reconnection is out of scope for generated clients;
- comment lines (`:` prefix) are ignored;
- malformed events (invalid JSON, oversized per `max_stream_record_bytes`) yield `SseDecodeError` without collecting the rest of the stream.

Server-side encoding writes one JSON document per event with no `event:`/`id:` fields unless the configured envelope mode says otherwise. Failures raised by the application's event stream after commit follow section 40.

---

## 19. Example 12 — NDJSON

### OpenAPI input

```yaml
paths:
  /records/export:
    get:
      operationId: exportRecords
      responses:
        '200':
          description: Newline-delimited records
          content:
            application/x-ndjson:
              schema:
                $ref: '#/components/schemas/Record'
```

### Output A — Reqwest client

```rust
pub type ExportRecordsStream = std::pin::Pin<
    Box<dyn futures_core::Stream<Item = Result<Record, NdjsonDecodeError>> + Send>
>;

pub enum ExportRecordsResponse {
    Ok200(ExportRecordsStream),
}
```

The decoder buffers only enough data to find and decode one record, subject to a per-record limit such as `max_stream_record_bytes`.

### Output B — Axum server

```rust
pub type ExportRecordsStream = std::pin::Pin<
    Box<dyn futures_core::Stream<Item = Result<Record, ServerStreamError>> + Send>
>;

pub enum ExportRecordsResponse {
    Ok200(ExportRecordsStream),
}
```

The generated response encoder serializes one item at a time followed by a newline and writes it as body chunks.

---

## 20. Example 13 — JSON Text Sequences

### OpenAPI input

```yaml
responses:
  '200':
    content:
      application/json-seq:
        schema:
          $ref: '#/components/schemas/Metric'
```

### Output A — Reqwest client

```rust
pub enum StreamMetricsResponse {
    Ok200(JsonSeqStream<Metric>),
}
```

### Output B — Axum server

```rust
pub enum StreamMetricsResponse {
    Ok200(JsonSeqOutputStream<Metric>),
}
```

The supporting codec is incremental; each element is independently bounded.

---

## 21. Example 14 — unknown vendor media type

### OpenAPI input

```yaml
responses:
  '200':
    description: Proprietary document
    content:
      application/vnd.acme.document-v7:
        schema: {}
```

### Output A — Reqwest client

```rust
pub enum GetDocumentResponse {
    Ok200(reqwest::Response),
}
```

### Output B — Axum server

```rust
pub enum GetDocumentResponse {
    Ok200(axum::body::Body),
}
```

The generator sets/checks the declared media type but does not guess a codec. This makes unknown content safely streamable by default.

---

## 22. Example 15 — explicit media type versus wildcard fallback

### OpenAPI input

```yaml
responses:
  '200':
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Widget'
      '*/*':
        schema: {}
```

### Output A — Reqwest client

```rust
pub enum GetWidget200Content {
    Json(Widget),
    Any(reqwest::Response),
}

pub enum GetWidgetResponse {
    Ok200(GetWidget200Content),
}
```

Content matching chooses `application/json` before `*/*`.

### Output B — Axum server

```rust
pub enum GetWidget200Content {
    Json(Widget),
    Any {
        content_type: mime::Mime,
        body: axum::body::Body,
    },
}
```

The wildcard server variant requires the application to provide an actual `Content-Type` because `*/*` is not a concrete response media type.

---

## 23. Example 16 — status ranges (`2XX`, `4XX`)

OpenAPI permits wildcard response status ranges. Explicit codes take precedence over a range.

### OpenAPI input

```yaml
responses:
  '200':
    description: Normal result
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Widget'
  '2XX':
    description: Other successful result
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/SuccessEnvelope'
  '4XX':
    description: Client error
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum GetWidgetResponse {
    Ok200(Widget),
    Success2xx {
        status: http::StatusCode,
        body: SuccessEnvelope,
    },
    ClientError4xx {
        status: http::StatusCode,
        body: ProblemDetails,
    },
}
```

A literal 200 maps to `Ok200`, not `Success2xx`.

### Output B — Axum server

```rust
pub enum GetWidgetResponse {
    Ok200(Widget),
    Success2xx {
        status: http::StatusCode,
        body: SuccessEnvelope,
    },
    ClientError4xx {
        status: http::StatusCode,
        body: ProblemDetails,
    },
}
```

Generated server validation SHOULD ensure that the status passed to `Success2xx` is actually in 200-299 and `ClientError4xx` is in 400-499.

---

## 24. Example 17 — `default` response

### OpenAPI input

```yaml
responses:
  '200':
    description: Success
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Widget'
  default:
    description: Undocumented API error
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
```

### Output A — Reqwest client

```rust
pub enum GetWidgetResponse {
    Ok200(Widget),
    Default {
        status: http::StatusCode,
        body: ProblemDetails,
    },
}
```

### Output B — Axum server

```rust
pub enum GetWidgetResponse {
    Ok200(Widget),
    Default {
        status: http::StatusCode,
        body: ProblemDetails,
    },
}
```

A server implementation MAY return any status through the `Default` variant except statuses covered by more-specific documented variants; generated debug assertions or checked constructors SHOULD enforce this.

If there is no OpenAPI `default`/range matching an unexpected client status, the client returns `ClientError::UndocumentedStatus` instead of inventing an enum variant.

---

## 25. Example 18 — same status with JSON, problem JSON, and text

### OpenAPI input

```yaml
responses:
  '400':
    description: Invalid request in one of several negotiated formats
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
      application/json:
        schema:
          $ref: '#/components/schemas/LegacyError'
      text/plain:
        schema:
          type: string
```

### Output A — Reqwest client

```rust
pub enum Submit400Content {
    ProblemJson(ProblemDetails),
    Json(LegacyError),
    TextPlain(String),
}

pub enum SubmitResponse {
    BadRequest400(Submit400Content),
}
```

### Output B — Axum server

```rust
pub enum Submit400Content {
    ProblemJson(ProblemDetails),
    Json(LegacyError),
    TextPlain(String),
}

pub enum SubmitResponse {
    BadRequest400(Submit400Content),
}
```

All three finite representations are bounded according to the configured structured/text body limits.

---

## 26. Optional request bodies

### OpenAPI input

```yaml
requestBody:
  required: false
  content:
    application/json:
      schema:
        $ref: '#/components/schemas/Patch'
```

Generated client:

```rust
pub async fn patch_widget(
    &self,
    body: Option<&Patch>,
) -> Result<PatchWidgetResponse, ClientError>;
```

Generated server:

```rust
async fn patch_widget(
    &self,
    body: Option<Patch>,
) -> PatchWidgetResponse;
```

For multiple request media types:

```rust
Option<PatchWidgetRequestBody>
```

Absence of a body is different from a JSON body containing `null` and MUST be modeled separately. The precise empty-body and missing-`Content-Type` interactions are defined in sections 28.2 and 28.3.

---

## 27. Nullable schemas versus absent body

The generator MUST distinguish these cases:

```text
No HTTP body at all
JSON body `null`
JSON object with nullable property
```

Example nullable response schema may produce:

```rust
Ok200(Option<Widget>)
```

while a 204 response produces:

```rust
NoContent204
```

These are not interchangeable.

---

## 28. Body presence and Content-Type dispatch rules

Media-type matching precedence, for request decoding and client response decoding:

1. Parse the media type without treating parameters such as `charset=utf-8` as a different base type.
2. Prefer exact OpenAPI content entries.
3. Then match structured suffix families where applicable (`+json`).
4. Then media type ranges (`text/*`, `application/*`).
5. Finally `*/*`.
6. If nothing matches, return an unsupported/unexpected-content-type protocol error.

Example:

```text
Content-Type: application/problem+json; charset=utf-8
```

matches `application/problem+json` exactly.

A JSON-family policy may also recognize `application/vnd.foo+json` when the OpenAPI declaration is an appropriate JSON media range, but generated exact declarations remain authoritative.

### 28.1 Malformed and duplicate Content-Type headers

- A syntactically unparseable `Content-Type` value is never ignored or defaulted: it is a `400` protocol rejection on the server and a decode error on the client.
- Multiple conflicting `Content-Type` headers on one message are equally ambiguous and produce the same `400` rejection; generated code does not pick one arbitrarily.

### 28.2 Missing Content-Type versus body presence

| Situation | Server behavior | Client behavior |
|---|---|---|
| Required body, nonempty bytes, no `Content-Type` | `415` rejection | `ClientError::UnexpectedContentType` |
| Required body, empty bytes, no `Content-Type` | `400` rejection (missing required body) | decode error |
| Optional body, empty bytes | decoded as absent (`None`), regardless of headers | absent variant |
| Optional body, nonempty bytes, no `Content-Type` | `415` rejection; generated code does not sniff bytes to guess | `ClientError::UnexpectedContentType` |

Implementation invariant (peek-and-preserve): body emptiness cannot be inferred from `Content-Length`, which may be absent on chunked HTTP/1.1 and HTTP/2 transfers. For optional-body operations the generated router therefore decides presence by awaiting the first body data frame, subject to a small bounded peek cap (`peek_buffer_bytes`). When bytes arrive, the router preserves the peeked frame by re-prepending it to the stream passed onward for decoding; it never collects the whole body to determine emptiness and never discards buffered frames.

### 28.3 Empty bodies with Content-Type present

- An empty body on an optional-body operation is absent (`None`) even when a well-formed matching `Content-Type` is present. Header presence does not fabricate a value; absence of body bytes wins.
- A required body that is empty is a `400` rejection even when `Content-Type` matches a documented entry: an empty byte sequence is not JSON `null` (section 27).
- A documented JSON response status received with an empty body is a client decode error, never `Ok200` of some default value. Generated servers cannot emit this state because bounded encoders either write a document or fail through the section 34 fallback path.

### 28.4 Charset handling for textual media

- JSON-family decoding is UTF-8 per RFC 8259; a `charset` parameter is tolerated when it declares UTF-8 family encodings and otherwise yields a decode error.
- Plain-text families default to UTF-8 and honor a supported `charset` parameter via the codec layer; unsupported charsets produce decode errors rather than replacement-character corruption.
- Form bodies percent-decode as UTF-8 per the HTML form specification.

### 28.5 Wildcards in incoming requests

The precedence list above governs matching *documented* entries against concrete incoming types. Conversely, a wildcard *sent by the client* (for example `*/*` as request `Content-Type`) does not select among multiple documented request entries: the server rejects it with `415` unless exactly one entry exists, in which case the match MAY be accepted. Client-side response decoding is unaffected because servers may legitimately send any documented representation.

---

## 29. `Accept` generation

The request is sent before any response status is known, so `Accept` cannot depend on which status will occur. It is therefore defined **operation-wide**, not per response status:

- the candidate set is the deterministic union of every media type the operation can decode across **all** documented response statuses, including ranges expanded to concrete entries and `default`;
- ordering is deterministic: configured preference order first, then OpenAPI declaration order;
- duplicates collapse and quality values (`q=`) are not emitted by default;
- an operation admitting exactly one decodable media type sends that single entry explicitly.

Example for an operation whose responses admit JSON, problem JSON, and binary:

```http
Accept: application/json, application/problem+json, application/octet-stream
```

Because a server may ignore `Accept`, the type system must still handle every media type documented for every status.

On the server, returning an enum representation incompatible with the request's `Accept` is **the application's responsibility**: the generated router does not validate `Accept`, does not generate `406 Not Acceptable`, and treats the constructed variant as authoritative. Explicit enum construction remains the core server API. A future opt-in Tower middleware MAY add strict `Accept` enforcement without changing trait signatures.

---

## 30. Transport policies

Reqwest exposes transport behaviors that can silently alter protocol semantics. Because the generated client promises that documented statuses are observable through exhaustive enums, these behaviors MUST be pinned down instead of inheriting defaults accidentally.

### 30.1 Redirects

The generated client constructs its `reqwest::Client` with `reqwest::redirect::Policy::none()` by default.

Rationale:

- a documented `301`, `302`, `303`, `307`, or `308` must reach the caller as the corresponding exhaustive enum variant;
- transparent following would make those variants unreachable and would hide redirect outcomes from the caller;
- following a redirect mid-upload requires replaying a consumed streaming body, which Reqwest cannot do safely (section 31).

Callers who want automatic following opt in explicitly via a generated builder hook (for example `ClientBuilder::follow_redirects(policy)`). Even then, generated code does not buffer bodies to enable replay: a redirect encountered while a one-shot body is partially sent surfaces as `ClientError::RedirectRequiresReplayableBody`. With redirects disabled, an undocumented 3xx becomes `ClientError::UndocumentedStatus` unless a documented variant, range, or `default` matches.

### 30.2 Decompression and content coding

Transparent decompression (`gzip`, `br`, `deflate`, `zstd`) MAY be enabled via generator configuration. The policy is:

- structured-body limits count **decoded** bytes: `collect_reqwest_limited` measures the stream after content coding is removed. Decoded size is the real memory exposure and the meaningful bound against decompression bombs;
- streaming raw bodies pass through whatever Hyper/Reqwest provide, without total-size accounting; applications needing transfer caps while streaming count chunks themselves;
- generated code never branches on `Content-Encoding`: media-type classification (section 5) is orthogonal to content coding.

### 30.3 Other transport knobs

Timeouts, proxies, connection pooling, and TLS configuration are out of scope for this specification. The generated builder MAY expose them pass-through, but their defaults are Reqwest defaults, deliberately not overridden by the generator.

### 30.4 Server-side request content coding

Inbound requests are the more security-sensitive decompression path because attackers control them directly. For v1 the policy is **identity-only**: the generated router accepts requests whose `Content-Encoding` is absent or `identity`; any other coding yields a `415` `ProtocolRejection` (`UnsupportedContentCoding`) before any body byte is decoded. This closes the request-direction decompression-bomb path entirely.

Transparent request decompression with post-decompression `structured_request_bytes` accounting MAY be added later as an explicit opt-in generator feature; it MUST NOT run by default.

---

## 31. Streaming request ergonomics

The core client signature for a streaming body is:

```rust
body: reqwest::Body
```

Generated helpers MAY improve ergonomics:

```rust
pub async fn body_from_file(path: impl AsRef<Path>) -> io::Result<reqwest::Body>;
pub fn body_from_stream<S>(stream: S) -> reqwest::Body;
```

A generated operation SHOULD NOT require `Vec<u8>` for binary uploads.

A replayable request body is a separate concern. Reqwest streaming bodies are generally one-shot. Automatic retry logic MUST NOT retry a consumed one-shot body unless the generator or caller supplied an explicit body factory/reopenable source.

---

## 32. Streaming response ergonomics

For raw streaming response content, the client SHOULD preserve ownership of `reqwest::Response` because this retains:

- body streaming through `bytes_stream()`;
- headers;
- status/version metadata;
- Reqwest/Hyper backpressure behavior.

A generated wrapper may provide convenience methods but MUST NOT aggregate the body.

Example:

```rust
pub struct BinaryResponse {
    inner: reqwest::Response,
}

impl BinaryResponse {
    pub fn headers(&self) -> &http::HeaderMap { self.inner.headers() }

    pub fn into_stream(
        self,
    ) -> impl futures_core::Stream<Item = reqwest::Result<bytes::Bytes>> {
        self.inner.bytes_stream()
    }
}
```

Using the raw `reqwest::Response` directly is also acceptable and keeps generated dependencies smaller.

---

## 33. Structured body limits

Generated configuration SHOULD separate limits by purpose:

```rust
pub struct BodyLimits {
    pub structured_request_bytes: usize,
    pub structured_response_bytes: usize,
    pub error_response_bytes: usize,
    pub structured_encode_bytes: usize,   // bounded serialization (section 34)
    pub text_body_bytes: usize,
    pub multipart_scalar_part_bytes: usize,
    pub max_stream_record_bytes: usize,

    // Multipart cardinality (section 17.1)
    pub max_multipart_parts: usize,
    pub max_part_header_bytes: usize,
    pub max_field_name_bytes: usize,
    pub max_file_name_bytes: usize,
}
```

Structured-body limits apply to **decoded** representations (section 30.2): a compressed transfer that decompresses beyond the limit is rejected even though its wire size was smaller.

Binary/raw streams have no total-size memory limit because they are not accumulated. Applications may impose independent transfer-size limits for security/business reasons, but those limits should count bytes while streaming rather than buffer them.

A client/server may therefore reject a 20 MiB JSON document while still safely transferring a 500 GiB object stream.

---

## 34. Bounded serialization

Decoding is not the only unbounded path: encoding a large value into an unrestricted `Vec<u8>` violates the same invariant. Generated code therefore never calls `serde_json::to_vec`, `serde_json::to_string`, `serde_urlencoded::to_string`, or Axum's `Json<T>`/`Form<T>` responders directly for documented finite bodies. It uses support helpers that stop as soon as the encoded output exceeds the configured limit:

```rust
pub fn serialize_json_limited<T: serde::Serialize>(
    value: &T,
    limit: usize,
) -> Result<bytes::Bytes, EncodeTooLarge>;

pub fn serialize_form_limited<T: serde::Serialize>(
    value: &T,
    limit: usize,
) -> Result<bytes::Bytes, EncodeTooLarge>;
```

Implementation requirement: the limit MUST fail fast during serialization — a counting writer that errors once `limit` bytes have been produced — rather than serializing into memory first and checking afterward.

These helpers are used:

- on the server, for every bounded response encoding: JSON, problem JSON, forms, and multipart scalar/JSON metadata parts;
- on the client, for request encoding; Reqwest's `.json(body)` and `.form(form)` conveniences are NOT used by generated code because they buffer without a bound;
- by multipart builders when emitting bounded metadata parts (section 17).

### 34.1 Server encode overflow

`IntoResponse` construction stays infallible by design. If bounded serialization of a documented response exceeds `structured_encode_bytes`:

1. partial output is discarded; nothing partial is written to the wire;
2. the encoder emits the fixed protocol-safe fallback response: `500 InternalServerError` with an empty body;
3. the configured encode-overflow hook fires with the operation id, variant, and limit, so operators can observe the condition.

The fallback 500 may itself be undocumented by the OpenAPI operation. This is the single sanctioned deviation from "the enum describes everything on the wire", chosen deliberately over fallible handler signatures (section 48). Applications wanting explicit control can use generated checked constructors such as `try_into_response()`, which return `Err(EncodeTooLarge)` instead of producing the fallback internally.

### 34.2 Client request encode overflow

If serializing a request body exceeds the encode limit, the client method returns `ClientError::BodyTooLarge { direction: Encode, limit }` without sending anything.

---

## 35. Bodies that must not exist

Beyond the 204 rule in section 14, several statuses and methods have wire-level body semantics that differ from ordinary status/content generation. Generated code enforces all of them:

| Case | Generated server encoder | Generated client decoder |
|---|---|---|
| `204 No Content`, `205 Reset Content`, `304 Not Modified` | never writes a body; omits body framing headers it does not need | treats the body as empty even if bytes arrive; documented body schemas are ignored |
| `HEAD` requests | Axum/Hyper suppresses response bodies automatically; typed header fields are still written | decodes typed headers only; never reads or validates a body, even when `Content-Length` describes one |
| `1xx` informational | never produced by application code | never surfaces as an enum variant; handled below the operation layer |

A `HEAD` operation in OpenAPI typically documents the metadata (headers, media type, length) of the representation a `GET` would return while having no body to decode. The generated HEAD decoder therefore produces the status variant with typed headers and no body field, regardless of any documented content entry.

Consistency rule: informational statuses are transport-layer events, never operation outcomes. OpenAPI response entries that attempt to model them as ordinary operation results (`'100'`–`'199'` keys or `1XX` ranges) are rejected at parse/normalization time with a diagnostic rather than being emitted as unreachable enum variants. This is the explicit exception to "every documented response status becomes an enum variant" (section 53).

---

## 36. Errors versus documented responses

Generated client methods use two levels:

```rust
Result<OperationResponse, ClientError>
```

`OperationResponse` is the exhaustive OpenAPI response enum.

`ClientError` covers failures that prevent the caller from obtaining a documented response value, for example:

```rust
pub enum ClientError {
    Transport(reqwest::Error),
    InvalidUrl(...),
    BodyTooLarge { direction: BodyLimitDirection, limit: usize },
    RedirectRequiresReplayableBody,
    Decode { content_type: Option<mime::Mime>, source: ... },
    MissingRequiredHeader { name: http::HeaderName },
    InvalidHeader { name: http::HeaderName, source: ... },
    UnexpectedContentType { expected: ..., actual: ... },
    UndocumentedStatus { status: http::StatusCode },
}

pub enum BodyLimitDirection {
    /// Request serialization exceeded `structured_encode_bytes` (section 34.2).
    Encode,
    /// Response collection exceeded a decode limit.
    Decode,
}
```

This section is the single authoritative definition of `ClientError`. Every other section of this specification references variants defined here; generated code MUST NOT invent ad-hoc client error shapes outside this enum.

A documented `404` is NOT `ClientError`; it is an enum variant.

This distinction is one of the primary design requirements.

---

## 37. Server errors

The generated API trait SHOULD return the documented response enum directly for normal protocol outcomes.

Application-internal failures may be represented according to one of two generator modes:

### Mode A — application converts failures itself

```rust
async fn get_widget(...) -> GetWidgetResponse;
```

The implementation must translate internal errors to documented variants.

### Mode B — generated application error hook

```rust
async fn get_widget(...)
    -> Result<GetWidgetResponse, AppError>;
```

The router receives an `AppErrorMapper` that converts `AppError` into an HTTP response. This response is operationally outside the OpenAPI response enum unless the mapper deliberately maps to a documented variant.

Mode A is the simplest and most protocol-exhaustive default.

---

## 38. Request validation before handler invocation

Generated server routing SHOULD validate before calling application code:

- path/query/header parameter syntax;
- required parameters;
- `Content-Type` compatibility;
- request `Content-Encoding` is absent or `identity` (section 30.4);
- bounded structured body size;
- JSON/form syntax;
- required structured fields through Serde/schema validation policy.

Wiring requirements:

- every route installs `axum::extract::DefaultBodyLimit::max(...)` matching the purpose-specific limit for that operation (`structured_request_bytes` for buffered bodies); streaming-body operations are exempt because nothing aggregates them;
- extractor rejections are never surfaced as Axum's default text/plain responses: they are translated into the canonical `ProtocolRejection` mapping of section 39;
- multipart routes enforce the cardinality limits of section 17.1 during parsing.

Raw streaming bodies cannot be fully semantically validated before the application consumes them. Validation that depends on the complete streamed payload must therefore be incremental or application-owned.

The generator MUST NOT buffer a raw body merely to claim complete validation.

---

## 39. Pre-handler protocol rejections

Validation failures detected before handler invocation (section 38) are reported through a generated `ProtocolRejection` type, not through the operation's documented response enum:

```rust
pub struct ProtocolRejection {
    pub kind: RejectionKind,
    // diagnostic detail for logs/observation; not guaranteed on the wire
}

pub enum RejectionKind {
    InvalidParameter,     // path/query/header syntax, missing required parameter
    MalformedBody,        // syntactically invalid JSON/form/multipart framing
    SchemaViolation,      // well-formed body violating the schema
    BodyTooLarge,         // bounded collection limit exceeded
    UnsupportedMediaType, // missing/unmatched Content-Type where a body is admitted
    UnsupportedContentCoding, // request Content-Encoding other than absent/identity (section 30.4)
}
```

Canonical status mapping:

| Condition | Status |
|---|---|
| Invalid or missing required path/query/header parameter | `400` |
| Syntactically malformed JSON/form/multipart framing | `400` |
| Body exceeding the configured structured limit | `413` |
| Missing, unparsable, wildcard, or unmatched `Content-Type` on a body-bearing request | `415` |
| Request `Content-Encoding` other than absent/`identity` | `415` |
| Well-formed body failing Serde/schema validation (e.g. missing required fields) | `422` |

Contract-boundary rules:

1. Rejections live **outside the documented operation enum**. The router emits them directly; the handler never observes them. Generated code never synthesizes instances of documented body types to fill a matching documented variant, because it cannot invent valid domain data.
2. A rejection may therefore produce a status the operation never documents. The claim that "the Rust enum exposes the API contract" holds for application-produced responses; infrastructure-level input validation sits below the operation layer, symmetric with `404 Unknown Route` and `405 Method Not Allowed`.
3. By default a rejection response carries only the canonical status with an empty body, keeping invented schemas off the wire. Generator configuration MAY switch rejections to a canned minimal RFC 9457 problem document under `application/problem+json`; this remains generator-owned and never uses application schema types.
4. The client side needs no special casing: whatever the server actually sends decodes normally, so a peer-generated rejection still matches a documented variant, range, or `default` when one exists, and otherwise surfaces as `ClientError::UndocumentedStatus`.

---

## 40. Failures after a streaming response has started

Server streams for SSE, NDJSON, JSON Sequences, and binary bodies yield `Result<_, ServerStreamError>` items. Once the encoder has committed the response — status and headers written, first body bytes flushed — the HTTP status can no longer change. The generated contract is:

1. **No fabricated statuses.** The encoder MUST NOT upgrade a committed response to `InternalServerError500` and MUST NOT inject in-band error frames: SSE, NDJSON, and JSON Text Sequences define no standard in-scope error representation.
2. **Terminate abruptly.** A stream item error ends the body immediately; the encoder aborts the connection so clients observe truncation rather than clean EOF.
3. **Observe.** The configured stream-failure hook fires with the operation id and error before the stream is dropped.
4. **Prefer pre-commit failure.** Errors known before returning the stream variant should be modeled as documented error variants instead; stream variants exist for failures that occur during production.

Client-visible effect: premature termination surfaces as an explicit truncated-stream decode error (`SseDecodeError::Truncated`, `NdjsonDecodeError::Truncated`, `JsonSeqDecodeError::Truncated`) — distinguishable from clean end-of-stream so callers never mistake truncation for success. Raw binary streams follow the same rule: an `Err` chunk from an `axum::body::Body` aborts the response abnormally.

This is the streaming counterpart of the bounded-encode fallback in section 34: both exist because HTTP forbids changing a committed response.

---

## 41. Response serialization

The server encoder follows the response enum in two stages:

```text
operation response enum
        |
        +-- status variant
                |
                +-- optional content enum
                        |
                        +-- concrete encoder / stream
```

Examples:

```text
Ok200(Json(widget))        -> 200 + application/json + bounded JSON serialization (section 34)
Ok200(OctetStream(body))   -> 200 + application/octet-stream + body unchanged
NotFound404(problem)       -> 404 + application/problem+json
NoContent204               -> 204 + empty body (section 35)
```

Bounded encoders stop at `structured_encode_bytes` and fall back per section 34.1 rather than writing partial bodies. Streaming variants encode incrementally and follow the post-commit failure contract of section 40.

For very large JSON generation, a future optional streaming JSON encoder may be supported for schemas that map naturally to sequences, but ordinary object-shaped JSON remains a finite document by default.

---

## 42. Multiple response media types plus multiple status codes

This produces two levels of enums, never a flattened cross-product unless configured otherwise.

### OpenAPI input

```yaml
responses:
  '200':
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Metadata'
      application/octet-stream:
        schema:
          type: string
          format: binary
  '202':
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Job'
  '404':
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
```

### Generated conceptual structure

```rust
pub enum Fetch200Content<B> {
    Json(Metadata),
    OctetStream(B),
}

pub enum FetchResponse<B> {
    Ok200(Fetch200Content<B>),
    Accepted202(Job),
    NotFound404(ProblemDetails),
}
```

In actual generated direct-framework modules, `B` is not necessarily exposed as a generic:

```text
client module B = reqwest::Response
server module B = axum::body::Body
```

The nested shape keeps the two independent protocol dimensions visible:

```text
first:  which HTTP status?
second: which representation for that status?
```

---

## 43. Requests with multiple media types and optionality

### OpenAPI input

```yaml
requestBody:
  required: false
  content:
    application/json:
      schema:
        $ref: '#/components/schemas/Command'
    text/plain:
      schema:
        type: string
    application/octet-stream:
      schema:
        type: string
        format: binary
```

### Reqwest output

```rust
pub enum ExecuteRequestBody {
    Json(Command),
    TextPlain(String),
    OctetStream(reqwest::Body),
}

pub async fn execute(
    &self,
    body: Option<ExecuteRequestBody>,
) -> Result<ExecuteResponse, ClientError>;
```

### Axum output

```rust
pub enum ExecuteRequestBody {
    Json(Command),
    TextPlain(String),
    OctetStream(axum::body::Body),
}

async fn execute(
    &self,
    body: Option<ExecuteRequestBody>,
) -> ExecuteResponse;
```

---

## 44. Generic textual and binary schemas

OpenAPI schema metadata does not override the actual media type's transport semantics.

Examples:

```yaml
content:
  application/octet-stream:
    schema:
      type: string
      format: binary
```

maps to a stream.

A custom binary media type with an empty schema:

```yaml
content:
  image/png: {}
```

also maps to a stream.

A textual value:

```yaml
content:
  text/plain:
    schema:
      type: string
```

maps to bounded `String` by default.

If users need unbounded plain-text streaming, the generator SHOULD support an extension/configuration such as:

```yaml
x-rust-body: stream
```

or a generator-side override without requiring changes to the OpenAPI document.

---

## 45. Optional typed codecs

The media-type classifier should be extensible.

For example, enabling an XML codec could change:

```text
application/xml
```

from:

```text
Reqwest: reqwest::Response
Axum:    axum::body::Body
```

into:

```text
Reqwest: generated T via bounded XML decode
Axum:    generated T via bounded XML decode/encode
```

The same applies to CBOR, MessagePack, Protobuf, etc.

The fallback remains raw streaming, so unsupported content is never silently discarded or eagerly loaded.

---

## 46. Recommended generated API example in one view

Given:

```yaml
operationId: fetchReport
requestBody:
  content:
    application/json:
      schema:
        $ref: '#/components/schemas/ReportQuery'
    application/octet-stream:
      schema:
        type: string
        format: binary
responses:
  '200':
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/Report'
      application/pdf:
        schema:
          type: string
          format: binary
  '202':
    content:
      application/json:
        schema:
          $ref: '#/components/schemas/ReportJob'
  '400':
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
  '404':
    content:
      application/problem+json:
        schema:
          $ref: '#/components/schemas/ProblemDetails'
```

The client surface is approximately:

```rust
pub enum FetchReportRequestBody {
    Json(ReportQuery),
    OctetStream(reqwest::Body),
}

pub enum FetchReport200Content {
    Json(Report),
    Pdf(reqwest::Response),
}

pub enum FetchReportResponse {
    Ok200(FetchReport200Content),
    Accepted202(ReportJob),
    BadRequest400(ProblemDetails),
    NotFound404(ProblemDetails),
}

impl Client {
    pub async fn fetch_report(
        &self,
        body: FetchReportRequestBody,
    ) -> Result<FetchReportResponse, ClientError>;
}
```

The server surface is approximately:

```rust
pub enum FetchReportRequestBody {
    Json(ReportQuery),
    OctetStream(axum::body::Body),
}

pub enum FetchReport200Content {
    Json(Report),
    Pdf(axum::body::Body),
}

pub enum FetchReportResponse {
    Ok200(FetchReport200Content),
    Accepted202(ReportJob),
    BadRequest400(ProblemDetails),
    NotFound404(ProblemDetails),
}

#[async_trait::async_trait]
pub trait ReportsApi: Send + Sync + 'static {
    async fn fetch_report(
        &self,
        body: FetchReportRequestBody,
    ) -> FetchReportResponse;
}
```

This example captures the core design of the generator.

---

## 47. Compile-time exhaustiveness guarantee

The generated enums are intentionally not marked `#[non_exhaustive]` by default.

When the generated OpenAPI client is regenerated after the API adds a newly documented status or representation, downstream exhaustive `match` expressions fail to compile until the developer handles the new variant.

This is considered a feature:

```text
API contract changes
      -> regenerated enum changes
      -> compiler identifies unhandled protocol case
```

For library authors who require semver-friendly generated crates, the generator MAY offer an opt-in `non_exhaustive` mode, but that weakens the explicit-handling property and is not the default described by this specification.

---

## 48. Generated response conversion should be infallible where practical

On the server, constructing a documented response variant should make protocol serialization as close to infallible as reasonable.

For headers that can fail conversion, prefer checked constructors:

```rust
impl CreateWidget201 {
    pub fn new(
        location: impl TryInto<http::HeaderValue>,
        body: Widget,
    ) -> Result<Self, InvalidResponseHeader>;
}
```

or store domain values and let `IntoResponse` convert them with a well-defined internal error path.

For range statuses, prefer checked constructors:

```rust
impl GetWidgetResponse {
    pub fn success_2xx(
        status: StatusCode,
        body: SuccessEnvelope,
    ) -> Result<Self, InvalidStatusRange>;
}
```

This prevents an invalid `404` from being accidentally placed in a `Success2xx` variant.

Infallibility has exactly one sanctioned exception: encode overflow (section 34.1) cannot fail the constructor without reintroducing fallible signatures, so the generated encoder discards the partial output, emits the fixed empty-bodied `500` fallback, and fires the overflow hook. Checked constructors such as `try_into_response()` remain available for applications that prefer explicit handling of that case.

---

## 49. What generated code MUST NOT do

The following patterns are forbidden for potentially unbounded bodies:

```rust
// Forbidden for binary/raw body paths
let bytes = response.bytes().await?;
let text = response.text().await?;
let all = axum::body::to_bytes(body, usize::MAX).await?;
let file: Vec<u8> = ...;
```

The following is also forbidden unless a configured finite bound is applied:

```rust
serde_json::from_slice(&response.bytes().await?)
```

Unbounded serialization is equally forbidden on documented finite-body paths:

```rust
// Forbidden: buffers the full document before any size check
serde_json::to_vec(&value)?
serde_urlencoded::to_string(&form)?          // same
axum::Json(value)                            // same, on the server responder path
```

Preferred structured pattern:

```rust
let bytes = collect_limited(response, configured_limit).await?;
let value = serde_json::from_slice(&bytes)?;
```

Bounded encode counterpart (section 34):

```rust
let payload = serialize_json_limited(&value, limits.structured_encode_bytes)?;
```

Preferred streaming pattern:

```rust
let mut stream = response.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?).await?;
}
```

or on Axum:

```rust
let mut stream = body.into_data_stream();
```

---

## 50. Tests the generator MUST have

The generator/runtime support test suite should cover at least:

1. JSON request under limit succeeds.
2. JSON request over limit fails before excessive allocation.
3. JSON response under limit succeeds.
4. JSON response over limit returns a body-limit error.
5. 10+ GiB synthetic upload can pass through a streaming test without memory proportional to payload size.
6. 10+ GiB synthetic download can pass through without aggregation.
7. Streaming backpressure: producer does not run arbitrarily ahead of consumer.
8. Cancellation stops upload/download work promptly.
9. Binary response error path remains bounded when the error body is JSON.
10. Multiple response status enum decodes each explicit status.
11. Explicit status takes precedence over `2XX`/`4XX` range.
12. `default` captures otherwise unmatched statuses.
13. Nested content enum selects correct media type.
14. Exact media type takes precedence over wildcard.
15. Multipart file remains streaming.
16. Multipart scalar/JSON part limit is enforced.
17. SSE produces events incrementally.
18. NDJSON handles records split across arbitrary HTTP chunks.
19. NDJSON rejects one oversized record without collecting the complete stream.
20. Required response headers are parsed and validated.
21. 204 never writes a response body.
22. Optional body distinguishes absent body from JSON `null`.

Contract-boundary tests:

23. Bounded serialization stops early: `serialize_json_limited` on an oversized value allocates memory bounded by the limit, not by the encoded size.
24. Server encode overflow produces the fixed empty-bodied 500 fallback, fires the hook, and never writes partial output.
25. Client request encode overflow returns `ClientError::BodyTooLarge` without sending.
26. Rejection matrix: each `RejectionKind` maps to its canonical status (400/413/415/422); handler is not invoked; documented enum is not synthesized.
27. Malformed and duplicate `Content-Type` headers produce 400 rejections / decode errors per section 28.1.
28. Missing `Content-Type` with nonempty required body yields 415; with optional body and no bytes yields absent; with nonempty bytes yields 415 (section 28.2).
29. Empty 200 `application/json` response decodes as an error, not a default value (section 28.3).
30. Charset handling: UTF-8 JSON passes; unsupported charsets error instead of corrupting (section 28.4).
31. Redirect policy defaults to none: a documented 307 reaches its enum variant; opt-in following surfaces `RedirectRequiresReplayableBody` for consumed streams.
32. Body limits count decoded bytes: a gzip bomb exceeding `structured_response_bytes` decoded is rejected despite small wire size.
33. Post-commit stream failure terminates the connection abruptly; client observes `Truncated`, distinct from clean EOF; hook fires (section 40).
34. No-body statuses: 205/304/HEAD carry no body in both directions; HEAD decodes typed headers only (section 35).
35. Operation-wide `Accept` header equals the deterministic union across all statuses (section 29).
36. Multipart cardinality: part-count limit, part-header limit, field/file-name limits enforced incrementally; duplicate scalar fields reject 422; repeated array fields collect in order (section 17.1).
37. SSE framing: multi-line data joins before parse; comments ignored; malformed events yield `SseDecodeError`; oversized events yield per-record errors (section 18.2).

Reproducibility and conformance testing:

38. Generated code compiles under `cargo check`/`clippy -D warnings` for every fixture spec; compile-failure fixtures use `trybuild`.
39. Generation is deterministic byte-for-byte: no timestamps, paths, or map-iteration order in output; golden/snapshot files cover every example section of this document.
40. Generated output is `rustfmt`-clean.
41. Fuzz targets for malformed JSON/form/multipart/SSE input assert bounded memory and rejection without panics.
42. Property tests decode NDJSON/JSON-seq/SSE bodies split at every possible chunk boundary against unsplit decoding.
43. Differential round trips: generated client against generated server over a real listener must reproduce identical values for every operation in the fixture corpus.
44. MSRV build: CI builds generated crates on the pinned MSRV toolchain.
45. Identity-only inbound content coding: a gzipped request body yields `415` before any decoding (section 30.4).
46. Optional-body presence detection works without `Content-Length` (chunked transfer): presence decided by first-frame peek and peeked bytes are delivered exactly once downstream (section 28.2).
47. Presence/nullability matrix: a missing required-nullable property fails validation; an explicit `null` on an optional non-nullable property fails (companion section 2.1).
48. `oneOf` ambiguity: a document validating more than one branch is rejected rather than resolved by declaration order (companion section 4.2).

Memory tests should use a synthetic producer that generates far more bytes than allowed by process memory, proving behavior rather than relying only on code inspection.

---

## 51. Open design questions

The following details should be decided before implementation stabilizes:

### 51.1 Should binary client responses expose raw `reqwest::Response` or a generated wrapper?

**Raw response advantages:** zero abstraction, fewer generated types, direct Reqwest API.  
**Wrapper advantages:** typed documented headers, consistent methods, hides unrelated Reqwest methods.

Recommended initial choice: generated status wrapper containing typed headers plus the raw `reqwest::Response` when needed.

### 51.2 Should single-status operations still use a response enum?

Two reasonable policies:

- Always enum: strongest consistency and future codegen predictability.
- Direct type when exactly one documented status: less boilerplate.

Recommended choice: **always generate a response enum** for public operation results. This preserves status semantics and makes later API expansion mechanically consistent.

### 51.3 Should request content enums own or borrow JSON values?

Owning enums are easiest:

```rust
Json(CreateWidget)
```

but can cause moves/clones. A borrowing generated request enum adds lifetimes and cannot uniformly hold a `reqwest::Body` as elegantly.

Recommended initial choice: own operation request enums; provide convenience methods taking references for single-content JSON operations.

### 51.4 Multipart server API shape

This is likely the most technically sensitive API because multipart fields are sequential and parser implementations may tie a field lifetime to the parent multipart parser.

The design MUST prioritize streaming correctness over pretending multipart is a normal in-memory Rust struct.

### 51.5 XML and other codecs

Recommended initial implementation scope:

```text
JSON
plain text
URL encoded form
raw/binary
multipart/form-data
SSE
NDJSON
JSON Sequence
```

Other formats use streaming raw-body fallback until codec plugins are added.

---

## 52. Initial implementation milestones

### Phase 0 — invariants and normalization

These decisions shape generated public APIs, so they are locked before code generation begins:

- bounded serializer helpers (`serialize_json_limited`, `serialize_form_limited`, counting-writer contract) and the encode-overflow fallback/hook policy (section 34);
- protocol rejection type, canonical status mapping, and the outside-the-enum rule (section 39);
- body-presence / Content-Type state machine including empty-body, malformed-header, and charset rules (section 28);
- client transport policies: redirects off by default, decompression with decoded-byte accounting (section 30);
- post-commit streaming failure contract and truncated-stream decode errors (section 40);
- stream item-schema convention plus `x-rust-stream-item` override (section 18.1);
- IR/normalization contract per the companion document: `$ref` resolution incl. external/cyclic refs, OpenAPI 3.0→3.1 normalization, composition keywords, discriminator handling, naming/keyword-collision rules;
- deterministic-generation requirements (no timestamps, stable ordering) so golden tests are possible from day one.

### Phase 1 — core protocol shapes

- OpenAPI parser/normalizer.
- Shared schema generation with Serde.
- Status response enums.
- Nested media-type enums.
- JSON bounded encode/decode.
- Raw streaming body request/response.
- Reqwest client generation.
- Axum server/router generation.

### Phase 2 — body-rich APIs

- URL encoded forms.
- typed/streaming multipart.
- typed response headers.
- content negotiation and wildcards.
- status ranges and `default`.

### Phase 3 — streaming structured protocols

- SSE.
- NDJSON.
- JSON Text Sequences.
- per-record limits and cancellation tests.

### Phase 4 — extensibility

- media-type codec plugin interface.
- XML/CBOR/MessagePack/etc. integrations.
- custom generator overrides.
- replayable upload helpers and retry policy integration.

---

## 53. Normative design summary

The generator SHOULD behave according to these concise rules:

1. **Generate directly for Reqwest and Axum.**
2. **Share schema models, not HTTP body abstractions.**
3. **Every documented final response status becomes an enum variant.** Informational (1xx) statuses are transport-layer events, never operation outcomes, and entries modeling them are rejected at parse time (section 35).
4. **Every set of alternative content types becomes a nested enum.**
5. **Documented HTTP outcomes are values, not `Result::Err`.**
6. **Transport/decode/protocol failures use `Result::Err`.**
7. **JSON/form/small textual documents are bounded before materialization.**
8. **Binary, file-like, unknown, and continuous bodies remain streaming.**
9. **Multipart is incremental and file parts remain streaming.**
10. **SSE, NDJSON, and JSON Sequence are incremental typed streams.**
11. **Unknown media types fall back to raw streaming instead of eager buffering.**
12. **Exact media types/statuses take precedence over wildcard media types/status ranges.**
13. **A `default` response becomes a status-carrying enum variant.**
14. **Generated server response variants determine status and content type explicitly.**
15. **Generated code never performs unbounded whole-body collection or unbounded serialization.**
16. **Bounded encoders stop at the configured limit; server overflow yields a fixed empty-bodied 500 plus an observable hook, client overflow yields `BodyTooLarge`.**
17. **Pre-handler protocol rejections use canonical statuses (400/413/415/422) and stay outside the documented operation enum.**
18. **After a streaming response is committed, failures terminate the body, fire a hook, and surface as truncated-stream errors — never as fabricated statuses.**
19. **Redirects default to off; decompression limits account for decoded bytes.**
20. **`Accept` is computed per operation as a deterministic union; server-side representation choice is application-authoritative.**
21. **204/205/304/HEAD/1xx carry no body in either direction; HEAD decodes headers only.**
22. **Stream-typed media types interpret their schema as the per-item type unless `x-rust-stream-item` overrides it.**

The intended developer experience is that the OpenAPI contract is visible directly in Rust's type system while large HTTP bodies retain the natural streaming and backpressure behavior already provided by Reqwest/Hyper and Axum/Hyper.

---

## 54. Research basis

This draft is based on the behavior and APIs of the current Rust HTTP ecosystem and OpenAPI response/content semantics, including:

- OpenAPI Specification 3.1.x/3.2 response objects, media type objects, response status ranges, multipart/file semantics, and content maps.
- Axum 0.8 body, JSON/Form/Multipart extractors, `IntoResponse`, and SSE support.
- Reqwest asynchronous request bodies, streaming request bodies, multipart, and response byte streams.
- `http` / `http-body` / `bytes` conventions used under the Tokio/Hyper ecosystem.
- Comparative review of Progenitor, OpenAPI Generator's Rust/Reqwest and Rust/Axum generators, and `openapi-to-rust`, particularly their buffering and streaming behavior.

This specification deliberately adopts a stricter invariant than the reviewed generators: **potentially unbounded bodies are streaming by construction, while any intentional materialization is bounded and visible in the generated codec path.**

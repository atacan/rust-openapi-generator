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

This document focuses on request/response body and response-status generation. Parameter serialization, authentication, callbacks, links, and schema-generation details are outside the core scope except where needed by examples.

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

The generated `support` module SHOULD be small and contain only reusable helpers such as bounded collection, content-type matching, percent encoding, structured decode errors, and event-stream codecs. It MUST NOT define a parallel HTTP transport abstraction.

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

### 5.7 Newline-delimited JSON

Recognized aliases MAY include:

```text
application/x-ndjson
application/ndjson
application/jsonl
```

Representation: asynchronous stream of decoded items.

Each logical JSON record is bounded independently; the entire body is not.

### 5.8 JSON Text Sequences

```text
application/json-seq
```

Representation: asynchronous stream of decoded items framed according to JSON Text Sequences.

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
        let response = self.http
            .post(self.url("/widgets")?)
            .json(body)
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

Generated router logic performs bounded JSON extraction and then dispatches to the trait. Generated `IntoResponse` logic maps each enum variant to the documented status and media type.

```rust
impl axum::response::IntoResponse for CreateWidgetResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Created201(body) => (
                http::StatusCode::CREATED,
                axum::Json(body),
            ).into_response(),

            Self::BadRequest400(body) => {
                // Content-Type is application/problem+json, not generic application/json.
                encode_problem_json(http::StatusCode::BAD_REQUEST, body)
            }
        }
    }
}
```

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
            CreateImportRequestBody::Json(value) => request.json(&value),
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

The generated response serializer MUST ensure the 204 variant does not emit a body.

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
    let response = self.http
        .post(self.url("/sessions")?)
        .form(form)
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

The generated route may use Axum `Form<T>` when the operation accepts only this one request media type and the configured body limit has been applied. If content negotiation is required, the route decodes from the raw body after matching `Content-Type`.

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

Absence of a body is different from a JSON body containing `null` and MUST be modeled separately.

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

## 28. Content-Type dispatch rules

For request decoding and client response decoding:

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

---

## 29. `Accept` generation

When a response status can use multiple media types, the Reqwest client SHOULD send an `Accept` header listing supported representations in deterministic order.

For example:

```http
Accept: application/json, application/octet-stream
```

Generator configuration MAY provide preference ordering. The type system must still handle every media type documented for the status because the server may legitimately choose any negotiated representation.

For server routing, generated code SHOULD validate request `Content-Type`; response content selection is controlled by the returned nested enum variant. Optional automatic `Accept` negotiation may be added later, but explicit enum construction remains the core server API.

---

## 30. Streaming request ergonomics

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

## 31. Streaming response ergonomics

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

## 32. Structured body limits

Generated configuration SHOULD separate limits by purpose:

```rust
pub struct BodyLimits {
    pub structured_request_bytes: usize,
    pub structured_response_bytes: usize,
    pub error_response_bytes: usize,
    pub text_body_bytes: usize,
    pub multipart_scalar_part_bytes: usize,
    pub max_stream_record_bytes: usize,
}
```

Binary/raw streams have no total-size memory limit because they are not accumulated. Applications may impose independent transfer-size limits for security/business reasons, but those limits should count bytes while streaming rather than buffer them.

A client/server may therefore reject a 20 MiB JSON document while still safely transferring a 500 GiB object stream.

---

## 33. Errors versus documented responses

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
    BodyTooLarge { limit: usize },
    Decode { content_type: Option<mime::Mime>, source: ... },
    MissingRequiredHeader { name: http::HeaderName },
    InvalidHeader { name: http::HeaderName, source: ... },
    UnexpectedContentType { expected: ..., actual: ... },
    UndocumentedStatus { status: http::StatusCode },
}
```

A documented `404` is NOT `ClientError`; it is an enum variant.

This distinction is one of the primary design requirements.

---

## 34. Server errors

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

## 35. Request validation before handler invocation

Generated server routing SHOULD validate before calling application code:

- path/query/header parameter syntax;
- required parameters;
- `Content-Type` compatibility;
- bounded structured body size;
- JSON/form syntax;
- required structured fields through Serde/schema validation policy.

Raw streaming bodies cannot be fully semantically validated before the application consumes them. Validation that depends on the complete streamed payload must therefore be incremental or application-owned.

The generator MUST NOT buffer a raw body merely to claim complete validation.

---

## 36. Response serialization

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
Ok200(Json(widget))        -> 200 + application/json + bounded JSON serialization
Ok200(OctetStream(body))   -> 200 + application/octet-stream + body unchanged
NotFound404(problem)       -> 404 + application/problem+json
NoContent204               -> 204 + empty body
```

For very large JSON generation, a future optional streaming JSON encoder may be supported for schemas that map naturally to sequences, but ordinary object-shaped JSON remains a finite document by default.

---

## 37. Multiple response media types plus multiple status codes

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

## 38. Requests with multiple media types and optionality

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

## 39. Generic textual and binary schemas

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

## 40. Optional typed codecs

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

## 41. Recommended generated API example in one view

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

## 42. Compile-time exhaustiveness guarantee

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

## 43. Generated response conversion should be infallible where practical

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

---

## 44. What generated code MUST NOT do

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

Preferred structured pattern:

```rust
let bytes = collect_limited(response, configured_limit).await?;
let value = serde_json::from_slice(&bytes)?;
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

## 45. Tests the generator MUST have

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

Memory tests should use a synthetic producer that generates far more bytes than allowed by process memory, proving behavior rather than relying only on code inspection.

---

## 46. Open design questions

The following details should be decided before implementation stabilizes:

### 46.1 Should binary client responses expose raw `reqwest::Response` or a generated wrapper?

**Raw response advantages:** zero abstraction, fewer generated types, direct Reqwest API.  
**Wrapper advantages:** typed documented headers, consistent methods, hides unrelated Reqwest methods.

Recommended initial choice: generated status wrapper containing typed headers plus the raw `reqwest::Response` when needed.

### 46.2 Should single-status operations still use a response enum?

Two reasonable policies:

- Always enum: strongest consistency and future codegen predictability.
- Direct type when exactly one documented status: less boilerplate.

Recommended choice: **always generate a response enum** for public operation results. This preserves status semantics and makes later API expansion mechanically consistent.

### 46.3 Should request content enums own or borrow JSON values?

Owning enums are easiest:

```rust
Json(CreateWidget)
```

but can cause moves/clones. A borrowing generated request enum adds lifetimes and cannot uniformly hold a `reqwest::Body` as elegantly.

Recommended initial choice: own operation request enums; provide convenience methods taking references for single-content JSON operations.

### 46.4 Multipart server API shape

This is likely the most technically sensitive API because multipart fields are sequential and parser implementations may tie a field lifetime to the parent multipart parser.

The design MUST prioritize streaming correctness over pretending multipart is a normal in-memory Rust struct.

### 46.5 XML and other codecs

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

## 47. Initial implementation milestones

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

## 48. Normative design summary

The generator SHOULD behave according to these concise rules:

1. **Generate directly for Reqwest and Axum.**
2. **Share schema models, not HTTP body abstractions.**
3. **Every documented response status becomes an enum variant.**
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
15. **Generated code never performs unbounded whole-body collection.**

The intended developer experience is that the OpenAPI contract is visible directly in Rust's type system while large HTTP bodies retain the natural streaming and backpressure behavior already provided by Reqwest/Hyper and Axum/Hyper.

---

## 49. Research basis

This draft is based on the behavior and APIs of the current Rust HTTP ecosystem and OpenAPI response/content semantics, including:

- OpenAPI Specification 3.1.x/3.2 response objects, media type objects, response status ranges, multipart/file semantics, and content maps.
- Axum 0.8 body, JSON/Form/Multipart extractors, `IntoResponse`, and SSE support.
- Reqwest asynchronous request bodies, streaming request bodies, multipart, and response byte streams.
- `http` / `http-body` / `bytes` conventions used under the Tokio/Hyper ecosystem.
- Comparative review of Progenitor, OpenAPI Generator's Rust/Reqwest and Rust/Axum generators, and `openapi-to-rust`, particularly their buffering and streaming behavior.

This specification deliberately adopts a stricter invariant than the reviewed generators: **potentially unbounded bodies are streaming by construction, while any intentional materialization is bounded and visible in the generated codec path.**

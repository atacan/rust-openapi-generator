/// Reqwest client generated from the OpenAPI document (main spec §8 Output A).
///
/// Bounded JSON/form bodies (§34), streaming raw payloads (§32), exhaustive documented-status enums (§2.4), typed documented response headers (§15), redirects off by default (§30.1), and the authoritative `ClientError` (§36). Recorded decision for multi-content statuses WITH documented headers: the typed fields hoist onto the status VARIANT beside the content enum. The source document declares OpenAPI 3.1.0.
///
/// Servers (companion §8): operation-level `servers` override path-level, path-level overrides root-level, and within each effective array the first entry is that operation's default base. Every DISTINCT effective default URL becomes its own stored base: `base_url` is the primary (the first operation's first effective server); further bases live in `base_url_<key>` fields whose keys are documented under `ClientBuilder::secondary_base_url`. Recorded decision: an explicit `base_url` replaces ONLY the primary base; each other base needs its own `secondary_base_url` override, so a relative secondary still requires an absolute value there (D-impl-relative-servers).
/// Generated deterministically byte-for-byte (main spec §50 test 39); do not edit by hand.
use super::models::{Ack, Event, EventPayload, Metric, ProblemDetails, Record};
use ::openapi_support::client_error::{BodyLimitDirection, ClientError};
use ::openapi_support::collect::collect_reqwest_limited;
use ::openapi_support::jsonseq::encode_jsonseq_item;
use ::openapi_support::limits::BodyLimits;
use ::openapi_support::mediatype::ParsedMediaType;
use ::openapi_support::ndjson::decode_ndjson;
use ::openapi_support::sse::decode_sse_json;
use ::openapi_support::stream_errors::{NdjsonDecodeError, SseDecodeError};

/// Client carrying one resolved base per distinct effective default server (companion §8): `base_url` is the PRIMARY base (the first operation's first effective server); every further distinct URL gets its own `base_url_<key>` field, and each generated method sends through its operation's own base.
#[derive(Clone)]
pub struct Client {
    http: ::reqwest::Client,
    base_url: String,
    limits: BodyLimits,
}

/// Builder for `Client` (main spec §30.1): redirects disabled unless opted in through `follow_redirects`; relative default servers require explicit overrides (D-impl-relative-servers). Recorded decision (companion §8): an explicit `base_url` replaces ONLY the primary base; every additional base is overridden per key through `secondary_base_url`.
pub struct ClientBuilder {
    http: ::reqwest::ClientBuilder,
    base_url: Option<String>,
    limits: BodyLimits,
    default_server_url: String,
    default_server_variables: Vec<(String, String, Option<Vec<String>>)>,
    server_variables: ::std::collections::BTreeMap<String, String>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// Process-default transport: no redirects (§30.1) and process-default body limits (§33).
    #[must_use]
    pub fn new() -> Self {
        let default_server_url = "/".to_owned();
        let default_server_variables = Vec::new();
        Self {
            http: ::reqwest::Client::builder().redirect(::reqwest::redirect::Policy::none()),
            base_url: None,
            limits: BodyLimits::process_default(),
            default_server_url,
            default_server_variables,
            server_variables: ::std::collections::BTreeMap::new(),
        }
    }

    /// Overrides the resolved PRIMARY base URL only (recorded companion §8 decision); required before `build` when the primary default server is not absolute. Secondary bases are overridden through `secondary_base_url`.
    pub fn base_url(mut self, value: impl Into<String>) -> Self {
        self.base_url = Some(value.into());
        self
    }

    /// Replaces the process-default body limits (main spec §33).
    pub fn limits(mut self, limits: BodyLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Opts into redirect following (§30.1); generated decoding never buffers bodies to enable replay.
    pub fn follow_redirects(mut self, policy: ::reqwest::redirect::Policy) -> Self {
        self.http = self.http.redirect(policy);
        self
    }

    /// Builds the client (main spec §30.1, companion §8): every distinct base resolves independently — builder overrides or declared defaults, validated against their enums — and a non-absolute base without its own override is `ClientError::InvalidUrl` (D-impl-relative-servers).
    pub fn build(self) -> Result<Client, ClientError> {
        let base_url = match self.base_url {
            Some(explicit) => explicit,
            None => substitute_server_variables(
                &self.default_server_url,
                &self.default_server_variables,
                &self.server_variables,
            )?,
        };
        let trimmed = base_url.trim_end_matches('/');
        if !is_absolute_url(trimmed) {
            return Err(ClientError::InvalidUrl(format!(
                "base URL `{trimmed}` is not absolute; call `base_url` because no \
         absolute default server exists"
            )));
        }
        let http = self.http.build().map_err(ClientError::Transport)?;
        Ok(Client {
            http,
            base_url: trimmed.to_owned(),
            limits: self.limits,
        })
    }
}

/// Streaming payload for status 200 of `export_records` (main spec §19 Output A): owns the response; `into_ndjson_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct ExportRecords200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl ExportRecords200Stream {
    /// Consumes the wrapper into the incremental `ndjson` item stream.
    pub fn into_ndjson_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<Record, NdjsonDecodeError>> {
        classify_ndjson_premature_ends(decode_ndjson::<Record, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `export_records` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ExportRecordsResponse {
    /// HTTP 200 Ok.
    Ok200(ExportRecords200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Streaming payload for status 200 of `stream_events` (main spec §18 Output A): owns the response; `into_sse_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct StreamEvents200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl StreamEvents200Stream {
    /// Consumes the wrapper into the incremental `sse` item stream.
    pub fn into_sse_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<Event, SseDecodeError>> {
        classify_sse_premature_ends(decode_sse_json::<Event, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `stream_events` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum StreamEventsResponse {
    /// HTTP 200 Ok.
    Ok200(StreamEvents200Stream),
    /// HTTP 401 Unauthorized.
    Unauthorized401(ProblemDetails),
}

/// Streaming payload for status 200 of `stream_envelope_events` (main spec §18 Output A): owns the response; `into_sse_stream` decodes items incrementally, bounding each record by `max_stream_record_bytes` — never collecting the body.
#[derive(Debug)]
pub struct StreamEnvelopeEvents200Stream {
    pub response: ::reqwest::Response,
    pub limits: BodyLimits,
}

impl StreamEnvelopeEvents200Stream {
    /// Consumes the wrapper into the incremental `sse` item stream.
    pub fn into_sse_stream(
        self,
    ) -> impl ::futures_core::Stream<Item = Result<EventPayload, SseDecodeError>> {
        classify_sse_premature_ends(decode_sse_json::<EventPayload, _, _>(
            self.response.bytes_stream(),
            self.limits.max_stream_record_bytes,
        ))
    }
}

/// Documented outcomes for `stream_envelope_events` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum StreamEnvelopeEventsResponse {
    /// HTTP 200 Ok.
    Ok200(StreamEnvelopeEvents200Stream),
    /// HTTP 404 NotFound.
    NotFound404(ProblemDetails),
}

/// Streamed `jsonseq` request items for `push_metrics` (main spec §6/§18.1): boxed erased item stream handed to the generated method. Items encode lazily with a per-item bound of `max_stream_record_bytes` (§34.2 pre-send head check, then mid-send lazy encode).
pub type PushMetricsJsonSeqBody =
    ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = Metric> + ::std::marker::Send>>;

/// Documented outcomes for `push_metrics` (main spec §8/§13): exhaustive match required; deliberately not `#[non_exhaustive]` (§47).
#[derive(Debug)]
pub enum PushMetricsResponse {
    /// HTTP 202 Accepted.
    Accepted202(Ack),
}

impl Client {
    /// `GET` `/records/export`.
    /// Operation `exportRecords`.
    pub async fn export_records(&self) -> Result<ExportRecordsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/records/export");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "application/x-ndjson, application/problem+json",
            )
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => Ok(ExportRecordsResponse::Ok200(ExportRecords200Stream {
                response,
                limits: self.limits,
            })),
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(ExportRecordsResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/events`.
    /// Operation `streamEvents`.
    pub async fn stream_events(&self) -> Result<StreamEventsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/events");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "text/event-stream, application/problem+json",
            )
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => Ok(StreamEventsResponse::Ok200(StreamEvents200Stream {
                response,
                limits: self.limits,
            })),
            ::http::StatusCode::UNAUTHORIZED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(StreamEventsResponse::Unauthorized401(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `GET` `/envelope-events`.
    /// Operation `streamEnvelopeEvents`.
    pub async fn stream_envelope_events(
        &self,
    ) -> Result<StreamEnvelopeEventsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/envelope-events");
        // §30.1: redirects are off by default so documented 3xx statuses reach the exhaustive enum; opt-in following never buffers bodies for replay.
        let response = self
            .http
            .request(::http::Method::GET, &url)
            .header(
                ::http::header::ACCEPT,
                "text/event-stream, application/problem+json",
            )
            .send()
            .await?;
        match response.status() {
            ::http::StatusCode::OK => Ok(StreamEnvelopeEventsResponse::Ok200(
                StreamEnvelopeEvents200Stream {
                    response,
                    limits: self.limits,
                },
            )),
            ::http::StatusCode::NOT_FOUND => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/problem+json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.error_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: ProblemDetails = json_decode(&bytes, Some(content_type))?;
                Ok(StreamEnvelopeEventsResponse::NotFound404(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }

    /// `POST` `/metrics`.
    /// Operation `pushMetrics`.
    pub async fn push_metrics(
        &self,
        body: PushMetricsJsonSeqBody,
    ) -> Result<PushMetricsResponse, ClientError> {
        let mut url = self.base_url.clone();
        url.push_str("/metrics");
        let mut request = self.http.request(::http::Method::POST, &url);
        let encoder = stream_request_encoder(
            body,
            encode_jsonseq_item::<Metric>,
            self.limits.max_stream_record_bytes,
        )
        .await?;
        request = request
            .header(::http::header::CONTENT_TYPE, "application/json-seq")
            .body(::reqwest::Body::wrap_stream(encoder));
        request = request.header(::http::header::ACCEPT, "application/json");
        let response = request.send().await?;
        match response.status() {
            ::http::StatusCode::ACCEPTED => {
                let parsed = parse_response_content_type(&response)?;
                let Some(parsed) = parsed else {
                    return Err(ClientError::UnexpectedContentType {
                        expected: vec!["application/json".to_owned()],
                        actual: None,
                    });
                };
                let content_type = mime_of(&parsed)?;
                ensure_utf8_charset(&parsed)?;
                let limit = self.limits.structured_response_bytes;
                let bytes = collect_reqwest_limited(response, limit).await?;
                if bytes.is_empty() {
                    return Err(ClientError::Decode {
                        content_type: Some(content_type),
                        source: Box::new(EmptyJsonBody),
                    });
                }
                let value: Ack = json_decode(&bytes, Some(content_type))?;
                Ok(PushMetricsResponse::Accepted202(value))
            }
            other => Err(ClientError::UndocumentedStatus { status: other }),
        }
    }
}

/// One predicate shared by every framing classifier (main spec §40): delegates to the support crate's hyper-aware READ-side premature-body-end classification.
fn premature_body_end(error: &(dyn ::std::error::Error + Send + Sync + 'static)) -> bool {
    ::openapi_support::transport_classify::is_premature_body_end(error)
}

/// Remaps ONE decoded item of the `ndjson` stream (main spec §40): hyper READ-side premature body ends — the connection closed before the promised message completed — become `NdjsonDecodeError::Truncated`; every other transport failure keeps flowing through as `NdjsonDecodeError::Source` with its cause preserved.
fn remap_ndjson_item<T>(item: Result<T, NdjsonDecodeError>) -> Result<T, NdjsonDecodeError> {
    match item {
        Ok(value) => Ok(value),
        Err(NdjsonDecodeError::Source(source)) if premature_body_end(source.as_ref()) => {
            Err(NdjsonDecodeError::Truncated)
        }
        other => other,
    }
}

/// Wraps one `ndjson` decoder so transport failures are classified once at the adapter boundary (main spec §40 client-visible effect): truncation is never mistaken for clean end-of-stream or an opaque transport fault.
struct ClassifyNdjsonPrematureEnds<S> {
    inner: ::std::pin::Pin<Box<S>>,
}

impl<S, T> ::futures_core::Stream for ClassifyNdjsonPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, NdjsonDecodeError>>,
{
    type Item = Result<T, NdjsonDecodeError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .as_mut()
            .poll_next(cx)
            .map(|option| option.map(remap_ndjson_item::<T>))
    }
}

/// Classifies transport failures beneath one `ndjson` decoder (main spec §40).
fn classify_ndjson_premature_ends<S, T>(inner: S) -> ClassifyNdjsonPrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, NdjsonDecodeError>>,
{
    ClassifyNdjsonPrematureEnds {
        inner: Box::pin(inner),
    }
}

/// Remaps ONE decoded item of the `sse` stream (main spec §40): hyper READ-side premature body ends — the connection closed before the promised message completed — become `SseDecodeError::Truncated`; every other transport failure keeps flowing through as `SseDecodeError::Source` with its cause preserved.
fn remap_sse_item<T>(item: Result<T, SseDecodeError>) -> Result<T, SseDecodeError> {
    match item {
        Ok(value) => Ok(value),
        Err(SseDecodeError::Source(source)) if premature_body_end(source.as_ref()) => {
            Err(SseDecodeError::Truncated)
        }
        other => other,
    }
}

/// Wraps one `sse` decoder so transport failures are classified once at the adapter boundary (main spec §40 client-visible effect): truncation is never mistaken for clean end-of-stream or an opaque transport fault.
struct ClassifySsePrematureEnds<S> {
    inner: ::std::pin::Pin<Box<S>>,
}

impl<S, T> ::futures_core::Stream for ClassifySsePrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, SseDecodeError>>,
{
    type Item = Result<T, SseDecodeError>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        self.get_mut()
            .inner
            .as_mut()
            .poll_next(cx)
            .map(|option| option.map(remap_sse_item::<T>))
    }
}

/// Classifies transport failures beneath one `sse` decoder (main spec §40).
fn classify_sse_premature_ends<S, T>(inner: S) -> ClassifySsePrematureEnds<S>
where
    S: ::futures_core::Stream<Item = Result<T, SseDecodeError>>,
{
    ClassifySsePrematureEnds {
        inner: Box::pin(inner),
    }
}

/// Polls the next item of a boxed erased item stream (main spec §6 request-direction streams).
fn poll_items<T>(
    items: &mut ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
) -> impl ::std::future::Future<Output = Option<T>> + '_ {
    ::std::future::poll_fn(|cx| ::futures_core::Stream::poll_next(items.as_mut(), cx))
}

/// Lazy mid-send encoder over one streamed record request body (§34.2): items encode one at a time under `limit`.
struct RequestItemEncoder<T> {
    items: ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
    head: Option<::bytes::Bytes>,
    limit: usize,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
    finished: bool,
}

impl<T: serde::Serialize> ::futures_core::Stream for RequestItemEncoder<T> {
    type Item = Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>;

    fn poll_next(
        self: ::std::pin::Pin<&mut Self>,
        cx: &mut ::core::task::Context<'_>,
    ) -> ::core::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return ::core::task::Poll::Ready(None);
        }
        if let Some(head) = this.head.take() {
            return ::core::task::Poll::Ready(Some(Ok(head)));
        }
        match ::futures_core::Stream::poll_next(this.items.as_mut(), cx) {
            ::core::task::Poll::Pending => ::core::task::Poll::Pending,
            ::core::task::Poll::Ready(None) => {
                this.finished = true;
                ::core::task::Poll::Ready(None)
            }
            ::core::task::Poll::Ready(Some(item)) => match (this.encode)(&item, this.limit) {
                Ok(bytes) => ::core::task::Poll::Ready(Some(Ok(bytes))),
                Err(error) => {
                    this.finished = true;
                    ::core::task::Poll::Ready(Some(Err(error)))
                }
            },
        }
    }
}

/// Encodes the first item of one streamed record request body eagerly (§34.2: an oversized head returns `ClientError::BodyTooLarge` without sending anything) and hands the remaining items to the lazy mid-send encoder.
/// `encode` is the per-framing bounded item encoder; `limit` is `max_stream_record_bytes`.
#[allow(clippy::missing_errors_doc)]
async fn stream_request_encoder<T>(
    items: ::std::pin::Pin<Box<dyn ::futures_core::Stream<Item = T> + ::std::marker::Send>>,
    encode: fn(&T, usize) -> Result<::bytes::Bytes, ::openapi_support::encode::EncodeTooLarge>,
    limit: usize,
) -> Result<RequestItemEncoder<T>, ClientError> {
    let mut items = items;
    let mut head = None;
    if let Some(item) = poll_items(&mut items).await {
        match encode(&item, limit) {
            Ok(bytes) => head = Some(bytes),
            Err(_) => return Err(encode_overflow_error(limit)),
        }
    }
    Ok(RequestItemEncoder {
        items,
        head,
        limit,
        encode,
        finished: false,
    })
}

/// Reads and parses the response `Content-Type` (§28 steps 1–2): duplicate headers are ambiguous decode errors (§28.1), a missing header yields `None`, malformed values surface as `MalformedContentType`.
fn parse_response_content_type(
    response: &::reqwest::Response,
) -> Result<Option<ParsedMediaType>, ClientError> {
    let values: Vec<&::http::HeaderValue> = response
        .headers()
        .get_all(::http::header::CONTENT_TYPE)
        .iter()
        .collect();
    if values.len() > 1 {
        return Err(ClientError::Decode {
            content_type: None,
            source: Box::new(DuplicateContentType),
        });
    }
    let Some(raw) = values.first() else {
        return Ok(None);
    };
    let text = raw.to_str().map_err(|_| {
        ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)
    })?;
    Ok(Some(::openapi_support::mediatype::parse_content_type(
        text,
    )?))
}

/// duplicate Content-Type headers are an ambiguous message (§28.1); generated code never picks one arbitrarily
#[derive(Debug)]
struct DuplicateContentType;

impl std::fmt::Display for DuplicateContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("duplicate Content-Type headers on one message")
    }
}

impl std::error::Error for DuplicateContentType {}

/// Builds the `mime::Mime` carried by [`ClientError`] fields.
#[allow(clippy::missing_errors_doc)]
fn mime_of(parsed: &ParsedMediaType) -> Result<::mime::Mime, ClientError> {
    let subtype = match &parsed.suffix {
        Some(suffix) => format!("{}+{}", parsed.subtype, suffix),
        None => parsed.subtype.clone(),
    };
    let text = format!("{}/{}", parsed.ty, subtype);
    text.parse().map_err(|_| {
        ClientError::MalformedContentType(::openapi_support::mediatype::MalformedContentType)
    })
}

/// §28.4 charset policy (D-impl-charset-rejection): textual media decode as UTF-8; any other declared charset is a decode error instead of replacement-character corruption.
#[allow(clippy::missing_errors_doc)]
fn ensure_utf8_charset(parsed: &ParsedMediaType) -> Result<(), ClientError> {
    if let Some((_, value)) = parsed.parameters.iter().find(|(name, _)| name == "charset") {
        let lowered = value.to_ascii_lowercase();
        if lowered != "utf-8" && lowered != "utf8" {
            return Err(ClientError::Decode {
                content_type: None,
                source: Box::new(UnsupportedCharset(value.clone())),
            });
        }
    }
    Ok(())
}

/// declared charset is outside the UTF-8 family (§28.4); generated clients surface this as `ClientError::Decode` (D-impl-charset-rejection)
#[derive(Debug)]
struct UnsupportedCharset(String);

impl std::fmt::Display for UnsupportedCharset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "charset `{}` is outside the UTF-8 family", self.0)
    }
}

impl std::error::Error for UnsupportedCharset {}

/// a documented JSON status arrived with an empty body; empty input is never decoded as a default value (§28.3)
#[derive(Debug)]
struct EmptyJsonBody;

impl std::fmt::Display for EmptyJsonBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("documented JSON status arrived with an empty body")
    }
}

impl std::error::Error for EmptyJsonBody {}

/// Maps bounded JSON decode failures onto [`ClientError::Decode`] (§36); `content_type` is carried for diagnostics.
fn json_decode<T>(bytes: &[u8], content_type: Option<::mime::Mime>) -> Result<T, ClientError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| ClientError::Decode {
        content_type,
        source: Box::new(error),
    })
}

/// Client-side encode overflow (§34.2): returned BEFORE anything is sent.
#[must_use]
fn encode_overflow_error(limit: usize) -> ClientError {
    ClientError::BodyTooLarge {
        direction: BodyLimitDirection::Encode,
        limit,
    }
}

/// Substitutes server variables with builder overrides or declared defaults, validating enum membership at build time (companion §8).
#[allow(clippy::missing_errors_doc)]
fn substitute_server_variables(
    url: &str,
    variables: &[(String, String, Option<Vec<String>>)],
    overrides: &::std::collections::BTreeMap<String, String>,
) -> Result<String, ClientError> {
    let mut resolved = url.to_owned();
    for (name, default, allowed) in variables {
        let value = if let Some(value) = overrides.get(name) {
            value.clone()
        } else {
            default.clone()
        };
        if let Some(allowed) = allowed {
            if !allowed.contains(&value) {
                return Err(ClientError::InvalidUrl(format!(
                    "server variable `{name}` value `{value}` is not one of {allowed:?}"
                )));
            }
        }
        let placeholder = format!("{{{name}}}");
        if !resolved.contains(&placeholder) {
            return Err(ClientError::InvalidUrl(format!(
                "server variable `{name}` has no placeholder in `{url}`"
            )));
        }
        resolved = resolved.replace(&placeholder, &value);
    }
    if resolved.contains('{') || resolved.contains('}') {
        return Err(ClientError::InvalidUrl(format!(
            "unresolved server variable placeholder in `{resolved}`"
        )));
    }
    Ok(resolved)
}

/// Absolute-URL gate for the resolved base (D-impl-relative-servers): scheme + `://` + non-empty remainder.
#[must_use]
fn is_absolute_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
        && !rest.is_empty()
}

//! §49 negative proof (compile half): a multipart binary part is a streaming
//! `reqwest::Body`, never a buffered `Vec<u8>`.
//!
//! The generated `UploadDocumentRequest` for fixture 11 declares part `file`
//! as `reqwest::Body` (main spec §17 Output A); assigning the §49-forbidden
//! `file: Vec<u8>` shape to that field must fail to typecheck.

use openapi_conformance::fixtures::fixture_11_multipart::client::UploadDocumentRequest;
use openapi_conformance::fixtures::fixture_11_multipart::models::DocumentMetadata;

fn main() {
    let _request = UploadDocumentRequest {
        metadata: DocumentMetadata {
            title: String::new(),
            pages: openapi_support::optional::OptionalField::Absent,
        },
        tags: Vec::new(),
        file: Vec::<u8>::new(),
        file_name: None,
        file_content_type: None,
    };
}

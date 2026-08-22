//! Version-agnostic internal IR produced by the loader (companion §2).
//!
//! Submodules split schema-level types ([`schema`]) from document-level
//! types ([`document`]); everything is re-exported here for convenience.

pub mod document;
pub mod schema;

pub use document::{
    classify_media_type, ContentEntryIr, HeaderSpecIr, HttpMethod, IrDocument, MediaClass,
    OpenApiVersion, OperationIr, ParameterIr, ParameterLocation, ParameterStyle, PathEntry,
    RangeClass, RequestBodyIr, ResponseEntryIr, ResponseStatusKey, ServerIr, ServerVariable,
};
pub use schema::{
    AdditionalPropertiesPolicy, DiscriminatorIr, EnumValues, Indirection, NumericValidation,
    PropertyIr, SchemaArena, SchemaEdge, SchemaId, SchemaKind, SchemaNode, SchemaRefName,
    UnsupportedReason, ValidationMeta,
};

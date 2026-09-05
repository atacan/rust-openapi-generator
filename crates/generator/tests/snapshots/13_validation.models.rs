//! Shared schema models generated from the OpenAPI document (main spec §2.6): one
//! module reused by both client and server operation codecs.
//!
//! Every named `components/schemas` entry appears below in document declaration
//! order; nested anonymous objects and enumerations become generated definitions
//! emitted before their parents (`<Parent><FieldPascal>` plus numeric collision
//! suffixes, companion §10).
//!
//! Property presence/nullability follows companion §2.1 cell-for-cell; bucket-2
//! validation constraints ride as documentation and as emitted `validate_request`
//! methods (companion §9; D-impl-runtime-validation-timing Phase 2 half). This file
//! is generated deterministically byte-for-byte (main spec §50 test 39); do not edit
//! by hand.
use std::collections::BTreeMap;

use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredSlug {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): pattern `^[a-z][a-z0-9-]*$`; minLength >= 2.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub slug: OptionalField<Slug>,
}

impl RegisteredSlug {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        if let OptionalField::Present(value) = &self.slug {
            ::openapi_support::validation::validate_string(
                value,
                &::openapi_support::validation::StringConstraints {
                    pattern: Some("^[a-z][a-z0-9-]*$"),
                    min_length: Some(2),
                    max_length: None,
                },
            )
            .map_err(|error| error.at_field("slug"))?;
        }
        Ok(())
    }
}

/// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): pattern `^[a-z][a-z0-9-]*$`; minLength >= 2.
pub type Slug = String;

/// Server-side request validation (companion §9) for the constrained scalar alias `Slug`: bucket-2 constraints enforced on server requests; client encoding stays lenient.
pub fn validate_slug_request(
    value: &str,
) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
    ::openapi_support::validation::validate_string(
        value,
        &::openapi_support::validation::StringConstraints {
            pattern: Some("^[a-z][a-z0-9-]*$"),
            min_length: Some(2),
            max_length: None,
        },
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteReceipt {
    pub stored: bool,
}

/// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minProperties >= 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TicketMeta {
    #[serde(flatten)]
    pub additional: BTreeMap<String, String>,
}

impl TicketMeta {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        let mut property_count = 0_usize;
        property_count += self.additional.len();
        ::openapi_support::validation::validate_object_props(property_count, Some(1), None)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): pattern `^[A-Z]{3}-[0-9]{4}$`; minLength >= 8.
    pub code: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `date-time`.
    pub when: String,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): format `email`.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub contact: OptionalField<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): exclusiveMinimum > 0; exclusiveMaximum < 13; format `int32`.
    pub seats: i32,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minimum >= 0; multipleOf 0.5.
    pub price: f64,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minItems >= 2; maxItems <= 4; uniqueItems.
    pub tags: Vec<String>,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minProperties >= 1.
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub meta: OptionalField<TicketMeta>,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub nested: OptionalField<LevelA>,
}

impl Ticket {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_string(
            &self.code,
            &::openapi_support::validation::StringConstraints {
                pattern: Some("^[A-Z]{3}-[0-9]{4}$"),
                min_length: Some(8),
                max_length: None,
            },
        )
        .map_err(|error| error.at_field("code"))?;
        ::openapi_support::validation::validate_format_string(&self.when, "date-time")
            .map_err(|error| error.at_field("when"))?;
        if let OptionalField::Present(value) = &self.contact {
            ::openapi_support::validation::validate_format_string(value, "email")
                .map_err(|error| error.at_field("contact"))?;
        }
        ::openapi_support::validation::validate_number(
            (self.seats) as f64,
            Some((0.0, true)),
            Some((13.0, true)),
            None,
        )
        .map_err(|error| error.at_field("seats"))?;
        ::openapi_support::validation::validate_number(
            self.price,
            Some((0.0, false)),
            None,
            Some(0.5),
        )
        .map_err(|error| error.at_field("price"))?;
        ::openapi_support::validation::validate_array_len(
            self.tags.len(),
            &::openapi_support::validation::ArrayConstraints {
                min_items: Some(2),
                max_items: Some(4),
            },
        )
        .map_err(|error| error.at_field("tags"))?;
        ::openapi_support::validation::require_unique_strings(self.tags.iter())
            .map_err(|error| error.at_field("tags"))?;
        for item in self.tags.iter() {
            ::openapi_support::validation::validate_string(
                item,
                &::openapi_support::validation::StringConstraints {
                    pattern: None,
                    min_length: Some(1),
                    max_length: None,
                },
            )
            .map_err(|error| error.at_field("tags[*]"))?;
        }
        if let OptionalField::Present(value) = &self.meta {
            ::openapi_support::validation::located("meta", value.validate_request())?;
        }
        if let OptionalField::Present(value) = &self.nested {
            ::openapi_support::validation::located("nested", value.validate_request())?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelA {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): pattern `^[a-z]+$`.
    pub label: String,
    pub next: LevelB,
}

impl LevelA {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_string(
            &self.label,
            &::openapi_support::validation::StringConstraints {
                pattern: Some("^[a-z]+$"),
                min_length: None,
                max_length: None,
            },
        )
        .map_err(|error| error.at_field("label"))?;
        ::openapi_support::validation::located("next", self.next.validate_request())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelB {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): exclusiveMinimum > 0.
    pub weight: i64,
}

impl LevelB {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_number(
            (self.weight) as f64,
            Some((0.0, true)),
            None,
            None,
        )
        .map_err(|error| error.at_field("weight"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedbackForm {
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): exclusiveMinimum > 0; exclusiveMaximum < 6.
    pub rating: i64,
    /// Constraints (enforced by generated routers on server requests, companion §9; lenient on client decode): minLength >= 4; maxLength <= 140.
    pub comment: String,
}

impl FeedbackForm {
    /// Server-side request validation (companion §9): structural checks stay in Serde decode; these enforce the D-§2
    /// bucket-2 constraints. Client decoding stays lenient.
    pub fn validate_request(
        &self,
    ) -> ::std::result::Result<(), ::openapi_support::validation::Violation> {
        ::openapi_support::validation::validate_number(
            (self.rating) as f64,
            Some((0.0, true)),
            Some((6.0, true)),
            None,
        )
        .map_err(|error| error.at_field("rating"))?;
        ::openapi_support::validation::validate_string(
            &self.comment,
            &::openapi_support::validation::StringConstraints {
                pattern: None,
                min_length: Some(4),
                max_length: Some(140),
            },
        )
        .map_err(|error| error.at_field("comment"))?;
        Ok(())
    }
}

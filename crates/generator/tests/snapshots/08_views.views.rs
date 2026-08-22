/// Directional view types generated from the OpenAPI document (companion §5):
/// `Write` views carry the client-request-encode / server-request-decode wire
/// shape with `readOnly` properties omitted; `Read` views carry the server-
/// response-encode / client-response-decode wire shape with `writeOnly`
/// properties treated as absent.
///
/// Requiredness is directional (companion §5): required `writeOnly` fields are
/// required only in `Write`, required `readOnly` fields only in `Read`, and
/// directionless required fields stay required in both. Every surviving field
/// applies the companion §2.1 presence/nullability matrix identically to
/// `super::models`.
///
/// Conversions are intentionally asymmetric (companion §5): projections
/// `From<&SharedModel> for *View` always exist; reconstructions
/// `From<&*View> for SharedModel` exist only when every view-omitted field is
/// optional in the shared model, so no value is ever fabricated. Models
/// without `readOnly`/`writeOnly` properties receive no view types.
///
/// This file is generated deterministically byte-for-byte (main spec §50 test
/// 39); do not edit by hand.
use openapi_support::optional::OptionalField;
use serde::{Deserialize, Serialize};

/// Directional write view of `Account` (companion §5): client request encode and server request decode wire shape.
/// Every property survives in this direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountWrite {
    pub id: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub note: OptionalField<String>,
}

/// Projects the shared model into the AccountWrite (companion §5, client request encode and server request decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Every shared property survives in this direction.
impl From<&Account> for AccountWrite {
    fn from(value: &Account) -> Self {
        Self {
            id: value.id.clone(),
            password: value.password.clone(),
            note: value.note.clone(),
        }
    }
}

/// Losslessly reconstructs the shared model from the view (companion §5): every field omitted from the view is optional in the shared model, so the conversion invents no values.
/// Nothing is omitted in this direction.
impl From<&AccountWrite> for Account {
    fn from(value: &AccountWrite) -> Self {
        Self {
            id: value.id.clone(),
            password: value.password.clone(),
            note: value.note.clone(),
        }
    }
}

/// Directional read view of `Account` (companion §5): server response encode and client response decode wire shape.
/// writeOnly properties are omitted here: password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountRead {
    pub id: String,
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub note: OptionalField<String>,
}

/// Projects the shared model into the AccountRead (companion §5, server response encode and client response decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Omitted here (writeOnly): password.
impl From<&Account> for AccountRead {
    fn from(value: &Account) -> Self {
        Self {
            id: value.id.clone(),
            note: value.note.clone(),
        }
    }
}

/// Directional write view of `AuditEntry` (companion §5): client request encode and server request decode wire shape.
/// readOnly properties are omitted here: created_at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntryWrite {
    #[serde(rename = "draftNote")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub draft_note: OptionalField<String>,
    #[serde(default)]
    pub metadata: Option<String>,
}

/// Projects the shared model into the AuditEntryWrite (companion §5, client request encode and server request decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Omitted here (readOnly): created_at.
impl From<&AuditEntry> for AuditEntryWrite {
    fn from(value: &AuditEntry) -> Self {
        Self {
            draft_note: value.draft_note.clone(),
            metadata: value.metadata.clone(),
        }
    }
}

/// Directional read view of `AuditEntry` (companion §5): server response encode and client response decode wire shape.
/// writeOnly properties are omitted here: draft_note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntryRead {
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(default)]
    pub metadata: Option<String>,
}

/// Projects the shared model into the AuditEntryRead (companion §5, server response encode and client response decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Omitted here (writeOnly): draft_note.
impl From<&AuditEntry> for AuditEntryRead {
    fn from(value: &AuditEntry) -> Self {
        Self {
            created_at: value.created_at.clone(),
            metadata: value.metadata.clone(),
        }
    }
}

/// Losslessly reconstructs the shared model from the view (companion §5): every field omitted from the view is optional in the shared model, so the conversion invents no values.
/// Missing fields default: draft_note to absent.
impl From<&AuditEntryRead> for AuditEntry {
    fn from(value: &AuditEntryRead) -> Self {
        Self {
            created_at: value.created_at.clone(),
            draft_note: openapi_support::optional::OptionalField::Absent,
            metadata: value.metadata.clone(),
        }
    }
}

/// Directional write view of `SyncedRecord` (companion §5): client request encode and server request decode wire shape.
/// readOnly properties are omitted here: id, reviewed_by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedRecordWrite {
    pub label: String,
    #[serde(rename = "secretToken")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub secret_token: OptionalField<String>,
}

/// Projects the shared model into the SyncedRecordWrite (companion §5, client request encode and server request decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Omitted here (readOnly): id, reviewed_by.
impl From<&SyncedRecord> for SyncedRecordWrite {
    fn from(value: &SyncedRecord) -> Self {
        Self {
            label: value.label.clone(),
            secret_token: value.secret_token.clone(),
        }
    }
}

/// Directional read view of `SyncedRecord` (companion §5): server response encode and client response decode wire shape.
/// writeOnly properties are omitted here: secret_token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncedRecordRead {
    pub id: String,
    pub label: String,
    #[serde(rename = "reviewedBy")]
    #[serde(default, skip_serializing_if = "openapi_support::optional::is_absent")]
    pub reviewed_by: OptionalField<String>,
}

/// Projects the shared model into the SyncedRecordRead (companion §5, server response encode and client response decode): kept fields clone or copy from the borrowed model; this projection always exists.
/// Omitted here (writeOnly): secret_token.
impl From<&SyncedRecord> for SyncedRecordRead {
    fn from(value: &SyncedRecord) -> Self {
        Self {
            id: value.id.clone(),
            label: value.label.clone(),
            reviewed_by: value.reviewed_by.clone(),
        }
    }
}

/// Losslessly reconstructs the shared model from the view (companion §5): every field omitted from the view is optional in the shared model, so the conversion invents no values.
/// Missing fields default: secret_token to absent.
impl From<&SyncedRecordRead> for SyncedRecord {
    fn from(value: &SyncedRecordRead) -> Self {
        Self {
            id: value.id.clone(),
            label: value.label.clone(),
            secret_token: openapi_support::optional::OptionalField::Absent,
            reviewed_by: value.reviewed_by.clone(),
        }
    }
}

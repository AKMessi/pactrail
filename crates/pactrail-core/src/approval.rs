use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{ApprovalId, Capability, RunId};

/// Versioned scope that an approval decision is permitted to authorize.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ApprovalBinding {
    pub run_id: RunId,
    pub capability: Capability,
    /// Canonical, non-secret resource representation.
    pub resource: String,
    /// Digest binding the request to the exact actor and arguments.
    pub actor_fingerprint: String,
    /// Stable process/tool boundary name.
    pub backend_kind: String,
    /// Immutable runtime or image identity when one exists.
    pub backend_identity: Option<String>,
    /// Digest of the complete enforcement profile.
    pub profile_digest: String,
}

/// User-facing details for a scoped approval request.
///
/// Values are deliberately strings so frontends can render the same bounded,
/// provenance-labelled request without gaining authority from presentation data.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ApprovalRequest {
    pub binding: ApprovalBinding,
    pub reason: String,
    #[serde(default)]
    pub presentation: BTreeMap<String, String>,
}

/// Decision made for one exact approval binding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowRun,
    Deny,
}

/// Durable approval decision recorded in the run's integrity chain.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ApprovalRecord {
    pub schema_version: u32,
    pub id: ApprovalId,
    pub binding: ApprovalBinding,
    pub decision: ApprovalDecision,
    #[schemars(with = "String")]
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[schemars(with = "Option<String>")]
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

impl ApprovalRecord {
    pub const SCHEMA_VERSION: u32 = 1;

    /// Creates a non-expiring run-local decision for an exact binding.
    #[must_use]
    pub fn new(binding: ApprovalBinding, decision: ApprovalDecision) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            id: ApprovalId::new(),
            binding,
            decision,
            created_at: OffsetDateTime::now_utc(),
            expires_at: None,
        }
    }

    /// Whether this known-version decision is still temporally valid.
    #[must_use]
    pub fn is_valid_at(&self, now: OffsetDateTime) -> bool {
        self.schema_version == Self::SCHEMA_VERSION
            && self.expires_at.is_none_or(|expiry| now < expiry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_records_are_versioned_and_bound() {
        let binding = ApprovalBinding {
            run_id: RunId::new(),
            capability: Capability::ProcessSpawn,
            resource: r#"{"program":"cargo"}"#.to_owned(),
            actor_fingerprint: "actor".to_owned(),
            backend_kind: "oci_restricted".to_owned(),
            backend_identity: Some("sha256:image".to_owned()),
            profile_digest: "profile".to_owned(),
        };
        let record = ApprovalRecord::new(binding.clone(), ApprovalDecision::AllowOnce);
        assert_eq!(record.binding, binding);
        assert!(record.is_valid_at(record.created_at));
    }
}

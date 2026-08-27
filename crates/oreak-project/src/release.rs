use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{
    ArtifactDigest, ArtifactId, CandidateId, CertificationId, PluginId, PromotionId,
    ReleaseChannelId, RevisionId, TimestampMs, UserId,
};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReleaseModelError {
    #[error("opaque certification evidence cannot be empty")]
    EmptyCertificationEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpaqueCertification {
    id: CertificationId,
    evidence: String,
}

#[derive(Deserialize)]
struct SerializedOpaqueCertification {
    id: CertificationId,
    evidence: String,
}

impl<'de> Deserialize<'de> for OpaqueCertification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let certification = SerializedOpaqueCertification::deserialize(deserializer)?;
        Self::new(certification.id, certification.evidence).map_err(D::Error::custom)
    }
}

impl OpaqueCertification {
    pub fn new(
        id: CertificationId,
        evidence: impl Into<String>,
    ) -> Result<Self, ReleaseModelError> {
        let evidence = evidence.into();
        if evidence.is_empty() {
            return Err(ReleaseModelError::EmptyCertificationEvidence);
        }
        Ok(Self { id, evidence })
    }

    #[must_use]
    pub const fn id(&self) -> &CertificationId {
        &self.id
    }

    #[must_use]
    pub fn evidence(&self) -> &str {
        &self.evidence
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecCertifications {
    pub project_codec: Option<OpaqueCertification>,
    pub plugin_codecs: BTreeMap<PluginId, OpaqueCertification>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseGate {
    require_project_codec: bool,
    required_plugin_codecs: BTreeSet<PluginId>,
}

impl ReleaseGate {
    #[must_use]
    pub const fn new(
        require_project_codec: bool,
        required_plugin_codecs: BTreeSet<PluginId>,
    ) -> Self {
        Self {
            require_project_codec,
            required_plugin_codecs,
        }
    }

    #[must_use]
    pub const fn require_project_codec(&self) -> bool {
        self.require_project_codec
    }

    #[must_use]
    pub const fn required_plugin_codecs(&self) -> &BTreeSet<PluginId> {
        &self.required_plugin_codecs
    }

    #[must_use]
    pub fn missing_certifications(
        &self,
        certifications: &CodecCertifications,
    ) -> MissingCertifications {
        MissingCertifications {
            project_codec: self.require_project_codec && certifications.project_codec.is_none(),
            plugin_codecs: self
                .required_plugin_codecs
                .iter()
                .filter(|plugin| !certifications.plugin_codecs.contains_key(*plugin))
                .cloned()
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissingCertifications {
    pub project_codec: bool,
    pub plugin_codecs: BTreeSet<PluginId>,
}

impl MissingCertifications {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.project_codec && self.plugin_codecs.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInput {
    pub id: ArtifactId,
    pub digest: ArtifactDigest,
    pub candidate_id: CandidateId,
    pub certifications: CodecCertifications,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    id: ArtifactId,
    digest: ArtifactDigest,
    candidate_id: CandidateId,
    revision_id: RevisionId,
    produced_by: UserId,
    produced_at: TimestampMs,
    certifications: CodecCertifications,
}

impl Artifact {
    pub(crate) fn new(
        input: ArtifactInput,
        revision_id: RevisionId,
        produced_by: UserId,
        produced_at: TimestampMs,
    ) -> Self {
        Self {
            id: input.id,
            digest: input.digest,
            candidate_id: input.candidate_id,
            revision_id,
            produced_by,
            produced_at,
            certifications: input.certifications,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &ArtifactId {
        &self.id
    }

    #[must_use]
    pub const fn digest(&self) -> &ArtifactDigest {
        &self.digest
    }

    #[must_use]
    pub const fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    #[must_use]
    pub const fn revision_id(&self) -> &RevisionId {
        &self.revision_id
    }

    #[must_use]
    pub const fn produced_by(&self) -> &UserId {
        &self.produced_by
    }

    #[must_use]
    pub const fn produced_at(&self) -> TimestampMs {
        self.produced_at
    }

    #[must_use]
    pub const fn certifications(&self) -> &CodecCertifications {
        &self.certifications
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseChannel {
    id: ReleaseChannelId,
    name: String,
    created_by: UserId,
    created_at: TimestampMs,
}

impl ReleaseChannel {
    pub(crate) fn new(
        id: ReleaseChannelId,
        name: String,
        created_by: UserId,
        created_at: TimestampMs,
    ) -> Self {
        Self {
            id,
            name,
            created_by,
            created_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &ReleaseChannelId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn created_by(&self) -> &UserId {
        &self.created_by
    }

    #[must_use]
    pub const fn created_at(&self) -> TimestampMs {
        self.created_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Promotion {
    id: PromotionId,
    channel_id: ReleaseChannelId,
    artifact_id: ArtifactId,
    previous_artifact_id: Option<ArtifactId>,
    promoted_by: UserId,
    promoted_at: TimestampMs,
}

impl Promotion {
    pub(crate) fn new(
        id: PromotionId,
        channel_id: ReleaseChannelId,
        artifact_id: ArtifactId,
        previous_artifact_id: Option<ArtifactId>,
        promoted_by: UserId,
        promoted_at: TimestampMs,
    ) -> Self {
        Self {
            id,
            channel_id,
            artifact_id,
            previous_artifact_id,
            promoted_by,
            promoted_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &PromotionId {
        &self.id
    }

    #[must_use]
    pub const fn channel_id(&self) -> &ReleaseChannelId {
        &self.channel_id
    }

    #[must_use]
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    #[must_use]
    pub const fn previous_artifact_id(&self) -> Option<&ArtifactId> {
        self.previous_artifact_id.as_ref()
    }

    #[must_use]
    pub const fn promoted_by(&self) -> &UserId {
        &self.promoted_by
    }

    #[must_use]
    pub const fn promoted_at(&self) -> TimestampMs {
        self.promoted_at
    }
}

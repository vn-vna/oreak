use serde::{Deserialize, Serialize};

use crate::{
    CodecCertificationState, CustomEntityInstance, LegacyCodecCertification,
    LegacyCodecDeclaration, PluginId, PluginManifest, ProjectPluginPin,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseGateIssueKind {
    EntityPluginNotPinned,
    ExactManifestUnavailable,
    CodecNotDeclared,
    CertificationMissing,
    CertificationNotApproved,
    CertifiedCodecVersionMismatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseGateIssue {
    pub kind: ReleaseGateIssueKind,
    pub entity_id: String,
    pub plugin_id: PluginId,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseGateEvaluation {
    pub allowed: bool,
    pub issues: Vec<ReleaseGateIssue>,
}

impl ReleaseGateEvaluation {
    pub fn is_allowed(&self) -> bool {
        self.allowed
    }
}

/// Evaluates the LevelData release gate for runtime custom entities.
///
/// A certification only applies when its complete pin (ID, plugin version, API
/// version, and source hash) equals the project pin and loaded manifest.
pub fn evaluate_release_gate(
    project_pins: &[ProjectPluginPin],
    manifests: &[PluginManifest],
    entities: &[CustomEntityInstance],
    certifications: &[LegacyCodecCertification],
) -> ReleaseGateEvaluation {
    let mut issues = Vec::new();

    for entity in entities.iter().filter(|entity| entity.runtime) {
        let exact_pin = project_pins.iter().find(|pin| **pin == entity.plugin);
        if exact_pin.is_none() {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::EntityPluginNotPinned,
                "runtime entity does not use an exact project plugin pin",
            );
            continue;
        }

        let Some(manifest) = manifests
            .iter()
            .find(|manifest| entity.plugin.matches_manifest(manifest))
        else {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::ExactManifestUnavailable,
                "manifest for the exact plugin pin is unavailable",
            );
            continue;
        };

        let LegacyCodecDeclaration::Provided { codec_version, .. } =
            &manifest.legacy_level_data_codec
        else {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::CodecNotDeclared,
                "the exact plugin manifest does not declare a LevelData codec",
            );
            continue;
        };

        let exact_certifications: Vec<_> = certifications
            .iter()
            .filter(|certification| certification.plugin == entity.plugin)
            .collect();
        if exact_certifications.is_empty() {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::CertificationMissing,
                "the exact plugin pin has no LevelData codec certification",
            );
            continue;
        }

        if exact_certifications.iter().any(|certification| {
            matches!(
                &certification.state,
                CodecCertificationState::Certified {
                    codec_version: certified_version,
                    ..
                } if certified_version == codec_version
            )
        }) {
            continue;
        }
        if exact_certifications.iter().any(|certification| {
            matches!(
                certification.state,
                CodecCertificationState::Certified { .. }
            )
        }) {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::CertifiedCodecVersionMismatch,
                "the certificate does not cover the manifest codec version",
            );
        } else {
            push_issue(
                &mut issues,
                entity,
                ReleaseGateIssueKind::CertificationNotApproved,
                "the exact plugin pin's LevelData codec is not certified",
            );
        }
    }

    ReleaseGateEvaluation {
        allowed: issues.is_empty(),
        issues,
    }
}

fn push_issue(
    issues: &mut Vec<ReleaseGateIssue>,
    entity: &CustomEntityInstance,
    kind: ReleaseGateIssueKind,
    message: &str,
) {
    issues.push(ReleaseGateIssue {
        kind,
        entity_id: entity.entity_id.clone(),
        plugin_id: entity.plugin.plugin_id.clone(),
        message: message.to_owned(),
    });
}

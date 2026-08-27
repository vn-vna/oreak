use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::RoleId;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    ViewProject,
    EditTimeline,
    CreateReviewCandidate,
    CommentOnReview,
    SubmitReview,
    ApproveReview,
    ManageArtifacts,
    ManageReleaseChannels,
    PromoteArtifacts,
    ManageProjectSettings,
    ManageMembers,
    ManageRoles,
    ManageTheme,
    ManageKpiPolicies,
    ManageReleaseGate,
    ViewAudit,
    Custom(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoleTemplate {
    Owner,
    Admin,
    Editor,
    Viewer,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProjectRole {
    Template(RoleTemplate),
    Custom(RoleId),
}

impl ProjectRole {
    #[must_use]
    pub const fn owner() -> Self {
        Self::Template(RoleTemplate::Owner)
    }

    #[must_use]
    pub const fn admin() -> Self {
        Self::Template(RoleTemplate::Admin)
    }

    #[must_use]
    pub const fn editor() -> Self {
        Self::Template(RoleTemplate::Editor)
    }

    #[must_use]
    pub const fn viewer() -> Self {
        Self::Template(RoleTemplate::Viewer)
    }

    #[must_use]
    pub const fn is_owner(&self) -> bool {
        matches!(self, Self::Template(RoleTemplate::Owner))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRole {
    id: RoleId,
    name: String,
    capabilities: BTreeSet<Capability>,
}

impl CustomRole {
    #[must_use]
    pub fn new(id: RoleId, name: impl Into<String>, capabilities: BTreeSet<Capability>) -> Self {
        Self {
            id,
            name: name.into(),
            capabilities,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &RoleId {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn capabilities(&self) -> &BTreeSet<Capability> {
        &self.capabilities
    }
}

#[must_use]
pub fn owner_capabilities() -> BTreeSet<Capability> {
    [
        Capability::ViewProject,
        Capability::EditTimeline,
        Capability::CreateReviewCandidate,
        Capability::CommentOnReview,
        Capability::SubmitReview,
        Capability::ApproveReview,
        Capability::ManageArtifacts,
        Capability::ManageReleaseChannels,
        Capability::PromoteArtifacts,
        Capability::ManageProjectSettings,
        Capability::ManageMembers,
        Capability::ManageRoles,
        Capability::ManageTheme,
        Capability::ManageKpiPolicies,
        Capability::ManageReleaseGate,
        Capability::ViewAudit,
    ]
    .into_iter()
    .collect()
}

#[must_use]
pub fn admin_capabilities() -> BTreeSet<Capability> {
    [
        Capability::ViewProject,
        Capability::CommentOnReview,
        Capability::SubmitReview,
        Capability::ApproveReview,
        Capability::ManageArtifacts,
        Capability::ManageReleaseChannels,
        Capability::PromoteArtifacts,
        Capability::ManageProjectSettings,
        Capability::ManageMembers,
        Capability::ManageRoles,
        Capability::ManageTheme,
        Capability::ManageKpiPolicies,
        Capability::ManageReleaseGate,
        Capability::ViewAudit,
    ]
    .into_iter()
    .collect()
}

#[must_use]
pub fn editor_capabilities() -> BTreeSet<Capability> {
    [
        Capability::ViewProject,
        Capability::EditTimeline,
        Capability::CreateReviewCandidate,
        Capability::CommentOnReview,
        Capability::SubmitReview,
    ]
    .into_iter()
    .collect()
}

#[must_use]
pub fn viewer_capabilities() -> BTreeSet<Capability> {
    [Capability::ViewProject, Capability::CommentOnReview]
        .into_iter()
        .collect()
}

pub(crate) fn template_capabilities(template: RoleTemplate) -> BTreeSet<Capability> {
    match template {
        RoleTemplate::Owner => owner_capabilities(),
        RoleTemplate::Admin => admin_capabilities(),
        RoleTemplate::Editor => editor_capabilities(),
        RoleTemplate::Viewer => viewer_capabilities(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn administrative_role_does_not_imply_edit_access() {
        let admin = admin_capabilities();
        assert!(admin.contains(&Capability::ManageMembers));
        assert!(!admin.contains(&Capability::EditTimeline));
    }
}

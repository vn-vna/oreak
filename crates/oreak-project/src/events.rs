use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, AuditEventId, CandidateId, CommentId, ContributionEventId, FormulaVersion,
    InvitationId, LevelId, ProjectId, ProjectRole, PromotionId, ReleaseChannelId, ReviewId, RoleId,
    ThemeId, TimestampMs, UserId, WorkspaceId, WorkspaceRole,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditContext {
    pub event_id: AuditEventId,
    pub actor: UserId,
    pub occurred_at: TimestampMs,
}

impl AuditContext {
    #[must_use]
    pub const fn new(event_id: AuditEventId, actor: UserId, occurred_at: TimestampMs) -> Self {
        Self {
            event_id,
            actor,
            occurred_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContributionContext {
    pub event_id: ContributionEventId,
    pub actor: UserId,
    pub occurred_at: TimestampMs,
}

impl ContributionContext {
    #[must_use]
    pub const fn new(
        event_id: ContributionEventId,
        actor: UserId,
        occurred_at: TimestampMs,
    ) -> Self {
        Self {
            event_id,
            actor,
            occurred_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    WorkspaceCreated,
    WorkspaceInvitationCreated {
        invitation_id: InvitationId,
        invitee: UserId,
        role: WorkspaceRole,
    },
    WorkspaceInvitationRevoked {
        invitation_id: InvitationId,
    },
    WorkspaceInvitationAccepted {
        invitation_id: InvitationId,
    },
    WorkspaceMemberRoleChanged {
        member: UserId,
        role: WorkspaceRole,
    },
    WorkspaceMemberRemoved {
        member: UserId,
    },
    ProjectRegistered {
        project_id: ProjectId,
    },
    ProjectCreated,
    ProjectInvitationCreated {
        invitation_id: InvitationId,
        invitee: UserId,
        roles: Vec<ProjectRole>,
    },
    ProjectInvitationRevoked {
        invitation_id: InvitationId,
    },
    ProjectInvitationAccepted {
        invitation_id: InvitationId,
    },
    ProjectMemberRolesChanged {
        member: UserId,
        roles: Vec<ProjectRole>,
    },
    ProjectMemberRemoved {
        member: UserId,
    },
    CustomRoleCreated {
        role_id: RoleId,
    },
    ApprovalPolicyChanged,
    ReleaseGateChanged,
    ProjectThemeChanged {
        theme_id: ThemeId,
    },
    UserThemeOverrideChanged {
        theme_id: Option<ThemeId>,
    },
    KpiPolicyAdded {
        version: FormulaVersion,
    },
    LevelCreated {
        level_id: LevelId,
    },
    LevelConfigurationChanged {
        level_id: LevelId,
    },
    ArtifactCreated {
        artifact_id: ArtifactId,
        candidate_id: CandidateId,
    },
    ReleaseChannelCreated {
        channel_id: ReleaseChannelId,
    },
    ArtifactPromoted {
        promotion_id: PromotionId,
        channel_id: ReleaseChannelId,
        artifact_id: ArtifactId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    id: AuditEventId,
    actor: UserId,
    occurred_at: TimestampMs,
    workspace_id: WorkspaceId,
    project_id: Option<ProjectId>,
    action: AuditAction,
}

impl AuditEvent {
    pub(crate) fn new(
        context: AuditContext,
        workspace_id: WorkspaceId,
        project_id: Option<ProjectId>,
        action: AuditAction,
    ) -> Self {
        Self {
            id: context.event_id,
            actor: context.actor,
            occurred_at: context.occurred_at,
            workspace_id,
            project_id,
            action,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &AuditEventId {
        &self.id
    }

    #[must_use]
    pub const fn actor(&self) -> &UserId {
        &self.actor
    }

    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMs {
        self.occurred_at
    }

    #[must_use]
    pub const fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    #[must_use]
    pub const fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }

    #[must_use]
    pub const fn action(&self) -> &AuditAction {
        &self.action
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContributionKind {
    TimelineRevision,
    ReviewCandidate,
    ReviewComment,
    ReviewDecision,
    CandidateStateChanged,
    ArtifactProduced,
    ReleasePromotion,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContributionChannel {
    Timeline,
    Review,
    Artifact,
    Release(ReleaseChannelId),
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WorkflowState {
    Active,
    InReview,
    ChangesRequested,
    Approved,
    Rejected,
    Withdrawn,
    Released,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContributionSubject {
    Revision(LevelId),
    Candidate(CandidateId),
    Comment(CommentId),
    Review(ReviewId),
    Artifact(ArtifactId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionEvent {
    id: ContributionEventId,
    project_id: ProjectId,
    contributor: UserId,
    occurred_at: TimestampMs,
    kind: ContributionKind,
    channel: ContributionChannel,
    workflow_state: WorkflowState,
    subject: ContributionSubject,
}

impl ContributionEvent {
    pub(crate) fn new(
        context: ContributionContext,
        project_id: ProjectId,
        kind: ContributionKind,
        channel: ContributionChannel,
        workflow_state: WorkflowState,
        subject: ContributionSubject,
    ) -> Self {
        Self {
            id: context.event_id,
            project_id,
            contributor: context.actor,
            occurred_at: context.occurred_at,
            kind,
            channel,
            workflow_state,
            subject,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &ContributionEventId {
        &self.id
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub const fn contributor(&self) -> &UserId {
        &self.contributor
    }

    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMs {
        self.occurred_at
    }

    #[must_use]
    pub const fn kind(&self) -> &ContributionKind {
        &self.kind
    }

    #[must_use]
    pub const fn channel(&self) -> &ContributionChannel {
        &self.channel
    }

    #[must_use]
    pub const fn workflow_state(&self) -> &WorkflowState {
        &self.workflow_state
    }

    #[must_use]
    pub const fn subject(&self) -> &ContributionSubject {
        &self.subject
    }
}

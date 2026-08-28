//! Pure governance domain logic for Oreak projects.
//!
//! IDs and timestamps are supplied by callers. The crate performs no I/O and
//! uses no platform clock or randomness, so the same state machines run on
//! native and WebAssembly targets.

#![forbid(unsafe_code)]

mod events;
mod ids;
mod kpi;
mod permissions;
mod project;
mod release;
mod review;
mod workspace;

pub use events::{
    AuditAction, AuditContext, AuditEvent, ContributionChannel, ContributionContext,
    ContributionEvent, ContributionKind, ContributionSubject, WorkflowState,
};
pub use ids::{
    ArtifactDigest, ArtifactId, AuditEventId, CandidateId, CertificationId, CommentId,
    ContributionEventId, Day, EntityId, FormulaVersion, GuideId, IdError, InvitationId, LevelId,
    PluginId, ProjectId, PromotionId, ReleaseChannelId, ReviewId, RevisionId, RoleId, ThemeId,
    TimestampMs, UserId, WorkspaceId,
};
pub use kpi::{DailyActivity, KpiError, KpiFormula, KpiPolicy, MAX_ACTIVITY_DAYS};
pub use permissions::{
    Capability, CustomRole, ProjectRole, RoleTemplate, admin_capabilities, editor_capabilities,
    owner_capabilities, viewer_capabilities,
};
pub use project::{
    LevelConfiguration, LevelTimeline, Project, ProjectError, ProjectInvitation,
    ProjectInvitationState, ProjectMember, TimelineRevision,
};
pub use release::{
    Artifact, ArtifactInput, CodecCertifications, MissingCertifications, OpaqueCertification,
    Promotion, ReleaseChannel, ReleaseGate, ReleaseModelError,
};
pub use review::{
    ApprovalPolicy, ApprovalStatus, BlindPixelRegion, GridCell, GridRegion, GuideEdge,
    GuideEdgeAnchor, ReviewAnchor, ReviewCandidate, ReviewComment, ReviewDecision,
    ReviewModelError, ReviewRecord, ReviewState,
};
pub use workspace::{
    Workspace, WorkspaceError, WorkspaceInvitation, WorkspaceInvitationState, WorkspaceKind,
    WorkspaceMember, WorkspaceRole,
};

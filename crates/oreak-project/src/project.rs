use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{
    ApprovalPolicy, ApprovalStatus, Artifact, ArtifactId, ArtifactInput, AuditAction, AuditContext,
    AuditEvent, AuditEventId, CandidateId, Capability, CommentId, ContributionChannel,
    ContributionContext, ContributionEvent, ContributionEventId, ContributionKind,
    ContributionSubject, CustomRole, DailyActivity, Day, FormulaVersion, InvitationId, KpiError,
    KpiFormula, KpiPolicy, LevelId, MAX_ACTIVITY_DAYS, MissingCertifications, ProjectId,
    ProjectRole, Promotion, PromotionId, ReleaseChannel, ReleaseChannelId, ReleaseGate,
    ReviewAnchor, ReviewCandidate, ReviewComment, ReviewDecision, ReviewId, ReviewRecord,
    ReviewState, RevisionId, RoleId, ThemeId, TimestampMs, UserId, WorkflowState, WorkspaceId,
    permissions::template_capabilities,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMember {
    user_id: UserId,
    roles: BTreeSet<ProjectRole>,
    joined_at: TimestampMs,
}

impl ProjectMember {
    #[must_use]
    pub const fn user_id(&self) -> &UserId {
        &self.user_id
    }

    #[must_use]
    pub const fn roles(&self) -> &BTreeSet<ProjectRole> {
        &self.roles
    }

    #[must_use]
    pub const fn joined_at(&self) -> TimestampMs {
        self.joined_at
    }

    #[must_use]
    pub fn is_owner(&self) -> bool {
        self.roles.contains(&ProjectRole::owner())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectInvitationState {
    Pending,
    Accepted,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInvitation {
    id: InvitationId,
    invitee: UserId,
    roles: BTreeSet<ProjectRole>,
    invited_by: UserId,
    invited_at: TimestampMs,
    expires_at: Option<TimestampMs>,
    state: ProjectInvitationState,
}

impl ProjectInvitation {
    #[must_use]
    pub const fn id(&self) -> &InvitationId {
        &self.id
    }

    #[must_use]
    pub const fn invitee(&self) -> &UserId {
        &self.invitee
    }

    #[must_use]
    pub const fn roles(&self) -> &BTreeSet<ProjectRole> {
        &self.roles
    }

    #[must_use]
    pub const fn invited_by(&self) -> &UserId {
        &self.invited_by
    }

    #[must_use]
    pub const fn invited_at(&self) -> TimestampMs {
        self.invited_at
    }

    #[must_use]
    pub const fn expires_at(&self) -> Option<TimestampMs> {
        self.expires_at
    }

    #[must_use]
    pub const fn state(&self) -> ProjectInvitationState {
        self.state
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineRevision {
    id: RevisionId,
    sequence: u64,
    parent_revision_id: Option<RevisionId>,
    author: UserId,
    occurred_at: TimestampMs,
}

impl TimelineRevision {
    #[must_use]
    pub const fn id(&self) -> &RevisionId {
        &self.id
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn parent_revision_id(&self) -> Option<&RevisionId> {
        self.parent_revision_id.as_ref()
    }

    #[must_use]
    pub const fn author(&self) -> &UserId {
        &self.author
    }

    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMs {
        self.occurred_at
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelConfiguration {
    name: String,
    duration_seconds: f64,
}

impl Eq for LevelConfiguration {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SerializedLevelConfiguration {
    name: String,
    duration_seconds: f64,
}

impl<'de> Deserialize<'de> for LevelConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let configuration = SerializedLevelConfiguration::deserialize(deserializer)?;
        Self::new(configuration.name, configuration.duration_seconds).map_err(D::Error::custom)
    }
}

impl LevelConfiguration {
    pub fn new(name: impl Into<String>, duration_seconds: f64) -> Result<Self, ProjectError> {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() {
            return Err(ProjectError::EmptyName);
        }
        if !duration_seconds.is_finite() || duration_seconds < 0.0 {
            return Err(ProjectError::InvalidLevelDuration);
        }
        Ok(Self {
            name: name.to_owned(),
            duration_seconds: if duration_seconds == 0.0 {
                0.0
            } else {
                duration_seconds
            },
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn duration_seconds(&self) -> f64 {
        self.duration_seconds
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelTimeline {
    level_id: LevelId,
    configuration: LevelConfiguration,
    revisions: Vec<TimelineRevision>,
}

impl LevelTimeline {
    #[must_use]
    pub const fn level_id(&self) -> &LevelId {
        &self.level_id
    }

    #[must_use]
    pub const fn configuration(&self) -> &LevelConfiguration {
        &self.configuration
    }

    #[must_use]
    pub fn revisions(&self) -> &[TimelineRevision] {
        &self.revisions
    }

    #[must_use]
    pub fn head(&self) -> Option<&TimelineRevision> {
        self.revisions.last()
    }

    fn append(
        &mut self,
        id: RevisionId,
        author: UserId,
        occurred_at: TimestampMs,
    ) -> Result<(), ProjectError> {
        let sequence = u64::try_from(self.revisions.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(ProjectError::TimelineExhausted)?;
        let parent_revision_id = self.head().map(|revision| revision.id.clone());
        self.revisions.push(TimelineRevision {
            id,
            sequence,
            parent_revision_id,
            author,
            occurred_at,
        });
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    id: ProjectId,
    workspace_id: WorkspaceId,
    members: BTreeMap<UserId, ProjectMember>,
    invitations: BTreeMap<InvitationId, ProjectInvitation>,
    custom_roles: BTreeMap<RoleId, CustomRole>,
    timelines: BTreeMap<LevelId, LevelTimeline>,
    revision_ids: BTreeSet<RevisionId>,
    candidates: BTreeMap<CandidateId, ReviewCandidate>,
    review_ids: BTreeSet<ReviewId>,
    comment_ids: BTreeSet<CommentId>,
    artifacts: BTreeMap<ArtifactId, Artifact>,
    release_channels: BTreeMap<ReleaseChannelId, ReleaseChannel>,
    promotions: Vec<Promotion>,
    promotion_ids: BTreeSet<PromotionId>,
    default_theme: ThemeId,
    theme_overrides: BTreeMap<UserId, ThemeId>,
    approval_policy: ApprovalPolicy,
    release_gate: ReleaseGate,
    kpi_policies: BTreeMap<Day, KpiPolicy>,
    kpi_versions: BTreeSet<FormulaVersion>,
    audit_events: Vec<AuditEvent>,
    audit_ids: BTreeSet<AuditEventId>,
    contribution_events: Vec<ContributionEvent>,
    contribution_ids: BTreeSet<ContributionEventId>,
}

impl Project {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: ProjectId,
        workspace_id: WorkspaceId,
        creator: UserId,
        default_theme: ThemeId,
        approval_policy: ApprovalPolicy,
        release_gate: ReleaseGate,
        context: AuditContext,
    ) -> Result<Self, ProjectError> {
        if creator != context.actor {
            return Err(ProjectError::ActorMismatch);
        }
        let member = ProjectMember {
            user_id: creator.clone(),
            roles: [ProjectRole::owner()].into_iter().collect(),
            joined_at: context.occurred_at,
        };
        let event = AuditEvent::new(
            context,
            workspace_id.clone(),
            Some(id.clone()),
            AuditAction::ProjectCreated,
        );
        Ok(Self {
            id,
            workspace_id,
            members: [(creator, member)].into_iter().collect(),
            invitations: BTreeMap::new(),
            custom_roles: BTreeMap::new(),
            timelines: BTreeMap::new(),
            revision_ids: BTreeSet::new(),
            candidates: BTreeMap::new(),
            review_ids: BTreeSet::new(),
            comment_ids: BTreeSet::new(),
            artifacts: BTreeMap::new(),
            release_channels: BTreeMap::new(),
            promotions: Vec::new(),
            promotion_ids: BTreeSet::new(),
            default_theme,
            theme_overrides: BTreeMap::new(),
            approval_policy,
            release_gate,
            kpi_policies: BTreeMap::new(),
            kpi_versions: BTreeSet::new(),
            audit_ids: [event.id().clone()].into_iter().collect(),
            audit_events: vec![event],
            contribution_events: Vec::new(),
            contribution_ids: BTreeSet::new(),
        })
    }

    #[must_use]
    pub const fn id(&self) -> &ProjectId {
        &self.id
    }

    #[must_use]
    pub const fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    #[must_use]
    pub const fn members(&self) -> &BTreeMap<UserId, ProjectMember> {
        &self.members
    }

    #[must_use]
    pub const fn invitations(&self) -> &BTreeMap<InvitationId, ProjectInvitation> {
        &self.invitations
    }

    #[must_use]
    pub const fn custom_roles(&self) -> &BTreeMap<RoleId, CustomRole> {
        &self.custom_roles
    }

    #[must_use]
    pub const fn timelines(&self) -> &BTreeMap<LevelId, LevelTimeline> {
        &self.timelines
    }

    #[must_use]
    pub const fn candidates(&self) -> &BTreeMap<CandidateId, ReviewCandidate> {
        &self.candidates
    }

    #[must_use]
    pub const fn artifacts(&self) -> &BTreeMap<ArtifactId, Artifact> {
        &self.artifacts
    }

    #[must_use]
    pub const fn release_channels(&self) -> &BTreeMap<ReleaseChannelId, ReleaseChannel> {
        &self.release_channels
    }

    #[must_use]
    pub fn promotions(&self) -> &[Promotion] {
        &self.promotions
    }

    #[must_use]
    pub const fn default_theme(&self) -> &ThemeId {
        &self.default_theme
    }

    #[must_use]
    pub const fn approval_policy(&self) -> &ApprovalPolicy {
        &self.approval_policy
    }

    #[must_use]
    pub const fn release_gate(&self) -> &ReleaseGate {
        &self.release_gate
    }

    #[must_use]
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.audit_events
    }

    #[must_use]
    pub fn contribution_events(&self) -> &[ContributionEvent] {
        &self.contribution_events
    }

    #[must_use]
    pub fn capabilities_for(&self, user: &UserId) -> BTreeSet<Capability> {
        let Some(member) = self.members.get(user) else {
            return BTreeSet::new();
        };
        member
            .roles
            .iter()
            .flat_map(|role| match role {
                ProjectRole::Template(template) => template_capabilities(*template),
                ProjectRole::Custom(role_id) => self
                    .custom_roles
                    .get(role_id)
                    .map_or_else(BTreeSet::new, |role| role.capabilities().clone()),
            })
            .collect()
    }

    #[must_use]
    pub fn has_capability(&self, user: &UserId, capability: &Capability) -> bool {
        self.capabilities_for(user).contains(capability)
    }

    pub fn create_custom_role(
        &mut self,
        role: CustomRole,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageRoles)?;
        if role.name().trim().is_empty() {
            return Err(ProjectError::EmptyName);
        }
        if self.custom_roles.contains_key(role.id()) {
            return Err(ProjectError::DuplicateRole(role.id().clone()));
        }
        let role_id = role.id().clone();
        self.custom_roles.insert(role_id.clone(), role);
        self.append_audit(context, AuditAction::CustomRoleCreated { role_id });
        Ok(())
    }

    pub fn invite_member(
        &mut self,
        invitation_id: InvitationId,
        invitee: UserId,
        roles: BTreeSet<ProjectRole>,
        expires_at: Option<TimestampMs>,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageMembers)?;
        self.validate_roles(&roles)?;
        if roles.iter().any(ProjectRole::is_owner) {
            self.require_project_owner(&context.actor)?;
        }
        if self.members.contains_key(&invitee) {
            return Err(ProjectError::AlreadyMember(invitee));
        }
        if self.invitations.contains_key(&invitation_id) {
            return Err(ProjectError::DuplicateInvitation(invitation_id));
        }
        if expires_at.is_some_and(|expiry| expiry.as_i64() <= context.occurred_at.as_i64()) {
            return Err(ProjectError::InvalidExpiration);
        }
        if self.invitations.values().any(|invitation| {
            invitation.invitee == invitee
                && invitation.state == ProjectInvitationState::Pending
                && invitation
                    .expires_at
                    .is_none_or(|expiry| expiry.as_i64() > context.occurred_at.as_i64())
        }) {
            return Err(ProjectError::PendingInvitationExists(invitee));
        }
        let invitation = ProjectInvitation {
            id: invitation_id.clone(),
            invitee: invitee.clone(),
            roles: roles.clone(),
            invited_by: context.actor.clone(),
            invited_at: context.occurred_at,
            expires_at,
            state: ProjectInvitationState::Pending,
        };
        self.invitations.insert(invitation_id.clone(), invitation);
        self.append_audit(
            context,
            AuditAction::ProjectInvitationCreated {
                invitation_id,
                invitee,
                roles: roles.into_iter().collect(),
            },
        );
        Ok(())
    }

    pub fn accept_invitation(
        &mut self,
        invitation_id: &InvitationId,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        let invitation = self
            .invitations
            .get(invitation_id)
            .ok_or_else(|| ProjectError::InvitationNotFound(invitation_id.clone()))?;
        if invitation.invitee != context.actor {
            return Err(ProjectError::InvitationNotForActor);
        }
        if invitation.state != ProjectInvitationState::Pending {
            return Err(ProjectError::InvitationNotPending);
        }
        if invitation
            .expires_at
            .is_some_and(|expiry| context.occurred_at.as_i64() >= expiry.as_i64())
        {
            return Err(ProjectError::InvitationExpired);
        }
        if self.members.contains_key(&context.actor) {
            return Err(ProjectError::AlreadyMember(context.actor));
        }
        let member = ProjectMember {
            user_id: context.actor.clone(),
            roles: invitation.roles.clone(),
            joined_at: context.occurred_at,
        };
        self.members.insert(context.actor.clone(), member);
        self.invitations
            .get_mut(invitation_id)
            .expect("invitation was checked")
            .state = ProjectInvitationState::Accepted;
        self.append_audit(
            context,
            AuditAction::ProjectInvitationAccepted {
                invitation_id: invitation_id.clone(),
            },
        );
        Ok(())
    }

    pub fn revoke_invitation(
        &mut self,
        invitation_id: &InvitationId,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageMembers)?;
        let invitation = self
            .invitations
            .get(invitation_id)
            .ok_or_else(|| ProjectError::InvitationNotFound(invitation_id.clone()))?;
        if invitation.state != ProjectInvitationState::Pending {
            return Err(ProjectError::InvitationNotPending);
        }
        if invitation.roles.iter().any(ProjectRole::is_owner) {
            self.require_project_owner(&context.actor)?;
        }
        self.invitations
            .get_mut(invitation_id)
            .expect("invitation was checked")
            .state = ProjectInvitationState::Revoked;
        self.append_audit(
            context,
            AuditAction::ProjectInvitationRevoked {
                invitation_id: invitation_id.clone(),
            },
        );
        Ok(())
    }

    pub fn set_member_roles(
        &mut self,
        member_id: &UserId,
        roles: BTreeSet<ProjectRole>,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageMembers)?;
        self.validate_roles(&roles)?;
        let current = self
            .members
            .get(member_id)
            .ok_or_else(|| ProjectError::MemberNotFound(member_id.clone()))?;
        if current.roles == roles {
            return Err(ProjectError::NoChange);
        }
        let was_owner = current.is_owner();
        let will_be_owner = roles.iter().any(ProjectRole::is_owner);
        if was_owner || will_be_owner {
            self.require_project_owner(&context.actor)?;
        }
        if was_owner && !will_be_owner && self.owner_count() == 1 {
            return Err(ProjectError::LastOwner);
        }
        self.members
            .get_mut(member_id)
            .expect("member was checked")
            .roles = roles.clone();
        self.append_audit(
            context,
            AuditAction::ProjectMemberRolesChanged {
                member: member_id.clone(),
                roles: roles.into_iter().collect(),
            },
        );
        Ok(())
    }

    pub fn remove_member(
        &mut self,
        member_id: &UserId,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        let member = self
            .members
            .get(member_id)
            .ok_or_else(|| ProjectError::MemberNotFound(member_id.clone()))?;
        if context.actor != *member_id {
            self.require(&context.actor, Capability::ManageMembers)?;
            if member.is_owner() {
                self.require_project_owner(&context.actor)?;
            }
        }
        if member.is_owner() && self.owner_count() == 1 {
            return Err(ProjectError::LastOwner);
        }
        self.members.remove(member_id);
        self.theme_overrides.remove(member_id);
        self.append_audit(
            context,
            AuditAction::ProjectMemberRemoved {
                member: member_id.clone(),
            },
        );
        Ok(())
    }

    pub fn set_approval_policy(
        &mut self,
        policy: ApprovalPolicy,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageProjectSettings)?;
        if self.approval_policy == policy {
            return Err(ProjectError::NoChange);
        }
        self.approval_policy = policy;
        self.append_audit(context, AuditAction::ApprovalPolicyChanged);
        Ok(())
    }

    pub fn set_release_gate(
        &mut self,
        release_gate: ReleaseGate,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageReleaseGate)?;
        if self.release_gate == release_gate {
            return Err(ProjectError::NoChange);
        }
        self.release_gate = release_gate;
        self.append_audit(context, AuditAction::ReleaseGateChanged);
        Ok(())
    }

    pub fn set_default_theme(
        &mut self,
        theme_id: ThemeId,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageTheme)?;
        if self.default_theme == theme_id {
            return Err(ProjectError::NoChange);
        }
        self.default_theme = theme_id.clone();
        self.append_audit(context, AuditAction::ProjectThemeChanged { theme_id });
        Ok(())
    }

    pub fn set_my_theme_override(
        &mut self,
        theme_id: Option<ThemeId>,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ViewProject)?;
        if self.theme_overrides.get(&context.actor) == theme_id.as_ref() {
            return Err(ProjectError::NoChange);
        }
        match &theme_id {
            Some(theme) => {
                self.theme_overrides
                    .insert(context.actor.clone(), theme.clone());
            }
            None => {
                self.theme_overrides.remove(&context.actor);
            }
        }
        self.append_audit(context, AuditAction::UserThemeOverrideChanged { theme_id });
        Ok(())
    }

    #[must_use]
    pub fn effective_theme(&self, user: &UserId) -> &ThemeId {
        self.theme_overrides
            .get(user)
            .unwrap_or(&self.default_theme)
    }

    pub fn create_level(
        &mut self,
        level_id: LevelId,
        configuration: LevelConfiguration,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::EditTimeline)?;
        if self.timelines.contains_key(&level_id) {
            return Err(ProjectError::DuplicateLevel(level_id));
        }
        self.timelines.insert(
            level_id.clone(),
            LevelTimeline {
                level_id: level_id.clone(),
                configuration,
                revisions: Vec::new(),
            },
        );
        self.append_audit(context, AuditAction::LevelCreated { level_id });
        Ok(())
    }

    pub fn set_level_configuration(
        &mut self,
        level_id: &LevelId,
        configuration: LevelConfiguration,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::EditTimeline)?;
        let timeline = self
            .timelines
            .get_mut(level_id)
            .ok_or_else(|| ProjectError::LevelNotFound(level_id.clone()))?;
        if timeline.configuration == configuration {
            return Err(ProjectError::NoChange);
        }
        timeline.configuration = configuration;
        self.append_audit(
            context,
            AuditAction::LevelConfigurationChanged {
                level_id: level_id.clone(),
            },
        );
        Ok(())
    }

    pub fn append_timeline_revision(
        &mut self,
        level_id: &LevelId,
        revision_id: RevisionId,
        context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_contribution(&context.event_id)?;
        self.require(&context.actor, Capability::EditTimeline)?;
        if self.revision_ids.contains(&revision_id) {
            return Err(ProjectError::DuplicateRevision(revision_id));
        }
        let timeline = self
            .timelines
            .get_mut(level_id)
            .ok_or_else(|| ProjectError::LevelNotFound(level_id.clone()))?;
        timeline.append(
            revision_id.clone(),
            context.actor.clone(),
            context.occurred_at,
        )?;
        self.revision_ids.insert(revision_id);
        self.append_contribution(
            context,
            ContributionKind::TimelineRevision,
            ContributionChannel::Timeline,
            WorkflowState::Active,
            ContributionSubject::Revision(level_id.clone()),
        );
        Ok(())
    }

    pub fn create_review_candidate(
        &mut self,
        candidate_id: CandidateId,
        level_id: &LevelId,
        revision_id: &RevisionId,
        context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_contribution(&context.event_id)?;
        self.require(&context.actor, Capability::CreateReviewCandidate)?;
        if self.candidates.contains_key(&candidate_id) {
            return Err(ProjectError::DuplicateCandidate(candidate_id));
        }
        let timeline = self
            .timelines
            .get(level_id)
            .ok_or_else(|| ProjectError::LevelNotFound(level_id.clone()))?;
        if !timeline
            .revisions
            .iter()
            .any(|revision| revision.id == *revision_id)
        {
            return Err(ProjectError::RevisionNotFound(revision_id.clone()));
        }
        let candidate = ReviewCandidate::new(
            candidate_id.clone(),
            level_id.clone(),
            revision_id.clone(),
            context.actor.clone(),
            context.occurred_at,
            self.approval_policy.clone(),
        );
        self.candidates.insert(candidate_id.clone(), candidate);
        self.append_contribution(
            context,
            ContributionKind::ReviewCandidate,
            ContributionChannel::Review,
            WorkflowState::InReview,
            ContributionSubject::Candidate(candidate_id),
        );
        Ok(())
    }

    pub fn add_review_comment(
        &mut self,
        candidate_id: &CandidateId,
        comment_id: CommentId,
        body: impl Into<String>,
        anchor: ReviewAnchor,
        context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_contribution(&context.event_id)?;
        self.require(&context.actor, Capability::CommentOnReview)?;
        if self.comment_ids.contains(&comment_id) {
            return Err(ProjectError::DuplicateComment(comment_id));
        }
        let body = body.into();
        if body.trim().is_empty() {
            return Err(ProjectError::EmptyComment);
        }
        let candidate = self
            .candidates
            .get(candidate_id)
            .ok_or_else(|| ProjectError::CandidateNotFound(candidate_id.clone()))?;
        if candidate.state().is_terminal() {
            return Err(ProjectError::CandidateTerminal(candidate.state()));
        }
        if anchor.revision_id() != candidate.revision_id() {
            return Err(ProjectError::AnchorRevisionMismatch);
        }
        let comment = ReviewComment::new(
            comment_id.clone(),
            context.actor.clone(),
            body,
            anchor,
            context.occurred_at,
        );
        self.candidates
            .get_mut(candidate_id)
            .expect("candidate was checked")
            .push_comment(comment);
        self.comment_ids.insert(comment_id.clone());
        self.append_contribution(
            context,
            ContributionKind::ReviewComment,
            ContributionChannel::Review,
            WorkflowState::InReview,
            ContributionSubject::Comment(comment_id),
        );
        Ok(())
    }

    pub fn submit_review(
        &mut self,
        candidate_id: &CandidateId,
        review_id: ReviewId,
        decision: ReviewDecision,
        context: ContributionContext,
    ) -> Result<ReviewState, ProjectError> {
        self.ensure_fresh_contribution(&context.event_id)?;
        self.require(&context.actor, Capability::SubmitReview)?;
        if self.review_ids.contains(&review_id) {
            return Err(ProjectError::DuplicateReview(review_id));
        }
        let candidate = self
            .candidates
            .get(candidate_id)
            .ok_or_else(|| ProjectError::CandidateNotFound(candidate_id.clone()))?;
        if candidate.state().is_terminal() {
            return Err(ProjectError::CandidateTerminal(candidate.state()));
        }
        let capabilities = self.capabilities_for(&context.actor);
        if decision == ReviewDecision::Approve {
            if !capabilities.contains(&Capability::ApproveReview) {
                return Err(ProjectError::PermissionDenied {
                    actor: context.actor,
                    capability: Capability::ApproveReview,
                });
            }
            if !candidate.approval_policy().allow_self_approval()
                && candidate.created_by() == &context.actor
            {
                return Err(ProjectError::SelfApprovalForbidden);
            }
            let missing = candidate
                .approval_policy()
                .required_capabilities()
                .difference(&capabilities)
                .cloned()
                .collect::<BTreeSet<_>>();
            if !missing.is_empty() {
                return Err(ProjectError::MissingApprovalCapabilities(missing));
            }
        }
        let record = ReviewRecord::new(
            review_id.clone(),
            context.actor.clone(),
            decision,
            context.occurred_at,
            capabilities,
        );
        self.candidates
            .get_mut(candidate_id)
            .expect("candidate was checked")
            .push_review(record);
        self.review_ids.insert(review_id.clone());
        let state = self.recompute_candidate_state(candidate_id);
        self.append_contribution(
            context,
            ContributionKind::ReviewDecision,
            ContributionChannel::Review,
            match state {
                ReviewState::Approved => WorkflowState::Approved,
                ReviewState::ChangesRequested => WorkflowState::ChangesRequested,
                ReviewState::Rejected => WorkflowState::Rejected,
                _ => WorkflowState::InReview,
            },
            ContributionSubject::Review(review_id),
        );
        Ok(state)
    }

    pub fn withdraw_candidate(
        &mut self,
        candidate_id: &CandidateId,
        context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_contribution(&context.event_id)?;
        let candidate = self
            .candidates
            .get(candidate_id)
            .ok_or_else(|| ProjectError::CandidateNotFound(candidate_id.clone()))?;
        if candidate.state().is_terminal() {
            return Err(ProjectError::CandidateTerminal(candidate.state()));
        }
        if candidate.created_by() != &context.actor
            && !self.has_capability(&context.actor, &Capability::ManageProjectSettings)
        {
            return Err(ProjectError::PermissionDenied {
                actor: context.actor,
                capability: Capability::ManageProjectSettings,
            });
        }
        self.candidates
            .get_mut(candidate_id)
            .expect("candidate was checked")
            .set_state(ReviewState::Withdrawn);
        self.append_contribution(
            context,
            ContributionKind::CandidateStateChanged,
            ContributionChannel::Review,
            WorkflowState::Withdrawn,
            ContributionSubject::Candidate(candidate_id.clone()),
        );
        Ok(())
    }

    #[must_use]
    pub fn approval_status(&self, candidate_id: &CandidateId) -> Option<ApprovalStatus> {
        let candidate = self.candidates.get(candidate_id)?;
        Some(Self::approval_status_for(candidate))
    }

    pub fn create_artifact(
        &mut self,
        input: ArtifactInput,
        audit_context: AuditContext,
        contribution_context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_contexts_match(&audit_context, &contribution_context)?;
        self.ensure_fresh_audit(&audit_context.event_id)?;
        self.ensure_fresh_contribution(&contribution_context.event_id)?;
        self.require(&audit_context.actor, Capability::ManageArtifacts)?;
        if self.artifacts.contains_key(&input.id) {
            return Err(ProjectError::DuplicateArtifact(input.id));
        }
        let candidate = self
            .candidates
            .get(&input.candidate_id)
            .ok_or_else(|| ProjectError::CandidateNotFound(input.candidate_id.clone()))?;
        if candidate.state() != ReviewState::Approved {
            return Err(ProjectError::CandidateNotApproved);
        }
        let missing = self
            .release_gate
            .missing_certifications(&input.certifications);
        if !missing.is_empty() {
            return Err(ProjectError::MissingCertifications(missing));
        }
        let artifact_id = input.id.clone();
        let candidate_id = input.candidate_id.clone();
        let artifact = Artifact::new(
            input,
            candidate.revision_id().clone(),
            audit_context.actor.clone(),
            audit_context.occurred_at,
        );
        self.artifacts.insert(artifact_id.clone(), artifact);
        self.append_audit(
            audit_context,
            AuditAction::ArtifactCreated {
                artifact_id: artifact_id.clone(),
                candidate_id,
            },
        );
        self.append_contribution(
            contribution_context,
            ContributionKind::ArtifactProduced,
            ContributionChannel::Artifact,
            WorkflowState::Approved,
            ContributionSubject::Artifact(artifact_id),
        );
        Ok(())
    }

    pub fn create_release_channel(
        &mut self,
        channel_id: ReleaseChannelId,
        name: impl Into<String>,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageReleaseChannels)?;
        if self.release_channels.contains_key(&channel_id) {
            return Err(ProjectError::DuplicateReleaseChannel(channel_id));
        }
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ProjectError::EmptyName);
        }
        let channel = ReleaseChannel::new(
            channel_id.clone(),
            name,
            context.actor.clone(),
            context.occurred_at,
        );
        self.release_channels.insert(channel_id.clone(), channel);
        self.append_audit(context, AuditAction::ReleaseChannelCreated { channel_id });
        Ok(())
    }

    pub fn promote_artifact(
        &mut self,
        promotion_id: PromotionId,
        channel_id: &ReleaseChannelId,
        artifact_id: &ArtifactId,
        audit_context: AuditContext,
        contribution_context: ContributionContext,
    ) -> Result<(), ProjectError> {
        self.ensure_contexts_match(&audit_context, &contribution_context)?;
        self.ensure_fresh_audit(&audit_context.event_id)?;
        self.ensure_fresh_contribution(&contribution_context.event_id)?;
        self.require(&audit_context.actor, Capability::PromoteArtifacts)?;
        if self.promotion_ids.contains(&promotion_id) {
            return Err(ProjectError::DuplicatePromotion(promotion_id));
        }
        if !self.release_channels.contains_key(channel_id) {
            return Err(ProjectError::ReleaseChannelNotFound(channel_id.clone()));
        }
        let artifact = self
            .artifacts
            .get(artifact_id)
            .ok_or_else(|| ProjectError::ArtifactNotFound(artifact_id.clone()))?;
        let missing = self
            .release_gate
            .missing_certifications(artifact.certifications());
        if !missing.is_empty() {
            return Err(ProjectError::MissingCertifications(missing));
        }
        let previous_artifact_id = self.current_artifact(channel_id).cloned();
        if previous_artifact_id.as_ref() == Some(artifact_id) {
            return Err(ProjectError::NoChange);
        }
        let promotion = Promotion::new(
            promotion_id.clone(),
            channel_id.clone(),
            artifact_id.clone(),
            previous_artifact_id,
            audit_context.actor.clone(),
            audit_context.occurred_at,
        );
        self.promotion_ids.insert(promotion_id.clone());
        self.promotions.push(promotion);
        self.append_audit(
            audit_context,
            AuditAction::ArtifactPromoted {
                promotion_id,
                channel_id: channel_id.clone(),
                artifact_id: artifact_id.clone(),
            },
        );
        self.append_contribution(
            contribution_context,
            ContributionKind::ReleasePromotion,
            ContributionChannel::Release(channel_id.clone()),
            WorkflowState::Released,
            ContributionSubject::Artifact(artifact_id.clone()),
        );
        Ok(())
    }

    #[must_use]
    pub fn current_artifact(&self, channel_id: &ReleaseChannelId) -> Option<&ArtifactId> {
        self.promotions
            .iter()
            .rev()
            .find(|promotion| promotion.channel_id() == channel_id)
            .map(Promotion::artifact_id)
    }

    #[must_use]
    pub fn promotion_history(&self, channel_id: &ReleaseChannelId) -> Vec<&Promotion> {
        self.promotions
            .iter()
            .filter(|promotion| promotion.channel_id() == channel_id)
            .collect()
    }

    pub fn add_kpi_policy(
        &mut self,
        version: FormulaVersion,
        effective_from: Day,
        formula: KpiFormula,
        context: AuditContext,
    ) -> Result<(), ProjectError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require(&context.actor, Capability::ManageKpiPolicies)?;
        if self.kpi_versions.contains(&version) {
            return Err(ProjectError::DuplicateFormulaVersion(version));
        }
        if self.kpi_policies.contains_key(&effective_from) {
            return Err(ProjectError::DuplicatePolicyDate(effective_from));
        }
        let policy = KpiPolicy::new(
            version.clone(),
            effective_from,
            context.occurred_at,
            formula,
        );
        self.kpi_versions.insert(version.clone());
        self.kpi_policies.insert(effective_from, policy);
        self.append_audit(context, AuditAction::KpiPolicyAdded { version });
        Ok(())
    }

    #[must_use]
    pub fn kpi_policy_for(&self, day: Day) -> Option<&KpiPolicy> {
        self.kpi_policies
            .range(..=day)
            .next_back()
            .map(|(_, policy)| policy)
    }

    pub fn daily_activity(
        &self,
        user: &UserId,
        start: Day,
        end: Day,
    ) -> Result<Vec<DailyActivity>, KpiError> {
        if end < start {
            return Err(KpiError::ReversedRange);
        }
        let length = i128::from(end.unix_day()) - i128::from(start.unix_day()) + 1;
        if length > i128::from(MAX_ACTIVITY_DAYS) {
            return Err(KpiError::RangeTooLarge);
        }
        let mut result = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
        let mut day = start;
        loop {
            let policy = self.kpi_policy_for(day);
            let mut score = 0_u64;
            if let Some(policy) = policy {
                for event in self
                    .contribution_events
                    .iter()
                    .filter(|event| event.contributor() == user && event.occurred_at().day() == day)
                {
                    score = score.saturating_add(u64::from(policy.formula().score(
                        event.kind(),
                        event.channel(),
                        event.workflow_state(),
                    )));
                }
                if let Some(cap) = policy.formula().daily_cap() {
                    score = score.min(u64::from(cap));
                }
            }
            result.push(DailyActivity {
                day,
                score: u32::try_from(score).unwrap_or(u32::MAX),
                formula_version: policy.map(|policy| policy.version().clone()),
            });
            if day == end {
                break;
            }
            day = day.checked_next().ok_or(KpiError::DayOverflow)?;
        }
        Ok(result)
    }

    fn recompute_candidate_state(&mut self, candidate_id: &CandidateId) -> ReviewState {
        let candidate = self
            .candidates
            .get(candidate_id)
            .expect("candidate exists while recomputing");
        let latest = Self::latest_decisions(candidate);
        let state = if latest
            .values()
            .any(|decision| *decision == ReviewDecision::Reject)
        {
            ReviewState::Rejected
        } else if latest
            .values()
            .any(|decision| *decision == ReviewDecision::RequestChanges)
        {
            ReviewState::ChangesRequested
        } else if Self::approval_status_for(candidate).satisfied {
            ReviewState::Approved
        } else {
            ReviewState::Open
        };
        self.candidates
            .get_mut(candidate_id)
            .expect("candidate exists while recomputing")
            .set_state(state);
        state
    }

    fn latest_decisions(candidate: &ReviewCandidate) -> BTreeMap<UserId, ReviewDecision> {
        candidate
            .reviews()
            .iter()
            .map(|review| (review.reviewer().clone(), review.decision()))
            .collect()
    }

    fn approval_status_for(candidate: &ReviewCandidate) -> ApprovalStatus {
        let approving_users = Self::latest_decisions(candidate)
            .into_iter()
            .filter_map(|(user, decision)| (decision == ReviewDecision::Approve).then_some(user))
            .collect::<BTreeSet<_>>();
        let approvals = u16::try_from(approving_users.len()).unwrap_or(u16::MAX);
        let required = candidate.approval_policy().min_approvals();
        ApprovalStatus {
            approvals,
            required,
            satisfied: approvals >= required,
            approving_users,
        }
    }

    fn validate_roles(&self, roles: &BTreeSet<ProjectRole>) -> Result<(), ProjectError> {
        if roles.is_empty() {
            return Err(ProjectError::EmptyRoles);
        }
        for role in roles {
            if let ProjectRole::Custom(role_id) = role
                && !self.custom_roles.contains_key(role_id)
            {
                return Err(ProjectError::RoleNotFound(role_id.clone()));
            }
        }
        Ok(())
    }

    fn owner_count(&self) -> usize {
        self.members
            .values()
            .filter(|member| member.is_owner())
            .count()
    }

    fn require(&self, actor: &UserId, capability: Capability) -> Result<(), ProjectError> {
        if self.has_capability(actor, &capability) {
            Ok(())
        } else {
            Err(ProjectError::PermissionDenied {
                actor: actor.clone(),
                capability,
            })
        }
    }

    fn require_project_owner(&self, actor: &UserId) -> Result<(), ProjectError> {
        if self.members.get(actor).is_some_and(ProjectMember::is_owner) {
            Ok(())
        } else {
            Err(ProjectError::ProjectOwnerRequired)
        }
    }

    fn ensure_contexts_match(
        &self,
        audit: &AuditContext,
        contribution: &ContributionContext,
    ) -> Result<(), ProjectError> {
        if audit.actor == contribution.actor && audit.occurred_at == contribution.occurred_at {
            Ok(())
        } else {
            Err(ProjectError::EventContextMismatch)
        }
    }

    fn ensure_fresh_audit(&self, id: &AuditEventId) -> Result<(), ProjectError> {
        if self.audit_ids.contains(id) {
            Err(ProjectError::DuplicateAuditEvent(id.clone()))
        } else {
            Ok(())
        }
    }

    fn ensure_fresh_contribution(&self, id: &ContributionEventId) -> Result<(), ProjectError> {
        if self.contribution_ids.contains(id) {
            Err(ProjectError::DuplicateContributionEvent(id.clone()))
        } else {
            Ok(())
        }
    }

    fn append_audit(&mut self, context: AuditContext, action: AuditAction) {
        let event = AuditEvent::new(
            context,
            self.workspace_id.clone(),
            Some(self.id.clone()),
            action,
        );
        self.audit_ids.insert(event.id().clone());
        self.audit_events.push(event);
    }

    fn append_contribution(
        &mut self,
        context: ContributionContext,
        kind: ContributionKind,
        channel: ContributionChannel,
        workflow_state: WorkflowState,
        subject: ContributionSubject,
    ) {
        let event = ContributionEvent::new(
            context,
            self.id.clone(),
            kind,
            channel,
            workflow_state,
            subject,
        );
        self.contribution_ids.insert(event.id().clone());
        self.contribution_events.push(event);
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ProjectError {
    #[error("the event actor must be the project creator")]
    ActorMismatch,
    #[error("audit and contribution contexts must have the same actor and timestamp")]
    EventContextMismatch,
    #[error("actor '{actor}' lacks capability {capability:?}")]
    PermissionDenied {
        actor: UserId,
        capability: Capability,
    },
    #[error("a project owner is required")]
    ProjectOwnerRequired,
    #[error("the last project owner cannot be removed or demoted")]
    LastOwner,
    #[error("project roles cannot be empty")]
    EmptyRoles,
    #[error("a name cannot be empty")]
    EmptyName,
    #[error("custom role '{0}' was not found")]
    RoleNotFound(RoleId),
    #[error("custom role '{0}' already exists")]
    DuplicateRole(RoleId),
    #[error("user '{0}' is already a project member")]
    AlreadyMember(UserId),
    #[error("project member '{0}' was not found")]
    MemberNotFound(UserId),
    #[error("invitation '{0}' already exists")]
    DuplicateInvitation(InvitationId),
    #[error("a pending invitation already exists for '{0}'")]
    PendingInvitationExists(UserId),
    #[error("invitation '{0}' was not found")]
    InvitationNotFound(InvitationId),
    #[error("the invitation does not belong to the event actor")]
    InvitationNotForActor,
    #[error("the invitation is not pending")]
    InvitationNotPending,
    #[error("the invitation has expired")]
    InvitationExpired,
    #[error("invitation expiration must be after its creation")]
    InvalidExpiration,
    #[error("the requested operation would make no change")]
    NoChange,
    #[error("level '{0}' already exists")]
    DuplicateLevel(LevelId),
    #[error("level '{0}' was not found")]
    LevelNotFound(LevelId),
    #[error("level duration must be finite and nonnegative")]
    InvalidLevelDuration,
    #[error("revision '{0}' already exists")]
    DuplicateRevision(RevisionId),
    #[error("revision '{0}' was not found on the level timeline")]
    RevisionNotFound(RevisionId),
    #[error("the level timeline sequence is exhausted")]
    TimelineExhausted,
    #[error("review candidate '{0}' already exists")]
    DuplicateCandidate(CandidateId),
    #[error("review candidate '{0}' was not found")]
    CandidateNotFound(CandidateId),
    #[error("candidate is terminal in state {0:?}")]
    CandidateTerminal(ReviewState),
    #[error("review comment cannot be empty")]
    EmptyComment,
    #[error("review anchor must reference the candidate revision")]
    AnchorRevisionMismatch,
    #[error("comment '{0}' already exists")]
    DuplicateComment(CommentId),
    #[error("review '{0}' already exists")]
    DuplicateReview(ReviewId),
    #[error("the candidate approval policy forbids self approval")]
    SelfApprovalForbidden,
    #[error("approver lacks required capabilities {0:?}")]
    MissingApprovalCapabilities(BTreeSet<Capability>),
    #[error("candidate must be approved before producing an artifact")]
    CandidateNotApproved,
    #[error("release gate certifications are missing: {0:?}")]
    MissingCertifications(MissingCertifications),
    #[error("artifact '{0}' already exists")]
    DuplicateArtifact(ArtifactId),
    #[error("artifact '{0}' was not found")]
    ArtifactNotFound(ArtifactId),
    #[error("release channel '{0}' already exists")]
    DuplicateReleaseChannel(ReleaseChannelId),
    #[error("release channel '{0}' was not found")]
    ReleaseChannelNotFound(ReleaseChannelId),
    #[error("promotion '{0}' already exists")]
    DuplicatePromotion(PromotionId),
    #[error("KPI formula version '{0}' already exists")]
    DuplicateFormulaVersion(FormulaVersion),
    #[error("a KPI policy is already effective on Unix day {0:?}")]
    DuplicatePolicyDate(Day),
    #[error("audit event '{0}' already exists")]
    DuplicateAuditEvent(AuditEventId),
    #[error("contribution event '{0}' already exists")]
    DuplicateContributionEvent(ContributionEventId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArtifactDigest, CertificationId, CodecCertifications, OpaqueCertification, PluginId,
    };

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }

    fn audit(id: &str, actor: &str, at: i64) -> AuditContext {
        AuditContext::new(
            AuditEventId::new(id).unwrap(),
            user(actor),
            TimestampMs::new(at),
        )
    }

    fn contribution(id: &str, actor: &str, at: i64) -> ContributionContext {
        ContributionContext::new(
            ContributionEventId::new(id).unwrap(),
            user(actor),
            TimestampMs::new(at),
        )
    }

    fn policy(
        self_approval: bool,
        approvals: u16,
        required: BTreeSet<Capability>,
    ) -> ApprovalPolicy {
        ApprovalPolicy::new(self_approval, approvals, required).unwrap()
    }

    fn project_with_policy(approval_policy: ApprovalPolicy, release_gate: ReleaseGate) -> Project {
        Project::new(
            ProjectId::new("project").unwrap(),
            WorkspaceId::new("workspace").unwrap(),
            user("owner"),
            ThemeId::new("dark").unwrap(),
            approval_policy,
            release_gate,
            audit("project-created", "owner", 0),
        )
        .unwrap()
    }

    fn project() -> Project {
        project_with_policy(policy(true, 1, BTreeSet::new()), ReleaseGate::default())
    }

    fn roles(values: &[ProjectRole]) -> BTreeSet<ProjectRole> {
        values.iter().cloned().collect()
    }

    fn level_configuration(name: &str) -> LevelConfiguration {
        LevelConfiguration::new(name, 0.0).unwrap()
    }

    fn add_member(project: &mut Project, name: &str, member_roles: BTreeSet<ProjectRole>, at: i64) {
        let invitation_id = InvitationId::new(format!("invite-{name}")).unwrap();
        project
            .invite_member(
                invitation_id.clone(),
                user(name),
                member_roles,
                None,
                audit(&format!("invited-{name}"), "owner", at),
            )
            .unwrap();
        project
            .accept_invitation(
                &invitation_id,
                audit(&format!("accepted-{name}"), name, at + 1),
            )
            .unwrap();
    }

    fn add_revision(project: &mut Project, actor: &str) -> (LevelId, RevisionId) {
        let level_id = LevelId::new("level").unwrap();
        let revision_id = RevisionId::new("revision-1").unwrap();
        project
            .create_level(
                level_id.clone(),
                level_configuration("Level"),
                audit("level-created", actor, 10),
            )
            .unwrap();
        project
            .append_timeline_revision(
                &level_id,
                revision_id.clone(),
                contribution("revision-created", actor, 11),
            )
            .unwrap();
        (level_id, revision_id)
    }

    fn approved_candidate(project: &mut Project) -> CandidateId {
        let (level_id, revision_id) = add_revision(project, "owner");
        let candidate_id = CandidateId::new("candidate").unwrap();
        project
            .create_review_candidate(
                candidate_id.clone(),
                &level_id,
                &revision_id,
                contribution("candidate-created", "owner", 12),
            )
            .unwrap();
        project
            .submit_review(
                &candidate_id,
                ReviewId::new("approval").unwrap(),
                ReviewDecision::Approve,
                contribution("candidate-approved", "owner", 13),
            )
            .unwrap();
        candidate_id
    }

    #[test]
    fn workspace_administration_never_implies_project_access() {
        let mut project = project();
        let result = project.create_level(
            LevelId::new("unauthorized").unwrap(),
            level_configuration("Unauthorized"),
            audit("other-level", "organization-admin", 1),
        );
        assert_eq!(
            result,
            Err(ProjectError::PermissionDenied {
                actor: user("organization-admin"),
                capability: Capability::EditTimeline,
            })
        );
    }

    #[test]
    fn admin_template_needs_an_explicit_editor_role_to_edit() {
        let mut project = project();
        add_member(&mut project, "admin", roles(&[ProjectRole::admin()]), 1);
        assert!(matches!(
            project.create_level(
                LevelId::new("level-a").unwrap(),
                level_configuration("Level A"),
                audit("admin-level-a", "admin", 3)
            ),
            Err(ProjectError::PermissionDenied { .. })
        ));
        project
            .set_member_roles(
                &user("admin"),
                roles(&[ProjectRole::admin(), ProjectRole::editor()]),
                audit("admin-is-editor", "owner", 4),
            )
            .unwrap();
        project
            .create_level(
                LevelId::new("level-b").unwrap(),
                level_configuration("Level B"),
                audit("admin-level-b", "admin", 5),
            )
            .unwrap();
    }

    #[test]
    fn level_configuration_is_validated_and_trimmed() {
        let configuration = LevelConfiguration::new("  Opening  ", 12.5).unwrap();
        assert_eq!(configuration.name(), "Opening");
        assert_eq!(configuration.duration_seconds(), 12.5);
        assert_eq!(
            LevelConfiguration::new("  ", 0.0),
            Err(ProjectError::EmptyName)
        );
        for duration in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                LevelConfiguration::new("Level", duration),
                Err(ProjectError::InvalidLevelDuration)
            );
        }
    }

    #[test]
    fn level_configuration_is_owned_by_the_timeline_and_changes_are_audited() {
        let mut project = project();
        let level_id = LevelId::new("level").unwrap();
        project
            .create_level(
                level_id.clone(),
                LevelConfiguration::new("  Opening  ", 10.0).unwrap(),
                audit("level-created", "owner", 1),
            )
            .unwrap();
        assert_eq!(
            project.timelines()[&level_id].configuration().name(),
            "Opening"
        );

        project
            .set_level_configuration(
                &level_id,
                LevelConfiguration::new("Finale", 30.0).unwrap(),
                audit("level-configured", "owner", 2),
            )
            .unwrap();
        let configuration = project.timelines()[&level_id].configuration();
        assert_eq!(configuration.name(), "Finale");
        assert_eq!(configuration.duration_seconds(), 30.0);
        assert!(matches!(
            project.audit_events().last().unwrap().action(),
            AuditAction::LevelConfigurationChanged { level_id: changed } if changed == &level_id
        ));
        assert_eq!(
            project.set_level_configuration(
                &level_id,
                configuration.clone(),
                audit("level-unchanged", "owner", 3),
            ),
            Err(ProjectError::NoChange)
        );
    }

    #[test]
    fn last_project_owner_is_protected() {
        let mut project = project();
        assert_eq!(
            project.set_member_roles(
                &user("owner"),
                roles(&[ProjectRole::admin()]),
                audit("demote-owner", "owner", 1),
            ),
            Err(ProjectError::LastOwner)
        );
    }

    #[test]
    fn timeline_is_linear_and_candidates_keep_their_revision() {
        let mut project = project();
        let (level_id, revision_1) = add_revision(&mut project, "owner");
        let revision_2 = RevisionId::new("revision-2").unwrap();
        project
            .append_timeline_revision(
                &level_id,
                revision_2.clone(),
                contribution("revision-2-event", "owner", 12),
            )
            .unwrap();
        assert_eq!(
            project.timelines()[&level_id].revisions()[1].parent_revision_id(),
            Some(&revision_1)
        );

        let candidate = CandidateId::new("candidate-old").unwrap();
        project
            .create_review_candidate(
                candidate.clone(),
                &level_id,
                &revision_1,
                contribution("old-candidate", "owner", 13),
            )
            .unwrap();
        assert_eq!(project.candidates()[&candidate].revision_id(), &revision_1);
        assert_eq!(
            project.timelines()[&level_id].head().unwrap().id(),
            &revision_2
        );
    }

    #[test]
    fn approvals_enforce_self_minimum_and_required_capabilities_independently() {
        let required = [Capability::Custom("senior-reviewer".into())]
            .into_iter()
            .collect();
        let mut project = project_with_policy(policy(false, 2, required), ReleaseGate::default());
        let reviewer_role = RoleId::new("reviewer").unwrap();
        project
            .create_custom_role(
                CustomRole::new(
                    reviewer_role.clone(),
                    "Reviewer",
                    [
                        Capability::ViewProject,
                        Capability::SubmitReview,
                        Capability::ApproveReview,
                        Capability::Custom("senior-reviewer".into()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                audit("role-created", "owner", 1),
            )
            .unwrap();
        add_member(
            &mut project,
            "reviewer-a",
            roles(&[ProjectRole::Custom(reviewer_role.clone())]),
            2,
        );
        add_member(
            &mut project,
            "reviewer-b",
            roles(&[ProjectRole::Custom(reviewer_role)]),
            5,
        );
        let (level_id, revision_id) = add_revision(&mut project, "owner");
        let candidate = CandidateId::new("candidate").unwrap();
        project
            .create_review_candidate(
                candidate.clone(),
                &level_id,
                &revision_id,
                contribution("candidate", "owner", 20),
            )
            .unwrap();
        assert_eq!(
            project.submit_review(
                &candidate,
                ReviewId::new("self-review").unwrap(),
                ReviewDecision::Approve,
                contribution("self-approval", "owner", 21),
            ),
            Err(ProjectError::SelfApprovalForbidden)
        );
        assert_eq!(
            project
                .submit_review(
                    &candidate,
                    ReviewId::new("approval-a").unwrap(),
                    ReviewDecision::Approve,
                    contribution("review-a", "reviewer-a", 22),
                )
                .unwrap(),
            ReviewState::Open
        );
        assert_eq!(
            project
                .submit_review(
                    &candidate,
                    ReviewId::new("approval-b").unwrap(),
                    ReviewDecision::Approve,
                    contribution("review-b", "reviewer-b", 23),
                )
                .unwrap(),
            ReviewState::Approved
        );
        assert_eq!(project.approval_status(&candidate).unwrap().approvals, 2);
    }

    #[test]
    fn latest_reviewer_decisions_drive_nonterminal_state_changes() {
        let mut project =
            project_with_policy(policy(true, 2, BTreeSet::new()), ReleaseGate::default());
        add_member(&mut project, "reviewer", roles(&[ProjectRole::owner()]), 1);
        let (level_id, revision_id) = add_revision(&mut project, "owner");
        let candidate = CandidateId::new("candidate").unwrap();
        project
            .create_review_candidate(
                candidate.clone(),
                &level_id,
                &revision_id,
                contribution("candidate", "owner", 12),
            )
            .unwrap();
        assert_eq!(
            project
                .submit_review(
                    &candidate,
                    ReviewId::new("changes").unwrap(),
                    ReviewDecision::RequestChanges,
                    contribution("changes-event", "reviewer", 13),
                )
                .unwrap(),
            ReviewState::ChangesRequested
        );
        project
            .submit_review(
                &candidate,
                ReviewId::new("reviewer-approval").unwrap(),
                ReviewDecision::Approve,
                contribution("reviewer-approval-event", "reviewer", 14),
            )
            .unwrap();
        assert_eq!(
            project
                .submit_review(
                    &candidate,
                    ReviewId::new("owner-approval").unwrap(),
                    ReviewDecision::Approve,
                    contribution("owner-approval-event", "owner", 15),
                )
                .unwrap(),
            ReviewState::Approved
        );
    }

    #[test]
    fn comments_must_anchor_to_the_candidate_revision() {
        let mut project = project();
        let (level_id, revision_id) = add_revision(&mut project, "owner");
        let candidate = CandidateId::new("candidate").unwrap();
        project
            .create_review_candidate(
                candidate.clone(),
                &level_id,
                &revision_id,
                contribution("candidate", "owner", 12),
            )
            .unwrap();
        assert_eq!(
            project.add_review_comment(
                &candidate,
                CommentId::new("comment").unwrap(),
                "Move this wall",
                ReviewAnchor::revision(RevisionId::new("other").unwrap()),
                contribution("comment-event", "owner", 13),
            ),
            Err(ProjectError::AnchorRevisionMismatch)
        );
    }

    #[test]
    fn codec_gates_and_promotion_history_are_immutable_inputs() {
        let plugin = PluginId::new("opaque-plugin").unwrap();
        let mut project = project_with_policy(
            policy(true, 1, BTreeSet::new()),
            ReleaseGate::new(true, [plugin.clone()].into_iter().collect()),
        );
        let candidate = approved_candidate(&mut project);
        let incomplete = ArtifactInput {
            id: ArtifactId::new("artifact-1").unwrap(),
            digest: ArtifactDigest::new("digest-1").unwrap(),
            candidate_id: candidate.clone(),
            certifications: CodecCertifications::default(),
        };
        assert!(matches!(
            project.create_artifact(
                incomplete,
                audit("artifact-audit-failed", "owner", 14),
                contribution("artifact-contribution-failed", "owner", 14),
            ),
            Err(ProjectError::MissingCertifications(_))
        ));

        let certifications = CodecCertifications {
            project_codec: Some(
                OpaqueCertification::new(
                    CertificationId::new("project-cert").unwrap(),
                    "opaque-project-proof",
                )
                .unwrap(),
            ),
            plugin_codecs: [(
                plugin,
                OpaqueCertification::new(
                    CertificationId::new("plugin-cert").unwrap(),
                    "opaque-plugin-proof",
                )
                .unwrap(),
            )]
            .into_iter()
            .collect(),
        };
        let artifact_id = ArtifactId::new("artifact-1").unwrap();
        project
            .create_artifact(
                ArtifactInput {
                    id: artifact_id.clone(),
                    digest: ArtifactDigest::new("digest-1").unwrap(),
                    candidate_id: candidate,
                    certifications,
                },
                audit("artifact-audit", "owner", 15),
                contribution("artifact-contribution", "owner", 15),
            )
            .unwrap();
        let channel = ReleaseChannelId::new("nightly").unwrap();
        project
            .create_release_channel(channel.clone(), "Nightly", audit("channel", "owner", 16))
            .unwrap();
        project
            .promote_artifact(
                PromotionId::new("promotion-1").unwrap(),
                &channel,
                &artifact_id,
                audit("promotion-audit", "owner", 17),
                contribution("promotion-contribution", "owner", 17),
            )
            .unwrap();
        assert_eq!(project.current_artifact(&channel), Some(&artifact_id));
        assert_eq!(project.promotion_history(&channel).len(), 1);
        assert_eq!(
            project.promote_artifact(
                PromotionId::new("promotion-2").unwrap(),
                &channel,
                &artifact_id,
                audit("promotion-2-audit", "owner", 18),
                contribution("promotion-2-contribution", "owner", 18),
            ),
            Err(ProjectError::NoChange)
        );
        assert_eq!(project.promotion_history(&channel).len(), 1);
    }

    #[test]
    fn theme_override_falls_back_to_project_default() {
        let mut project = project();
        assert_eq!(project.effective_theme(&user("owner")).as_str(), "dark");
        project
            .set_my_theme_override(
                Some(ThemeId::new("light").unwrap()),
                audit("theme-override", "owner", 1),
            )
            .unwrap();
        assert_eq!(project.effective_theme(&user("owner")).as_str(), "light");
        project
            .set_my_theme_override(None, audit("theme-clear", "owner", 2))
            .unwrap();
        assert_eq!(project.effective_theme(&user("owner")).as_str(), "dark");
    }

    #[test]
    fn effective_dated_kpis_are_deterministic_and_capped() {
        let mut project = project();
        let day_0 = Day::new(0);
        let day_1 = Day::new(1);
        let formula_1 = KpiFormula::new(
            [(ContributionKind::TimelineRevision, 7)]
                .into_iter()
                .collect(),
            Some(10),
            [ContributionChannel::Timeline].into_iter().collect(),
            [WorkflowState::Active].into_iter().collect(),
        )
        .unwrap();
        project
            .add_kpi_policy(
                FormulaVersion::new("v1").unwrap(),
                day_0,
                formula_1,
                audit("kpi-v1", "owner", 0),
            )
            .unwrap();
        let formula_2 = KpiFormula::new(
            [(ContributionKind::TimelineRevision, 2)]
                .into_iter()
                .collect(),
            None,
            [ContributionChannel::Timeline].into_iter().collect(),
            [WorkflowState::Active].into_iter().collect(),
        )
        .unwrap();
        project
            .add_kpi_policy(
                FormulaVersion::new("v2").unwrap(),
                day_1,
                formula_2,
                audit("kpi-v2", "owner", 1),
            )
            .unwrap();

        let level = LevelId::new("level").unwrap();
        project
            .create_level(
                level.clone(),
                level_configuration("Level"),
                audit("level", "owner", 2),
            )
            .unwrap();
        project
            .append_timeline_revision(
                &level,
                RevisionId::new("r1").unwrap(),
                contribution("r1-event", "owner", 3),
            )
            .unwrap();
        project
            .append_timeline_revision(
                &level,
                RevisionId::new("r2").unwrap(),
                contribution("r2-event", "owner", 4),
            )
            .unwrap();
        project
            .append_timeline_revision(
                &level,
                RevisionId::new("r3").unwrap(),
                contribution("r3-event", "owner", 86_400_001),
            )
            .unwrap();

        let activity = project
            .daily_activity(&user("owner"), day_0, day_1)
            .unwrap();
        assert_eq!(activity[0].score, 10);
        assert_eq!(activity[0].formula_version.as_ref().unwrap().as_str(), "v1");
        assert_eq!(activity[1].score, 2);
        assert_eq!(activity[1].formula_version.as_ref().unwrap().as_str(), "v2");
        assert_eq!(
            activity,
            project
                .daily_activity(&user("owner"), day_0, day_1)
                .unwrap()
        );
    }

    #[test]
    fn audit_and_contribution_streams_are_separate() {
        let mut project = project();
        let audit_count = project.audit_events().len();
        let contribution_count = project.contribution_events().len();
        let level = LevelId::new("level").unwrap();
        project
            .create_level(
                level.clone(),
                level_configuration("Level"),
                audit("level", "owner", 1),
            )
            .unwrap();
        project
            .append_timeline_revision(
                &level,
                RevisionId::new("revision").unwrap(),
                contribution("revision", "owner", 2),
            )
            .unwrap();
        assert_eq!(project.audit_events().len(), audit_count + 1);
        assert_eq!(project.contribution_events().len(), contribution_count + 1);
        assert!(matches!(
            project.contribution_events()[0].kind(),
            ContributionKind::TimelineRevision
        ));
    }
}

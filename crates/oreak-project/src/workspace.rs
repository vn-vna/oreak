use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AuditAction, AuditContext, AuditEvent, AuditEventId, InvitationId, ProjectId, TimestampMs,
    UserId, WorkspaceId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceKind {
    Personal,
    Organization,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMember {
    user_id: UserId,
    role: WorkspaceRole,
    joined_at: TimestampMs,
}

impl WorkspaceMember {
    #[must_use]
    pub const fn user_id(&self) -> &UserId {
        &self.user_id
    }

    #[must_use]
    pub const fn role(&self) -> WorkspaceRole {
        self.role
    }

    #[must_use]
    pub const fn joined_at(&self) -> TimestampMs {
        self.joined_at
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceInvitationState {
    Pending,
    Accepted,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInvitation {
    id: InvitationId,
    invitee: UserId,
    role: WorkspaceRole,
    invited_by: UserId,
    invited_at: TimestampMs,
    expires_at: Option<TimestampMs>,
    state: WorkspaceInvitationState,
}

impl WorkspaceInvitation {
    #[must_use]
    pub const fn id(&self) -> &InvitationId {
        &self.id
    }

    #[must_use]
    pub const fn invitee(&self) -> &UserId {
        &self.invitee
    }

    #[must_use]
    pub const fn role(&self) -> WorkspaceRole {
        self.role
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
    pub const fn state(&self) -> WorkspaceInvitationState {
        self.state
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    id: WorkspaceId,
    kind: WorkspaceKind,
    members: BTreeMap<UserId, WorkspaceMember>,
    invitations: BTreeMap<InvitationId, WorkspaceInvitation>,
    projects: BTreeSet<ProjectId>,
    audit_events: Vec<AuditEvent>,
    audit_ids: BTreeSet<AuditEventId>,
}

impl Workspace {
    pub fn new_personal(
        id: WorkspaceId,
        owner: UserId,
        context: AuditContext,
    ) -> Result<Self, WorkspaceError> {
        Self::new(id, WorkspaceKind::Personal, owner, context)
    }

    pub fn new_organization(
        id: WorkspaceId,
        owner: UserId,
        context: AuditContext,
    ) -> Result<Self, WorkspaceError> {
        Self::new(id, WorkspaceKind::Organization, owner, context)
    }

    fn new(
        id: WorkspaceId,
        kind: WorkspaceKind,
        owner: UserId,
        context: AuditContext,
    ) -> Result<Self, WorkspaceError> {
        if context.actor != owner {
            return Err(WorkspaceError::ActorMismatch);
        }
        let member = WorkspaceMember {
            user_id: owner.clone(),
            role: WorkspaceRole::Owner,
            joined_at: context.occurred_at,
        };
        let event = AuditEvent::new(context, id.clone(), None, AuditAction::WorkspaceCreated);
        Ok(Self {
            id,
            kind,
            members: [(owner, member)].into_iter().collect(),
            invitations: BTreeMap::new(),
            projects: BTreeSet::new(),
            audit_ids: [event.id().clone()].into_iter().collect(),
            audit_events: vec![event],
        })
    }

    #[must_use]
    pub const fn id(&self) -> &WorkspaceId {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> WorkspaceKind {
        self.kind
    }

    #[must_use]
    pub const fn members(&self) -> &BTreeMap<UserId, WorkspaceMember> {
        &self.members
    }

    #[must_use]
    pub const fn invitations(&self) -> &BTreeMap<InvitationId, WorkspaceInvitation> {
        &self.invitations
    }

    #[must_use]
    pub const fn projects(&self) -> &BTreeSet<ProjectId> {
        &self.projects
    }

    #[must_use]
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.audit_events
    }

    #[must_use]
    pub fn can_administer(&self, user: &UserId) -> bool {
        self.members.get(user).is_some_and(|member| {
            matches!(member.role, WorkspaceRole::Owner | WorkspaceRole::Admin)
        })
    }

    pub fn invite_member(
        &mut self,
        invitation_id: InvitationId,
        invitee: UserId,
        role: WorkspaceRole,
        expires_at: Option<TimestampMs>,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        if self.kind == WorkspaceKind::Personal {
            return Err(WorkspaceError::PersonalWorkspaceMembershipFixed);
        }
        self.require_admin(&context.actor)?;
        if role == WorkspaceRole::Owner {
            self.require_owner(&context.actor)?;
        }
        if self.members.contains_key(&invitee) {
            return Err(WorkspaceError::AlreadyMember(invitee));
        }
        if self.invitations.contains_key(&invitation_id) {
            return Err(WorkspaceError::DuplicateInvitation(invitation_id));
        }
        if expires_at.is_some_and(|expiry| expiry.as_i64() <= context.occurred_at.as_i64()) {
            return Err(WorkspaceError::InvalidExpiration);
        }
        if self.invitations.values().any(|invitation| {
            invitation.invitee == invitee
                && invitation.state == WorkspaceInvitationState::Pending
                && invitation
                    .expires_at
                    .is_none_or(|expiry| expiry.as_i64() > context.occurred_at.as_i64())
        }) {
            return Err(WorkspaceError::PendingInvitationExists(invitee));
        }

        let invitation = WorkspaceInvitation {
            id: invitation_id.clone(),
            invitee: invitee.clone(),
            role,
            invited_by: context.actor.clone(),
            invited_at: context.occurred_at,
            expires_at,
            state: WorkspaceInvitationState::Pending,
        };
        self.invitations.insert(invitation_id.clone(), invitation);
        self.append_audit(
            context,
            AuditAction::WorkspaceInvitationCreated {
                invitation_id,
                invitee,
                role,
            },
        );
        Ok(())
    }

    pub fn accept_invitation(
        &mut self,
        invitation_id: &InvitationId,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        let invitation = self
            .invitations
            .get(invitation_id)
            .ok_or_else(|| WorkspaceError::InvitationNotFound(invitation_id.clone()))?;
        if invitation.invitee != context.actor {
            return Err(WorkspaceError::InvitationNotForActor);
        }
        if invitation.state != WorkspaceInvitationState::Pending {
            return Err(WorkspaceError::InvitationNotPending);
        }
        if invitation
            .expires_at
            .is_some_and(|expiry| context.occurred_at.as_i64() >= expiry.as_i64())
        {
            return Err(WorkspaceError::InvitationExpired);
        }
        if self.members.contains_key(&context.actor) {
            return Err(WorkspaceError::AlreadyMember(context.actor));
        }

        let role = invitation.role;
        let member = WorkspaceMember {
            user_id: context.actor.clone(),
            role,
            joined_at: context.occurred_at,
        };
        self.members.insert(context.actor.clone(), member);
        self.invitations
            .get_mut(invitation_id)
            .expect("invitation was checked")
            .state = WorkspaceInvitationState::Accepted;
        self.append_audit(
            context,
            AuditAction::WorkspaceInvitationAccepted {
                invitation_id: invitation_id.clone(),
            },
        );
        Ok(())
    }

    pub fn revoke_invitation(
        &mut self,
        invitation_id: &InvitationId,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require_admin(&context.actor)?;
        let invitation = self
            .invitations
            .get(invitation_id)
            .ok_or_else(|| WorkspaceError::InvitationNotFound(invitation_id.clone()))?;
        if invitation.state != WorkspaceInvitationState::Pending {
            return Err(WorkspaceError::InvitationNotPending);
        }
        if invitation.role == WorkspaceRole::Owner {
            self.require_owner(&context.actor)?;
        }
        self.invitations
            .get_mut(invitation_id)
            .expect("invitation was checked")
            .state = WorkspaceInvitationState::Revoked;
        self.append_audit(
            context,
            AuditAction::WorkspaceInvitationRevoked {
                invitation_id: invitation_id.clone(),
            },
        );
        Ok(())
    }

    pub fn change_member_role(
        &mut self,
        member_id: &UserId,
        role: WorkspaceRole,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        if self.kind == WorkspaceKind::Personal {
            return Err(WorkspaceError::PersonalWorkspaceMembershipFixed);
        }
        self.require_admin(&context.actor)?;
        let current = self
            .members
            .get(member_id)
            .ok_or_else(|| WorkspaceError::MemberNotFound(member_id.clone()))?
            .role;
        if current == role {
            return Err(WorkspaceError::NoChange);
        }
        if current == WorkspaceRole::Owner || role == WorkspaceRole::Owner {
            self.require_owner(&context.actor)?;
        }
        if current == WorkspaceRole::Owner && self.owner_count() == 1 {
            return Err(WorkspaceError::LastOwner);
        }
        self.members
            .get_mut(member_id)
            .expect("member was checked")
            .role = role;
        self.append_audit(
            context,
            AuditAction::WorkspaceMemberRoleChanged {
                member: member_id.clone(),
                role,
            },
        );
        Ok(())
    }

    pub fn remove_member(
        &mut self,
        member_id: &UserId,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        if self.kind == WorkspaceKind::Personal {
            return Err(WorkspaceError::PersonalWorkspaceMembershipFixed);
        }
        let member = self
            .members
            .get(member_id)
            .ok_or_else(|| WorkspaceError::MemberNotFound(member_id.clone()))?;
        if context.actor != *member_id {
            self.require_admin(&context.actor)?;
            if member.role == WorkspaceRole::Owner {
                self.require_owner(&context.actor)?;
            }
        }
        if member.role == WorkspaceRole::Owner && self.owner_count() == 1 {
            return Err(WorkspaceError::LastOwner);
        }
        self.members.remove(member_id);
        self.append_audit(
            context,
            AuditAction::WorkspaceMemberRemoved {
                member: member_id.clone(),
            },
        );
        Ok(())
    }

    pub fn register_project(
        &mut self,
        project_id: ProjectId,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require_admin(&context.actor)?;
        if !self.projects.insert(project_id.clone()) {
            return Err(WorkspaceError::DuplicateProject(project_id));
        }
        self.append_audit(context, AuditAction::ProjectRegistered { project_id });
        Ok(())
    }

    pub fn unregister_project(
        &mut self,
        project_id: &ProjectId,
        context: AuditContext,
    ) -> Result<(), WorkspaceError> {
        self.ensure_fresh_audit(&context.event_id)?;
        self.require_admin(&context.actor)?;
        if !self.projects.remove(project_id) {
            return Err(WorkspaceError::ProjectNotFound(project_id.clone()));
        }
        self.append_audit(
            context,
            AuditAction::ProjectUnregistered {
                project_id: project_id.clone(),
            },
        );
        Ok(())
    }

    fn owner_count(&self) -> usize {
        self.members
            .values()
            .filter(|member| member.role == WorkspaceRole::Owner)
            .count()
    }

    fn require_admin(&self, actor: &UserId) -> Result<(), WorkspaceError> {
        if self.can_administer(actor) {
            Ok(())
        } else {
            Err(WorkspaceError::PermissionDenied)
        }
    }

    fn require_owner(&self, actor: &UserId) -> Result<(), WorkspaceError> {
        if self
            .members
            .get(actor)
            .is_some_and(|member| member.role == WorkspaceRole::Owner)
        {
            Ok(())
        } else {
            Err(WorkspaceError::OwnerRequired)
        }
    }

    fn ensure_fresh_audit(&self, id: &AuditEventId) -> Result<(), WorkspaceError> {
        if self.audit_ids.contains(id) {
            Err(WorkspaceError::DuplicateAuditEvent(id.clone()))
        } else {
            Ok(())
        }
    }

    fn append_audit(&mut self, context: AuditContext, action: AuditAction) {
        let event = AuditEvent::new(context, self.id.clone(), None, action);
        self.audit_ids.insert(event.id().clone());
        self.audit_events.push(event);
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WorkspaceError {
    #[error("the event actor must be the initial owner")]
    ActorMismatch,
    #[error("permission denied")]
    PermissionDenied,
    #[error("an organization owner is required")]
    OwnerRequired,
    #[error("personal workspace membership is fixed to its owner")]
    PersonalWorkspaceMembershipFixed,
    #[error("user '{0}' is already a workspace member")]
    AlreadyMember(UserId),
    #[error("workspace member '{0}' was not found")]
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
    #[error("the last workspace owner cannot be removed or demoted")]
    LastOwner,
    #[error("the requested operation would make no change")]
    NoChange,
    #[error("project '{0}' is already registered")]
    DuplicateProject(ProjectId),
    #[error("project '{0}' was not found")]
    ProjectNotFound(ProjectId),
    #[error("audit event '{0}' already exists")]
    DuplicateAuditEvent(AuditEventId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(value: &str) -> UserId {
        UserId::new(value).unwrap()
    }

    fn invitation(value: &str) -> InvitationId {
        InvitationId::new(value).unwrap()
    }

    fn context(id: &str, actor: &str, at: i64) -> AuditContext {
        AuditContext::new(
            AuditEventId::new(id).unwrap(),
            user(actor),
            TimestampMs::new(at),
        )
    }

    fn organization() -> Workspace {
        Workspace::new_organization(
            WorkspaceId::new("workspace").unwrap(),
            user("owner"),
            context("create", "owner", 0),
        )
        .unwrap()
    }

    #[test]
    fn invitation_acceptance_is_explicit_and_audited() {
        let mut workspace = organization();
        workspace
            .invite_member(
                invitation("invite"),
                user("admin"),
                WorkspaceRole::Admin,
                Some(TimestampMs::new(100)),
                context("invite-event", "owner", 1),
            )
            .unwrap();
        workspace
            .accept_invitation(&invitation("invite"), context("accept-event", "admin", 2))
            .unwrap();
        assert_eq!(
            workspace.members()[&user("admin")].role(),
            WorkspaceRole::Admin
        );
        assert_eq!(workspace.audit_events().len(), 3);
    }

    #[test]
    fn last_owner_is_protected() {
        let mut workspace = organization();
        assert_eq!(
            workspace.remove_member(&user("owner"), context("remove", "owner", 1)),
            Err(WorkspaceError::LastOwner)
        );
    }

    #[test]
    fn admins_cannot_create_owners() {
        let mut workspace = organization();
        workspace
            .invite_member(
                invitation("admin-invite"),
                user("admin"),
                WorkspaceRole::Admin,
                None,
                context("admin-invited", "owner", 1),
            )
            .unwrap();
        workspace
            .accept_invitation(
                &invitation("admin-invite"),
                context("admin-accepted", "admin", 2),
            )
            .unwrap();
        assert_eq!(
            workspace.invite_member(
                invitation("owner-invite"),
                user("other"),
                WorkspaceRole::Owner,
                None,
                context("owner-invited", "admin", 3),
            ),
            Err(WorkspaceError::OwnerRequired)
        );
    }

    #[test]
    fn unregistering_a_project_requires_workspace_administration_and_is_audited() {
        let mut workspace = organization();
        let project_id = ProjectId::new("project-to-delete").unwrap();
        workspace
            .register_project(
                project_id.clone(),
                context("project-registered", "owner", 1),
            )
            .unwrap();

        assert_eq!(
            workspace
                .unregister_project(&project_id, context("project-delete-denied", "member", 2)),
            Err(WorkspaceError::PermissionDenied)
        );
        assert!(workspace.projects().contains(&project_id));

        workspace
            .unregister_project(&project_id, context("project-unregistered", "owner", 3))
            .unwrap();
        assert!(!workspace.projects().contains(&project_id));
        assert_eq!(
            workspace.audit_events().last().unwrap().action(),
            &AuditAction::ProjectUnregistered {
                project_id: project_id.clone()
            }
        );
        let audit_count = workspace.audit_events().len();
        assert_eq!(
            workspace
                .unregister_project(&project_id, context("project-delete-missing", "owner", 4)),
            Err(WorkspaceError::ProjectNotFound(project_id))
        );
        assert_eq!(workspace.audit_events().len(), audit_count);
    }

    #[test]
    fn personal_workspaces_reject_invitations() {
        let mut workspace = Workspace::new_personal(
            WorkspaceId::new("personal").unwrap(),
            user("owner"),
            context("create-personal", "owner", 0),
        )
        .unwrap();
        assert_eq!(
            workspace.invite_member(
                invitation("invite"),
                user("other"),
                WorkspaceRole::Member,
                None,
                context("invite-other", "owner", 1),
            ),
            Err(WorkspaceError::PersonalWorkspaceMembershipFixed)
        );
    }
}

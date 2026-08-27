use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{
    CandidateId, Capability, CommentId, EntityId, GuideId, LevelId, ReviewId, RevisionId,
    TimestampMs, UserId,
};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReviewModelError {
    #[error("an approval policy must require at least one approval")]
    ZeroApprovals,
    #[error("a grid region must have positive width and height")]
    EmptyGridRegion,
    #[error("a Blind pixel region must have positive width and height")]
    EmptyBlindPixelRegion,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ApprovalPolicy {
    allow_self_approval: bool,
    min_approvals: u16,
    required_capabilities: BTreeSet<Capability>,
}

#[derive(Deserialize)]
struct SerializedApprovalPolicy {
    allow_self_approval: bool,
    min_approvals: u16,
    required_capabilities: BTreeSet<Capability>,
}

impl<'de> Deserialize<'de> for ApprovalPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let policy = SerializedApprovalPolicy::deserialize(deserializer)?;
        Self::new(
            policy.allow_self_approval,
            policy.min_approvals,
            policy.required_capabilities,
        )
        .map_err(D::Error::custom)
    }
}

impl ApprovalPolicy {
    pub fn new(
        allow_self_approval: bool,
        min_approvals: u16,
        required_capabilities: BTreeSet<Capability>,
    ) -> Result<Self, ReviewModelError> {
        if min_approvals == 0 {
            return Err(ReviewModelError::ZeroApprovals);
        }
        Ok(Self {
            allow_self_approval,
            min_approvals,
            required_capabilities,
        })
    }

    #[must_use]
    pub const fn allow_self_approval(&self) -> bool {
        self.allow_self_approval
    }

    #[must_use]
    pub const fn min_approvals(&self) -> u16 {
        self.min_approvals
    }

    #[must_use]
    pub const fn required_capabilities(&self) -> &BTreeSet<Capability> {
        &self.required_capabilities
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridCell {
    pub x: i32,
    pub y: i32,
}

impl GridCell {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct GridRegion {
    min: GridCell,
    max_exclusive: GridCell,
}

#[derive(Deserialize)]
struct SerializedGridRegion {
    min: GridCell,
    max_exclusive: GridCell,
}

impl<'de> Deserialize<'de> for GridRegion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let region = SerializedGridRegion::deserialize(deserializer)?;
        Self::new(region.min, region.max_exclusive).map_err(D::Error::custom)
    }
}

impl GridRegion {
    pub fn new(min: GridCell, max_exclusive: GridCell) -> Result<Self, ReviewModelError> {
        if max_exclusive.x <= min.x || max_exclusive.y <= min.y {
            return Err(ReviewModelError::EmptyGridRegion);
        }
        Ok(Self { min, max_exclusive })
    }

    #[must_use]
    pub const fn min(&self) -> GridCell {
        self.min
    }

    #[must_use]
    pub const fn max_exclusive(&self) -> GridCell {
        self.max_exclusive
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BlindPixelRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct SerializedBlindPixelRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl<'de> Deserialize<'de> for BlindPixelRegion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let region = SerializedBlindPixelRegion::deserialize(deserializer)?;
        Self::new(region.x, region.y, region.width, region.height).map_err(D::Error::custom)
    }
}

impl BlindPixelRegion {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, ReviewModelError> {
        if width == 0 || height == 0 {
            return Err(ReviewModelError::EmptyBlindPixelRegion);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(&self) -> u32 {
        self.x
    }

    #[must_use]
    pub const fn y(&self) -> u32 {
        self.y
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuideEdge {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideEdgeAnchor {
    guide_id: GuideId,
    edge: GuideEdge,
}

impl GuideEdgeAnchor {
    #[must_use]
    pub const fn new(guide_id: GuideId, edge: GuideEdge) -> Self {
        Self { guide_id, edge }
    }

    #[must_use]
    pub const fn guide_id(&self) -> &GuideId {
        &self.guide_id
    }

    #[must_use]
    pub const fn edge(&self) -> GuideEdge {
        self.edge
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewAnchor {
    Revision {
        revision_id: RevisionId,
    },
    Entity {
        revision_id: RevisionId,
        entity_id: EntityId,
    },
    GridCell {
        revision_id: RevisionId,
        cell: GridCell,
    },
    GridRegion {
        revision_id: RevisionId,
        region: GridRegion,
    },
    BlindPixelRegion {
        revision_id: RevisionId,
        region: BlindPixelRegion,
    },
    GuideEdge {
        revision_id: RevisionId,
        anchor: GuideEdgeAnchor,
    },
}

impl ReviewAnchor {
    #[must_use]
    pub const fn revision(revision_id: RevisionId) -> Self {
        Self::Revision { revision_id }
    }

    #[must_use]
    pub const fn entity(revision_id: RevisionId, entity_id: EntityId) -> Self {
        Self::Entity {
            revision_id,
            entity_id,
        }
    }

    #[must_use]
    pub const fn grid_cell(revision_id: RevisionId, cell: GridCell) -> Self {
        Self::GridCell { revision_id, cell }
    }

    #[must_use]
    pub const fn grid_region(revision_id: RevisionId, region: GridRegion) -> Self {
        Self::GridRegion {
            revision_id,
            region,
        }
    }

    #[must_use]
    pub const fn blind_pixel_region(revision_id: RevisionId, region: BlindPixelRegion) -> Self {
        Self::BlindPixelRegion {
            revision_id,
            region,
        }
    }

    #[must_use]
    pub const fn guide_edge(revision_id: RevisionId, anchor: GuideEdgeAnchor) -> Self {
        Self::GuideEdge {
            revision_id,
            anchor,
        }
    }

    #[must_use]
    pub const fn revision_id(&self) -> &RevisionId {
        match self {
            Self::Revision { revision_id }
            | Self::Entity { revision_id, .. }
            | Self::GridCell { revision_id, .. }
            | Self::GridRegion { revision_id, .. }
            | Self::BlindPixelRegion { revision_id, .. }
            | Self::GuideEdge { revision_id, .. } => revision_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Approve,
    RequestChanges,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewState {
    Open,
    ChangesRequested,
    Approved,
    Rejected,
    Withdrawn,
}

impl ReviewState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Approved | Self::Rejected | Self::Withdrawn)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRecord {
    id: ReviewId,
    reviewer: UserId,
    decision: ReviewDecision,
    occurred_at: TimestampMs,
    validated_capabilities: BTreeSet<Capability>,
}

impl ReviewRecord {
    pub(crate) fn new(
        id: ReviewId,
        reviewer: UserId,
        decision: ReviewDecision,
        occurred_at: TimestampMs,
        validated_capabilities: BTreeSet<Capability>,
    ) -> Self {
        Self {
            id,
            reviewer,
            decision,
            occurred_at,
            validated_capabilities,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &ReviewId {
        &self.id
    }

    #[must_use]
    pub const fn reviewer(&self) -> &UserId {
        &self.reviewer
    }

    #[must_use]
    pub const fn decision(&self) -> ReviewDecision {
        self.decision
    }

    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMs {
        self.occurred_at
    }

    #[must_use]
    pub const fn validated_capabilities(&self) -> &BTreeSet<Capability> {
        &self.validated_capabilities
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    id: CommentId,
    author: UserId,
    body: String,
    anchor: ReviewAnchor,
    occurred_at: TimestampMs,
}

impl ReviewComment {
    pub(crate) fn new(
        id: CommentId,
        author: UserId,
        body: String,
        anchor: ReviewAnchor,
        occurred_at: TimestampMs,
    ) -> Self {
        Self {
            id,
            author,
            body,
            anchor,
            occurred_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &CommentId {
        &self.id
    }

    #[must_use]
    pub const fn author(&self) -> &UserId {
        &self.author
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    #[must_use]
    pub const fn anchor(&self) -> &ReviewAnchor {
        &self.anchor
    }

    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMs {
        self.occurred_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewCandidate {
    id: CandidateId,
    level_id: LevelId,
    revision_id: RevisionId,
    created_by: UserId,
    created_at: TimestampMs,
    approval_policy: ApprovalPolicy,
    state: ReviewState,
    reviews: Vec<ReviewRecord>,
    comments: Vec<ReviewComment>,
}

impl ReviewCandidate {
    pub(crate) fn new(
        id: CandidateId,
        level_id: LevelId,
        revision_id: RevisionId,
        created_by: UserId,
        created_at: TimestampMs,
        approval_policy: ApprovalPolicy,
    ) -> Self {
        Self {
            id,
            level_id,
            revision_id,
            created_by,
            created_at,
            approval_policy,
            state: ReviewState::Open,
            reviews: Vec::new(),
            comments: Vec::new(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> &CandidateId {
        &self.id
    }

    #[must_use]
    pub const fn level_id(&self) -> &LevelId {
        &self.level_id
    }

    #[must_use]
    pub const fn revision_id(&self) -> &RevisionId {
        &self.revision_id
    }

    #[must_use]
    pub const fn created_by(&self) -> &UserId {
        &self.created_by
    }

    #[must_use]
    pub const fn created_at(&self) -> TimestampMs {
        self.created_at
    }

    #[must_use]
    pub const fn approval_policy(&self) -> &ApprovalPolicy {
        &self.approval_policy
    }

    #[must_use]
    pub const fn state(&self) -> ReviewState {
        self.state
    }

    #[must_use]
    pub fn reviews(&self) -> &[ReviewRecord] {
        &self.reviews
    }

    #[must_use]
    pub fn comments(&self) -> &[ReviewComment] {
        &self.comments
    }

    pub(crate) fn set_state(&mut self, state: ReviewState) {
        self.state = state;
    }

    pub(crate) fn push_review(&mut self, review: ReviewRecord) {
        self.reviews.push(review);
    }

    pub(crate) fn push_comment(&mut self, comment: ReviewComment) {
        self.comments.push(comment);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalStatus {
    pub approvals: u16,
    pub required: u16,
    pub approving_users: BTreeSet<UserId>,
    pub satisfied: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_regions_reject_empty_shapes() {
        let cell = GridCell::new(2, 3);
        assert_eq!(
            GridRegion::new(cell, cell),
            Err(ReviewModelError::EmptyGridRegion)
        );
        assert_eq!(
            BlindPixelRegion::new(0, 0, 0, 4),
            Err(ReviewModelError::EmptyBlindPixelRegion)
        );
    }
}

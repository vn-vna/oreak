use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

use crate::{
    ContributionChannel, ContributionKind, Day, FormulaVersion, TimestampMs, WorkflowState,
};

pub const MAX_ACTIVITY_DAYS: u64 = 3_660;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum KpiError {
    #[error("a KPI formula must define at least one positive event weight")]
    EmptyWeights,
    #[error("event weights must be positive")]
    ZeroWeight,
    #[error("a daily cap must be positive")]
    ZeroDailyCap,
    #[error("a KPI formula must include at least one contribution channel")]
    EmptyChannels,
    #[error("a KPI formula must include at least one workflow state")]
    EmptyWorkflowStates,
    #[error("activity range ends before it starts")]
    ReversedRange,
    #[error("activity range exceeds {MAX_ACTIVITY_DAYS} days")]
    RangeTooLarge,
    #[error("activity day range overflowed")]
    DayOverflow,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KpiFormula {
    weights: BTreeMap<ContributionKind, u32>,
    daily_cap: Option<u32>,
    included_channels: BTreeSet<ContributionChannel>,
    included_workflow_states: BTreeSet<WorkflowState>,
}

#[derive(Deserialize)]
struct SerializedKpiFormula {
    weights: BTreeMap<ContributionKind, u32>,
    daily_cap: Option<u32>,
    included_channels: BTreeSet<ContributionChannel>,
    included_workflow_states: BTreeSet<WorkflowState>,
}

impl<'de> Deserialize<'de> for KpiFormula {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let formula = SerializedKpiFormula::deserialize(deserializer)?;
        Self::new(
            formula.weights,
            formula.daily_cap,
            formula.included_channels,
            formula.included_workflow_states,
        )
        .map_err(D::Error::custom)
    }
}

impl KpiFormula {
    pub fn new(
        weights: BTreeMap<ContributionKind, u32>,
        daily_cap: Option<u32>,
        included_channels: BTreeSet<ContributionChannel>,
        included_workflow_states: BTreeSet<WorkflowState>,
    ) -> Result<Self, KpiError> {
        if weights.is_empty() {
            return Err(KpiError::EmptyWeights);
        }
        if weights.values().any(|weight| *weight == 0) {
            return Err(KpiError::ZeroWeight);
        }
        if daily_cap == Some(0) {
            return Err(KpiError::ZeroDailyCap);
        }
        if included_channels.is_empty() {
            return Err(KpiError::EmptyChannels);
        }
        if included_workflow_states.is_empty() {
            return Err(KpiError::EmptyWorkflowStates);
        }
        Ok(Self {
            weights,
            daily_cap,
            included_channels,
            included_workflow_states,
        })
    }

    #[must_use]
    pub const fn weights(&self) -> &BTreeMap<ContributionKind, u32> {
        &self.weights
    }

    #[must_use]
    pub const fn daily_cap(&self) -> Option<u32> {
        self.daily_cap
    }

    #[must_use]
    pub const fn included_channels(&self) -> &BTreeSet<ContributionChannel> {
        &self.included_channels
    }

    #[must_use]
    pub const fn included_workflow_states(&self) -> &BTreeSet<WorkflowState> {
        &self.included_workflow_states
    }

    #[must_use]
    pub fn score(
        &self,
        kind: &ContributionKind,
        channel: &ContributionChannel,
        workflow_state: &WorkflowState,
    ) -> u32 {
        if !self.included_channels.contains(channel)
            || !self.included_workflow_states.contains(workflow_state)
        {
            return 0;
        }
        self.weights.get(kind).copied().unwrap_or(0)
    }
}

impl Default for KpiFormula {
    fn default() -> Self {
        Self {
            weights: [
                (ContributionKind::TimelineRevision, 4),
                (ContributionKind::ReviewCandidate, 3),
                (ContributionKind::ReviewComment, 1),
                (ContributionKind::ReviewDecision, 2),
                (ContributionKind::CandidateStateChanged, 1),
                (ContributionKind::ArtifactProduced, 5),
                (ContributionKind::ReleasePromotion, 3),
            ]
            .into_iter()
            .collect(),
            daily_cap: Some(20),
            included_channels: [
                ContributionChannel::Timeline,
                ContributionChannel::Review,
                ContributionChannel::Artifact,
            ]
            .into_iter()
            .collect(),
            included_workflow_states: [
                WorkflowState::Active,
                WorkflowState::InReview,
                WorkflowState::ChangesRequested,
                WorkflowState::Approved,
                WorkflowState::Rejected,
                WorkflowState::Withdrawn,
                WorkflowState::Released,
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KpiPolicy {
    version: FormulaVersion,
    effective_from: Day,
    created_at: TimestampMs,
    formula: KpiFormula,
}

impl KpiPolicy {
    #[must_use]
    pub const fn new(
        version: FormulaVersion,
        effective_from: Day,
        created_at: TimestampMs,
        formula: KpiFormula,
    ) -> Self {
        Self {
            version,
            effective_from,
            created_at,
            formula,
        }
    }

    #[must_use]
    pub const fn version(&self) -> &FormulaVersion {
        &self.version
    }

    #[must_use]
    pub const fn effective_from(&self) -> Day {
        self.effective_from
    }

    #[must_use]
    pub const fn created_at(&self) -> TimestampMs {
        self.created_at
    }

    #[must_use]
    pub const fn formula(&self) -> &KpiFormula {
        &self.formula
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyActivity {
    pub day: Day,
    pub score: u32,
    pub formula_version: Option<FormulaVersion>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_formula_scores_semantic_events_not_custom_raw_counts() {
        let formula = KpiFormula::default();
        assert_eq!(
            formula.score(
                &ContributionKind::TimelineRevision,
                &ContributionChannel::Timeline,
                &WorkflowState::Active
            ),
            4
        );
        assert_eq!(
            formula.score(
                &ContributionKind::Custom("pointer-move".into()),
                &ContributionChannel::Timeline,
                &WorkflowState::Active
            ),
            0
        );
    }
}

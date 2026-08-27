use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;

const MAX_ID_BYTES: usize = 256;
const MILLIS_PER_DAY: i64 = 86_400_000;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum IdError {
    #[error("an ID cannot be empty or whitespace-only")]
    Empty,
    #[error("an ID cannot exceed {MAX_ID_BYTES} bytes")]
    TooLong,
    #[error("an ID cannot contain control characters")]
    ControlCharacter,
}

fn validate_id(value: &str) -> Result<(), IdError> {
    if value.trim().is_empty() {
        return Err(IdError::Empty);
    }
    if value.len() > MAX_ID_BYTES {
        return Err(IdError::TooLong);
    }
    if value.chars().any(char::is_control) {
        return Err(IdError::ControlCharacter);
    }
    Ok(())
}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_id(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(D::Error::custom)
            }
        }
    };
}

string_id!(UserId);
string_id!(WorkspaceId);
string_id!(ProjectId);
string_id!(InvitationId);
string_id!(RoleId);
string_id!(LevelId);
string_id!(RevisionId);
string_id!(CandidateId);
string_id!(ReviewId);
string_id!(CommentId);
string_id!(ArtifactId);
string_id!(ArtifactDigest);
string_id!(ReleaseChannelId);
string_id!(PromotionId);
string_id!(PluginId);
string_id!(ThemeId);
string_id!(AuditEventId);
string_id!(ContributionEventId);
string_id!(FormulaVersion);
string_id!(CertificationId);
string_id!(EntityId);
string_id!(GuideId);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct TimestampMs(i64);

impl TimestampMs {
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_i64(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn day(self) -> Day {
        Day(self.0.div_euclid(MILLIS_PER_DAY))
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct Day(i64);

impl Day {
    #[must_use]
    pub const fn new(unix_day: i64) -> Self {
        Self(unix_day)
    }

    #[must_use]
    pub const fn unix_day(self) -> i64 {
        self.0
    }

    pub(crate) const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_invalid_values() {
        assert_eq!(UserId::new("   "), Err(IdError::Empty));
        assert_eq!(UserId::new("bad\nvalue"), Err(IdError::ControlCharacter));
        assert_eq!(UserId::new("x".repeat(257)), Err(IdError::TooLong));
    }

    #[test]
    fn timestamps_use_euclidean_days() {
        assert_eq!(TimestampMs::new(0).day(), Day::new(0));
        assert_eq!(TimestampMs::new(86_399_999).day(), Day::new(0));
        assert_eq!(TimestampMs::new(-1).day(), Day::new(-1));
    }
}

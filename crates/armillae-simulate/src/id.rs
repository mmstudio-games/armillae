use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentifierKind {
    Module,
    ExecuteEntry,
    ClockType,
    ClockInstance,
    ClockErrorCode,
    SystemErrorCode,
    System,
    Backend,
    Capability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidIdentifierReason {
    Empty,
    TooLong { max_bytes: usize },
    NonGraphicAscii,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid {kind:?} identifier: {reason:?}")]
pub struct InvalidIdentifier {
    pub kind: IdentifierKind,
    pub reason: InvalidIdentifierReason,
}

fn validate_identifier(value: &str, kind: IdentifierKind) -> Result<(), InvalidIdentifier> {
    let reason = if value.is_empty() {
        Some(InvalidIdentifierReason::Empty)
    } else if value.len() > 255 {
        Some(InvalidIdentifierReason::TooLong { max_bytes: 255 })
    } else if !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
        Some(InvalidIdentifierReason::NonGraphicAscii)
    } else {
        None
    };

    match reason {
        Some(reason) => Err(InvalidIdentifier { kind, reason }),
        None => Ok(()),
    }
}

macro_rules! define_identifier {
    ($name:ident, $kind:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
        #[serde(transparent)]
        #[schemars(transparent)]
        pub struct $name(
            #[schemars(length(min = 1, max = 255), regex(pattern = r"^[!-~]+$"))] String,
        );

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidIdentifier> {
                let value = value.into();
                validate_identifier(&value, $kind)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = InvalidIdentifier;

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
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

define_identifier!(ModuleId, IdentifierKind::Module);
define_identifier!(ExecuteEntryId, IdentifierKind::ExecuteEntry);
define_identifier!(ClockTypeId, IdentifierKind::ClockType);
define_identifier!(ClockInstanceId, IdentifierKind::ClockInstance);
define_identifier!(ClockErrorCode, IdentifierKind::ClockErrorCode);
define_identifier!(SystemErrorCode, IdentifierKind::SystemErrorCode);
define_identifier!(SystemId, IdentifierKind::System);
define_identifier!(BackendId, IdentifierKind::Backend);
define_identifier!(CapabilityId, IdentifierKind::Capability);

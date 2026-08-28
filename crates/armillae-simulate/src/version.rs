use std::{
    borrow::Cow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionKind {
    SemanticVersion,
    Requirement,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid {kind:?}: {message}")]
pub struct InvalidVersion {
    pub kind: VersionKind,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct SemanticVersion {
    canonical: String,
    parsed: semver::Version,
}

impl SemanticVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidVersion> {
        let value = value.into();
        let parsed = semver::Version::parse(&value).map_err(|error| InvalidVersion {
            kind: VersionKind::SemanticVersion,
            message: error.to_string(),
        })?;
        Ok(Self {
            canonical: parsed.to_string(),
            parsed,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }
}

#[derive(Clone, Debug)]
pub struct VersionRequirement {
    canonical: String,
    parsed: semver::VersionReq,
}

impl VersionRequirement {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidVersion> {
        let value = value.into();
        let parsed = semver::VersionReq::parse(&value).map_err(|error| InvalidVersion {
            kind: VersionKind::Requirement,
            message: error.to_string(),
        })?;
        Ok(Self {
            canonical: parsed.to_string(),
            parsed,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub fn matches(&self, version: &SemanticVersion) -> bool {
        self.parsed.matches(&version.parsed)
    }
}

macro_rules! impl_version_value {
    ($name:ident, $schema_name:literal, $schema_id:literal, $schema:expr) => {
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.canonical == other.canonical
            }
        }

        impl Eq for $name {}

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $name {
            fn cmp(&self, other: &Self) -> Ordering {
                self.canonical.cmp(&other.canonical)
            }
        }

        impl Hash for $name {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.canonical.hash(state);
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
            type Err = InvalidVersion;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
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

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                $schema_name.into()
            }

            fn schema_id() -> Cow<'static, str> {
                $schema_id.into()
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                $schema
            }
        }
    };
}

impl_version_value!(
    SemanticVersion,
    "SemanticVersion",
    "armillae_simulate::SemanticVersion",
    json_schema!({
        "type": "string",
        "pattern": r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"
    })
);

impl_version_value!(
    VersionRequirement,
    "VersionRequirement",
    "armillae_simulate::VersionRequirement",
    json_schema!({ "type": "string" })
);

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ClockErrorCode, ClockInstanceId, ClockTypeId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("clock transition rejected: {code}: {message}")]
pub struct ClockTransitionError {
    pub code: ClockErrorCode,
    pub message: String,
}

pub trait Clock: Clone + Send + Sync + Serialize + DeserializeOwned + JsonSchema + 'static {
    type Step: Clone + Send + Sync + Serialize + DeserializeOwned + JsonSchema + 'static;

    fn validate(&self) -> Result<(), ClockTransitionError> {
        Ok(())
    }

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedAdvanceTarget<S> {
    pub instance: ClockInstanceId,
    pub step: S,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedAdvanceRequest<S> {
    pub targets: Vec<TypedAdvanceTarget<S>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedClockTransition<C: Clock> {
    pub instance: ClockInstanceId,
    pub before: C,
    pub step: C::Step,
    pub after: C,
}

#[derive(Clone)]
pub struct TypedAdvanceOutcome<C: Clock> {
    pub clock_type: ClockTypeId,
    pub transitions: Vec<TypedClockTransition<C>>,
}

impl<C> std::fmt::Debug for TypedAdvanceOutcome<C>
where
    C: Clock + std::fmt::Debug,
    C::Step: std::fmt::Debug,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TypedAdvanceOutcome")
            .field("clock_type", &self.clock_type)
            .field("transitions", &self.transitions)
            .finish()
    }
}

impl<C> PartialEq for TypedAdvanceOutcome<C>
where
    C: Clock + PartialEq,
    C::Step: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.clock_type == other.clock_type && self.transitions == other.transitions
    }
}

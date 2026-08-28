use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AdvanceOutcome, AdvanceRequest, ClockKey, ClockState, ExecuteOutcome, ExecuteRequest,
    SimulationCapabilities, SimulationError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SimulationStatus {
    Active,
    Stopped,
    Faulted,
}

pub trait Simulation: Send {
    fn status(&self) -> SimulationStatus;

    fn capabilities(&self) -> SimulationCapabilities;

    fn execute(&mut self, request: ExecuteRequest) -> Result<ExecuteOutcome, SimulationError>;

    fn read_clock(&self, key: &ClockKey) -> Result<ClockState, SimulationError>;

    fn insert_clock(&mut self, state: ClockState) -> Result<(), SimulationError>;

    fn remove_clock(&mut self, key: &ClockKey) -> Result<ClockState, SimulationError>;

    fn advance(&mut self, request: AdvanceRequest) -> Result<AdvanceOutcome, SimulationError>;

    fn stop(&mut self) -> Result<(), SimulationError>;
}

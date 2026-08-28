use std::{collections::VecDeque, sync::Mutex};

use serde_json::json;

use crate::*;

#[derive(Clone, Debug)]
pub struct ContractFixture {
    pub module: ModuleDescriptor,
    pub execute_request: ExecuteRequest,
    pub primary_clock: ClockState,
    pub secondary_clock: ClockState,
    pub probe_clock: ClockState,
    pub advance_request: AdvanceRequest,
}

fn module_id(value: &str) -> ModuleId {
    ModuleId::new(value).expect("hard-coded contract module ID is valid visible ASCII")
}

fn execute_id(value: &str) -> ExecuteEntryId {
    ExecuteEntryId::new(value).expect("hard-coded contract execute ID is valid visible ASCII")
}

fn clock_type_id(value: &str) -> ClockTypeId {
    ClockTypeId::new(value).expect("hard-coded contract clock type ID is valid visible ASCII")
}

fn clock_instance_id(value: &str) -> ClockInstanceId {
    ClockInstanceId::new(value)
        .expect("hard-coded contract clock instance ID is valid visible ASCII")
}

fn system_id(value: &str) -> SystemId {
    SystemId::new(value).expect("hard-coded contract system ID is valid visible ASCII")
}

pub fn standard_fixture(execution: ExecutionPlane) -> ContractFixture {
    let execute_entry = execute_id("armillae.simulate.contract/increment_probe");
    let counter_type = clock_type_id("armillae.simulate.contract/counter");
    let probe_type = clock_type_id("armillae.simulate.contract/probe");
    let primary = clock_instance_id("primary");
    let secondary = clock_instance_id("secondary");
    let probe = clock_instance_id("probe");
    let value_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "value": { "type": "integer" } },
        "required": ["value"]
    });
    let delta_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "delta": { "type": "integer" } },
        "required": ["delta"]
    });

    ContractFixture {
        module: ModuleDescriptor {
            api_version: SIMULATE_API_VERSION.to_owned(),
            id: module_id("armillae.simulate.contract/fixture"),
            version: SemanticVersion::new("1.0.0")
                .expect("hard-coded contract semantic version is valid"),
            dependencies: Vec::new(),
            execution,
            required_capabilities: Default::default(),
            execute_entries: vec![ExecuteEntryDefinition {
                id: execute_entry.clone(),
                input_schema: delta_schema.clone(),
                output_schema: Some(value_schema.clone()),
            }],
            clocks: vec![
                ClockDefinition {
                    id: counter_type.clone(),
                    value_schema: value_schema.clone(),
                    step_schema: delta_schema.clone(),
                },
                ClockDefinition {
                    id: probe_type.clone(),
                    value_schema,
                    step_schema: delta_schema,
                },
            ],
            systems: vec![
                SystemDefinition {
                    id: system_id("armillae.simulate.contract/system/increment_probe"),
                    trigger: SystemTrigger::Execute {
                        entry: execute_entry.clone(),
                    },
                    before: Vec::new(),
                    after: Vec::new(),
                },
                SystemDefinition {
                    id: system_id("armillae.simulate.contract/system/count_advance"),
                    trigger: SystemTrigger::Advance {
                        clock_type: counter_type.clone(),
                    },
                    before: Vec::new(),
                    after: Vec::new(),
                },
            ],
        },
        execute_request: ExecuteRequest {
            entry: execute_entry,
            input: json!({ "delta": 4 }),
        },
        primary_clock: ClockState {
            key: ClockKey {
                clock_type: counter_type.clone(),
                instance: primary.clone(),
            },
            value: json!({ "value": 0 }),
        },
        secondary_clock: ClockState {
            key: ClockKey {
                clock_type: counter_type.clone(),
                instance: secondary.clone(),
            },
            value: json!({ "value": 10 }),
        },
        probe_clock: ClockState {
            key: ClockKey {
                clock_type: probe_type,
                instance: probe,
            },
            value: json!({ "value": 0 }),
        },
        advance_request: AdvanceRequest {
            clock_type: counter_type,
            targets: vec![
                AdvanceTarget {
                    instance: primary,
                    step: json!({ "delta": 2 }),
                },
                AdvanceTarget {
                    instance: secondary,
                    step: json!({ "delta": 3 }),
                },
            ],
        },
    }
}

pub trait BackendContractFactory: Send + Sync {
    fn capabilities(&self) -> SimulationCapabilities;
    fn execution_plane(&self) -> ExecutionPlane;
    fn create_fixture(
        &self,
        fixture: ContractFixture,
    ) -> Result<Box<dyn Simulation>, ContractSetupError>;
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("contract setup failed: {code}: {message}")]
pub struct ContractSetupError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("backend contract `{case}` failed: {message}")]
pub struct ContractViolation {
    pub case: String,
    pub message: String,
}

fn violation(case: &str, message: impl Into<String>) -> ContractViolation {
    ContractViolation {
        case: case.to_owned(),
        message: message.into(),
    }
}

fn fixture_simulation(
    factory: &dyn BackendContractFactory,
    case: &str,
) -> Result<(ContractFixture, Box<dyn Simulation>), ContractViolation> {
    let fixture = standard_fixture(factory.execution_plane());
    let simulation = factory
        .create_fixture(fixture.clone())
        .map_err(|error| violation(case, error.to_string()))?;
    if simulation.capabilities() != factory.capabilities() {
        return Err(violation(
            case,
            "factory and simulation capabilities differ",
        ));
    }
    Ok((fixture, simulation))
}

pub fn assert_backend_runtime_contract(
    factory: &dyn BackendContractFactory,
) -> Result<(), ContractViolation> {
    let (fixture, mut simulation) = fixture_simulation(factory, "execute")?;
    let outcome = simulation
        .execute(fixture.execute_request.clone())
        .map_err(|error| violation("execute", error.to_string()))?;
    if outcome.entry != fixture.execute_request.entry
        || outcome.output != Some(json!({ "value": 4 }))
    {
        return Err(violation("execute", "unexpected execute outcome"));
    }
    let primary = simulation
        .read_clock(&fixture.primary_clock.key)
        .map_err(|error| violation("execute_no_advance", error.to_string()))?;
    if primary != fixture.primary_clock {
        return Err(violation("execute_no_advance", "execute changed a clock"));
    }

    let (fixture, mut simulation) = fixture_simulation(factory, "invalid_input")?;
    let invalid = ExecuteRequest {
        entry: fixture.execute_request.entry,
        input: json!({ "delta": "invalid" }),
    };
    if !matches!(
        simulation.execute(invalid),
        Err(SimulationError::InvalidExecuteInput { .. })
    ) {
        return Err(violation("invalid_input", "invalid input was not rejected"));
    }
    if simulation.status() != SimulationStatus::Active {
        return Err(violation(
            "invalid_input",
            "validation faulted the simulation",
        ));
    }

    let (fixture, mut simulation) = fixture_simulation(factory, "advance")?;
    let outcome = simulation
        .advance(fixture.advance_request.clone())
        .map_err(|error| violation("advance", error.to_string()))?;
    let instances: Vec<_> = outcome
        .transitions
        .iter()
        .map(|transition| transition.instance.clone())
        .collect();
    let expected_instances: Vec<_> = fixture
        .advance_request
        .targets
        .iter()
        .map(|target| target.instance.clone())
        .collect();
    if instances != expected_instances {
        return Err(violation(
            "advance",
            "transition order differs from request",
        ));
    }
    let primary = simulation
        .read_clock(&fixture.primary_clock.key)
        .map_err(|error| violation("advance", error.to_string()))?;
    let secondary = simulation
        .read_clock(&fixture.secondary_clock.key)
        .map_err(|error| violation("advance", error.to_string()))?;
    if primary.value != json!({ "value": 2 }) || secondary.value != json!({ "value": 13 }) {
        return Err(violation(
            "advance",
            "clock instances were not advanced independently",
        ));
    }
    let probe = simulation
        .read_clock(&fixture.probe_clock.key)
        .map_err(|error| violation("advance_response", error.to_string()))?;
    if probe.value != json!({ "value": 1 }) {
        return Err(violation(
            "advance_response",
            "advance response system did not run once",
        ));
    }

    let (fixture, mut simulation) = fixture_simulation(factory, "stop")?;
    simulation
        .stop()
        .map_err(|error| violation("stop", error.to_string()))?;
    simulation
        .stop()
        .map_err(|error| violation("stop", error.to_string()))?;
    if simulation.status() != SimulationStatus::Stopped {
        return Err(violation("stop", "stop did not enter stopped state"));
    }
    simulation
        .read_clock(&fixture.primary_clock.key)
        .map_err(|error| violation("stopped_read", error.to_string()))?;
    if !matches!(
        simulation.advance(fixture.advance_request),
        Err(SimulationError::InvalidState {
            status: SimulationStatus::Stopped,
            ..
        })
    ) {
        return Err(violation(
            "stopped_write",
            "stopped simulation accepted a write",
        ));
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RecordedSimulationCall {
    Execute(ExecuteRequest),
    ReadClock(ClockKey),
    InsertClock(ClockState),
    RemoveClock(ClockKey),
    Advance(AdvanceRequest),
    Stop,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ScriptedReply {
    Execute(Result<ExecuteOutcome, SimulationError>),
    ReadClock(Result<ClockState, SimulationError>),
    InsertClock(Result<(), SimulationError>),
    RemoveClock(Result<ClockState, SimulationError>),
    Advance(Result<AdvanceOutcome, SimulationError>),
}

struct ScriptState {
    status: SimulationStatus,
    replies: VecDeque<ScriptedReply>,
    calls: Vec<RecordedSimulationCall>,
}

pub struct ScriptedSimulation {
    capabilities: SimulationCapabilities,
    state: Mutex<ScriptState>,
}

impl ScriptedSimulation {
    pub fn new(
        capabilities: SimulationCapabilities,
        replies: impl IntoIterator<Item = ScriptedReply>,
    ) -> Self {
        Self {
            capabilities,
            state: Mutex::new(ScriptState {
                status: SimulationStatus::Active,
                replies: replies.into_iter().collect(),
                calls: Vec::new(),
            }),
        }
    }

    pub fn calls(&self) -> Vec<RecordedSimulationCall> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .calls
            .clone()
    }

    fn mismatch<T>(
        &self,
        state: &mut ScriptState,
        operation: SimulationOperation,
    ) -> Result<T, SimulationError> {
        state.status = SimulationStatus::Faulted;
        Err(SimulationError::BackendFailure {
            backend: self.capabilities.backend.id.clone(),
            operation,
            code: "armillae.simulate/mock_script_mismatch".to_owned(),
            message: "scripted simulation reply did not match the call".to_owned(),
        })
    }

    fn reject<T>(
        status: SimulationStatus,
        operation: SimulationOperation,
    ) -> Result<T, SimulationError> {
        match status {
            SimulationStatus::Faulted => Err(SimulationError::Faulted { operation }),
            SimulationStatus::Stopped | SimulationStatus::Active => {
                Err(SimulationError::InvalidState { operation, status })
            }
        }
    }

    fn update_status<T>(state: &mut ScriptState, result: &Result<T, SimulationError>) {
        if result
            .as_ref()
            .err()
            .is_some_and(SimulationError::faults_simulation)
        {
            state.status = SimulationStatus::Faulted;
        }
    }
}

macro_rules! scripted_method {
    ($method:ident, $argument:ident : $argument_type:ty, $output:ty, $call:ident, $reply:ident, $operation:ident) => {
        fn $method(&mut self, $argument: $argument_type) -> Result<$output, SimulationError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.calls.push(RecordedSimulationCall::$call($argument));
            if state.status != SimulationStatus::Active {
                return Self::reject(state.status, SimulationOperation::$operation);
            }
            if !matches!(state.replies.front(), Some(ScriptedReply::$reply(_))) {
                return self.mismatch(&mut state, SimulationOperation::$operation);
            }
            let result = match state.replies.pop_front() {
                Some(ScriptedReply::$reply(result)) => result,
                _ => return self.mismatch(&mut state, SimulationOperation::$operation),
            };
            Self::update_status(&mut state, &result);
            result
        }
    };
}

impl Simulation for ScriptedSimulation {
    fn status(&self) -> SimulationStatus {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .status
    }

    fn capabilities(&self) -> SimulationCapabilities {
        self.capabilities.clone()
    }

    scripted_method!(
        execute,
        request: ExecuteRequest,
        ExecuteOutcome,
        Execute,
        Execute,
        Execute
    );

    fn read_clock(&self, key: &ClockKey) -> Result<ClockState, SimulationError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .calls
            .push(RecordedSimulationCall::ReadClock(key.clone()));
        if state.status == SimulationStatus::Faulted {
            return Self::reject(state.status, SimulationOperation::ReadClock);
        }
        if !matches!(state.replies.front(), Some(ScriptedReply::ReadClock(_))) {
            return self.mismatch(&mut state, SimulationOperation::ReadClock);
        }
        let result = match state.replies.pop_front() {
            Some(ScriptedReply::ReadClock(result)) => result,
            _ => return self.mismatch(&mut state, SimulationOperation::ReadClock),
        };
        Self::update_status(&mut state, &result);
        result
    }

    scripted_method!(
        insert_clock,
        state_value: ClockState,
        (),
        InsertClock,
        InsertClock,
        InsertClock
    );

    fn remove_clock(&mut self, key: &ClockKey) -> Result<ClockState, SimulationError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .calls
            .push(RecordedSimulationCall::RemoveClock(key.clone()));
        if state.status != SimulationStatus::Active {
            return Self::reject(state.status, SimulationOperation::RemoveClock);
        }
        if !matches!(state.replies.front(), Some(ScriptedReply::RemoveClock(_))) {
            return self.mismatch(&mut state, SimulationOperation::RemoveClock);
        }
        let result = match state.replies.pop_front() {
            Some(ScriptedReply::RemoveClock(result)) => result,
            _ => return self.mismatch(&mut state, SimulationOperation::RemoveClock),
        };
        Self::update_status(&mut state, &result);
        result
    }

    scripted_method!(
        advance,
        request: AdvanceRequest,
        AdvanceOutcome,
        Advance,
        Advance,
        Advance
    );

    fn stop(&mut self) -> Result<(), SimulationError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.calls.push(RecordedSimulationCall::Stop);
        match state.status {
            SimulationStatus::Active => {
                state.status = SimulationStatus::Stopped;
                Ok(())
            }
            SimulationStatus::Stopped => Ok(()),
            SimulationStatus::Faulted => Self::reject(state.status, SimulationOperation::Stop),
        }
    }
}

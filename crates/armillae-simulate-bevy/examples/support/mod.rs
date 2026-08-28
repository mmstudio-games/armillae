#![allow(dead_code, clippy::result_large_err)]

use armillae_simulate::{
    BackendId, Clock, ClockDefinition, ClockErrorCode, ClockTransitionError, ClockTypeId,
    ExecuteEntryDefinition, ExecuteEntryId, ExecutionPlane, ModuleDescriptor, ModuleId,
    SIMULATE_API_VERSION, SemanticVersion, SimulationBuildError, SystemDefinition, SystemErrorCode,
    SystemExecutionError, SystemExecutionResult, SystemId, SystemTrigger, VersionRequirement,
};
use armillae_simulate_bevy::{
    AdvanceContext, BEVY_BACKEND_ID, BevyModule, BevyModuleRegistrar, BevySimulation,
    BevySimulationBuilder, ExecuteContext,
};
use bevy_ecs::prelude::{Res, ResMut, Resource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const ACTION_ENTRY: &str = "armillae.example.simulate/add_action";
pub const CLOCK_TYPE: &str = "armillae.example.simulate/counter";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CounterClock {
    pub value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CounterStep {
    pub delta: i64,
}

impl Clock for CounterClock {
    type Step = CounterStep;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        self.value
            .checked_add(step.delta)
            .map(|value| Self { value })
            .ok_or_else(|| ClockTransitionError {
                code: ClockErrorCode::new("armillae.example.simulate/overflow")
                    .expect("hard-coded example clock error code is valid"),
                message: "counter overflow".to_owned(),
            })
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ActionInput {
    pub delta: i64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ActionOutput {
    pub total: i64,
}

#[derive(Resource, Default)]
pub struct ActionTotal(pub i64);

#[derive(Resource, Default)]
pub struct AdvanceBatches(pub usize);

fn module_id() -> ModuleId {
    ModuleId::new("armillae.example.simulate/module")
        .expect("hard-coded example module ID is valid")
}

pub fn action_entry_id() -> ExecuteEntryId {
    ExecuteEntryId::new(ACTION_ENTRY).expect("hard-coded example execute entry ID is valid")
}

pub fn clock_type_id() -> ClockTypeId {
    ClockTypeId::new(CLOCK_TYPE).expect("hard-coded example clock type ID is valid")
}

fn action_system_id() -> SystemId {
    SystemId::new("armillae.example.simulate/system/add_action")
        .expect("hard-coded example action system ID is valid")
}

fn advance_system_id() -> SystemId {
    SystemId::new("armillae.example.simulate/system/observe_advance")
        .expect("hard-coded example advance system ID is valid")
}

fn native_plane() -> ExecutionPlane {
    ExecutionPlane::Native {
        backend: BackendId::new(BEVY_BACKEND_ID).expect("hard-coded Bevy backend ID is valid"),
        adapter: VersionRequirement::new(format!("={}", env!("CARGO_PKG_VERSION")))
            .expect("example adapter version requirement is valid"),
    }
}

fn system_error(code: &str, message: &str) -> SystemExecutionError {
    SystemExecutionError {
        code: SystemErrorCode::new(code).expect("hard-coded example system error code is valid"),
        message: message.to_owned(),
    }
}

fn add_action(
    context: Res<ExecuteContext>,
    mut total: ResMut<ActionTotal>,
) -> SystemExecutionResult {
    let input: ActionInput = context.decode().map_err(|_| {
        system_error(
            "armillae.example.simulate/decode",
            "action input decode failed",
        )
    })?;
    total.0 = total.0.checked_add(input.delta).ok_or_else(|| {
        system_error(
            "armillae.example.simulate/overflow",
            "action total overflow",
        )
    })?;
    context
        .set_output(&ActionOutput { total: total.0 })
        .map_err(Into::into)
}

fn observe_advance(
    context: Res<AdvanceContext<CounterClock>>,
    mut batches: ResMut<AdvanceBatches>,
) {
    if !context.transitions().is_empty() {
        batches.0 += 1;
    }
}

pub struct DemoModule {
    action: bool,
    clock: bool,
    observe_advances: bool,
}

impl DemoModule {
    pub fn action_only() -> Self {
        Self {
            action: true,
            clock: false,
            observe_advances: false,
        }
    }

    pub fn clock_only() -> Self {
        Self {
            action: false,
            clock: true,
            observe_advances: false,
        }
    }

    pub fn mixed() -> Self {
        Self {
            action: true,
            clock: true,
            observe_advances: true,
        }
    }
}

impl BevyModule for DemoModule {
    fn descriptor(&self) -> ModuleDescriptor {
        let execute_entries = self
            .action
            .then(|| {
                ExecuteEntryDefinition::for_input_output::<ActionInput, ActionOutput>(
                    action_entry_id(),
                )
            })
            .into_iter()
            .collect();
        let clocks = self
            .clock
            .then(|| ClockDefinition::for_clock::<CounterClock>(clock_type_id()))
            .into_iter()
            .collect();
        let mut systems = Vec::new();
        if self.action {
            systems.push(SystemDefinition {
                id: action_system_id(),
                trigger: SystemTrigger::Execute {
                    entry: action_entry_id(),
                },
                before: Vec::new(),
                after: Vec::new(),
            });
        }
        if self.observe_advances {
            systems.push(SystemDefinition {
                id: advance_system_id(),
                trigger: SystemTrigger::Advance {
                    clock_type: clock_type_id(),
                },
                before: Vec::new(),
                after: Vec::new(),
            });
        }
        ModuleDescriptor {
            api_version: SIMULATE_API_VERSION.to_owned(),
            id: module_id(),
            version: SemanticVersion::new("1.0.0")
                .expect("hard-coded example module version is valid"),
            dependencies: Vec::new(),
            execution: native_plane(),
            required_capabilities: Default::default(),
            execute_entries,
            clocks,
            systems,
        }
    }

    fn register(
        self: Box<Self>,
        registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError> {
        if self.clock {
            registrar.bind_clock::<CounterClock>(&clock_type_id())?;
        }
        if self.action {
            registrar.add_fallible_system(&action_system_id(), add_action)?;
        }
        if self.observe_advances {
            registrar.add_system(&advance_system_id(), observe_advance)?;
        }
        Ok(())
    }
}

pub fn activate(module: DemoModule) -> Result<BevySimulation, SimulationBuildError> {
    let mut builder = BevySimulationBuilder::new();
    builder.register_module(module)?;
    builder.activate()
}

#![allow(clippy::result_large_err)]

use armillae_simulate::{
    CapabilityId, Clock, ClockErrorCode, ClockTransitionError, ExecutionPlane, Simulation,
    SimulationBuildError, SystemErrorCode, SystemExecutionError, SystemExecutionResult,
    VersionRequirement,
    testing::{
        BackendContractFactory, ContractFixture, ContractSetupError,
        assert_backend_runtime_contract,
    },
};
use armillae_simulate_bevy::{
    AdvanceContext, BevyModule, BevyModuleRegistrar, BevySimulationBuilder, ClockComponent,
    ExecuteContext,
};
use bevy_ecs::prelude::{Query, Res};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct CounterClock {
    value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct ProbeClock {
    value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct Delta {
    delta: i64,
}

fn overflow() -> ClockTransitionError {
    ClockTransitionError {
        code: ClockErrorCode::new("armillae.simulate.contract/overflow")
            .expect("hard-coded test clock error code is valid"),
        message: "counter overflow".to_owned(),
    }
}

impl Clock for CounterClock {
    type Step = Delta;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        self.value
            .checked_add(step.delta)
            .map(|value| Self { value })
            .ok_or_else(overflow)
    }
}

impl Clock for ProbeClock {
    type Step = Delta;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        self.value
            .checked_add(step.delta)
            .map(|value| Self { value })
            .ok_or_else(overflow)
    }
}

fn system_error(code: &str, message: &str) -> SystemExecutionError {
    SystemExecutionError {
        code: SystemErrorCode::new(code).expect("hard-coded test system error code is valid"),
        message: message.to_owned(),
    }
}

fn increment_probe(
    context: Res<ExecuteContext>,
    mut probes: Query<&mut ClockComponent<ProbeClock>>,
) -> SystemExecutionResult {
    let delta: Delta = context.decode().map_err(|_| {
        system_error(
            "armillae.simulate.contract/decode",
            "contract input decode failed",
        )
    })?;
    let mut probe = probes.single_mut().map_err(|_| {
        system_error(
            "armillae.simulate.contract/probe",
            "contract probe is missing",
        )
    })?;
    probe.value_mut().value = probe
        .value()
        .value
        .checked_add(delta.delta)
        .ok_or_else(|| {
            system_error(
                "armillae.simulate.contract/overflow",
                "contract probe overflow",
            )
        })?;
    context.set_output(probe.value()).map_err(Into::into)
}

fn count_advance(
    context: Res<AdvanceContext<CounterClock>>,
    mut probes: Query<&mut ClockComponent<ProbeClock>>,
) -> SystemExecutionResult {
    if context.transitions().is_empty() {
        return Err(system_error(
            "armillae.simulate.contract/transitions",
            "contract transition batch is empty",
        ));
    }
    let mut probe = probes.single_mut().map_err(|_| {
        system_error(
            "armillae.simulate.contract/probe",
            "contract probe is missing",
        )
    })?;
    probe.value_mut().value = probe.value().value.checked_add(1).ok_or_else(|| {
        system_error(
            "armillae.simulate.contract/overflow",
            "contract probe overflow",
        )
    })?;
    Ok(())
}

struct ContractModule {
    descriptor: armillae_simulate::ModuleDescriptor,
}

impl BevyModule for ContractModule {
    fn descriptor(&self) -> armillae_simulate::ModuleDescriptor {
        self.descriptor.clone()
    }

    fn register(
        self: Box<Self>,
        registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError> {
        let counter = self.descriptor.clocks[0].id.clone();
        let probe = self.descriptor.clocks[1].id.clone();
        let execute = self.descriptor.systems[0].id.clone();
        let advance = self.descriptor.systems[1].id.clone();
        registrar.bind_clock::<CounterClock>(&counter)?;
        registrar.bind_clock::<ProbeClock>(&probe)?;
        registrar.add_fallible_system(&execute, increment_probe)?;
        registrar.add_fallible_system(&advance, count_advance)?;
        Ok(())
    }
}

struct BevyContractFactory;

impl BevyContractFactory {
    fn empty_simulation() -> armillae_simulate_bevy::BevySimulation {
        BevySimulationBuilder::new()
            .activate()
            .expect("empty Bevy simulation activates")
    }
}

impl BackendContractFactory for BevyContractFactory {
    fn capabilities(&self) -> armillae_simulate::SimulationCapabilities {
        Self::empty_simulation().capabilities()
    }

    fn execution_plane(&self) -> ExecutionPlane {
        let capabilities = self.capabilities();
        ExecutionPlane::Native {
            backend: capabilities.backend.id,
            adapter: VersionRequirement::new(format!("={}", env!("CARGO_PKG_VERSION")))
                .expect("test adapter requirement is valid"),
        }
    }

    fn create_fixture(
        &self,
        fixture: ContractFixture,
    ) -> Result<Box<dyn Simulation>, ContractSetupError> {
        let clocks = [
            fixture.primary_clock.clone(),
            fixture.secondary_clock.clone(),
            fixture.probe_clock.clone(),
        ];
        let mut builder = BevySimulationBuilder::new();
        builder
            .register_module(ContractModule {
                descriptor: fixture.module,
            })
            .map_err(setup_error)?;
        let mut simulation = builder.activate().map_err(setup_error)?;
        for clock in clocks {
            simulation.insert_clock(clock).map_err(setup_error)?;
        }
        Ok(Box::new(simulation))
    }
}

fn setup_error(error: impl std::fmt::Display) -> ContractSetupError {
    ContractSetupError {
        code: "armillae.simulate.contract/setup".to_owned(),
        message: error.to_string(),
    }
}

#[test]
fn bevy_satisfies_the_shared_runtime_contract() {
    assert_backend_runtime_contract(&BevyContractFactory)
        .expect("Bevy backend must satisfy the shared runtime contract");
}

fn assert_send<T: Send>() {}

#[test]
fn bevy_simulation_is_send() {
    assert_send::<armillae_simulate_bevy::BevySimulation>();
}

#[test]
fn parallel_capability_matches_the_actual_executor_configuration() {
    let capability = CapabilityId::new("armillae.simulate/parallel_systems")
        .expect("parallel capability ID is valid");
    let reported = BevyContractFactory.capabilities().supports(&capability);
    assert_eq!(
        reported,
        cfg!(all(feature = "parallel", not(target_arch = "wasm32")))
    );
}

#![allow(clippy::result_large_err)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use armillae_simulate::{
    BackendId, CapabilityId, ClockDefinition, ClockTypeId, ExecuteEntryDefinition, ExecuteEntryId,
    ExecutionPlane, ModuleDependency, ModuleDescriptor, ModuleId, OrderingError,
    SIMULATE_API_VERSION, SemanticVersion, SimulationBuildError, SystemDefinition, SystemId,
    SystemTrigger, VersionRequirement,
};
use armillae_simulate_bevy::{
    BEVY_BACKEND_ID, BevyModule, BevyModuleRegistrar, BevySimulationBuilder,
};
use bevy_ecs::{prelude::Local, world::FromWorld};
use serde_json::json;

type Register = Box<
    dyn for<'a> FnOnce(&mut BevyModuleRegistrar<'a>) -> Result<(), SimulationBuildError> + Send,
>;

struct TestModule {
    descriptor: ModuleDescriptor,
    register: Register,
}

impl BevyModule for TestModule {
    fn descriptor(&self) -> ModuleDescriptor {
        self.descriptor.clone()
    }

    fn register(
        self: Box<Self>,
        registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError> {
        (self.register)(registrar)
    }
}

fn module_id(value: &str) -> ModuleId {
    ModuleId::new(value).expect("test module ID is valid")
}

fn execute_id(value: &str) -> ExecuteEntryId {
    ExecuteEntryId::new(value).expect("test execute entry ID is valid")
}

fn clock_id(value: &str) -> ClockTypeId {
    ClockTypeId::new(value).expect("test clock type ID is valid")
}

fn system_id(value: &str) -> SystemId {
    SystemId::new(value).expect("test system ID is valid")
}

fn native_plane() -> ExecutionPlane {
    ExecutionPlane::Native {
        backend: BackendId::new(BEVY_BACKEND_ID).expect("Bevy backend ID is valid"),
        adapter: VersionRequirement::new(format!("={}", env!("CARGO_PKG_VERSION")))
            .expect("test adapter requirement is valid"),
    }
}

fn descriptor(id: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        api_version: SIMULATE_API_VERSION.to_owned(),
        id: module_id(id),
        version: SemanticVersion::new("1.0.0").expect("test module version is valid"),
        dependencies: Vec::new(),
        execution: native_plane(),
        required_capabilities: Default::default(),
        execute_entries: Vec::new(),
        clocks: Vec::new(),
        systems: Vec::new(),
    }
}

fn module(
    descriptor: ModuleDescriptor,
    register: impl for<'a> FnOnce(&mut BevyModuleRegistrar<'a>) -> Result<(), SimulationBuildError>
    + Send
    + 'static,
) -> TestModule {
    TestModule {
        descriptor,
        register: Box::new(register),
    }
}

fn noop() {}

#[test]
fn failed_registration_is_atomic_and_builder_remains_reusable() {
    let mut declared = descriptor("test/atomic");
    let system = system_id("test/atomic/system");
    let entry = execute_id("test/atomic/entry");
    declared.execute_entries.push(ExecuteEntryDefinition {
        id: entry.clone(),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
    });
    declared.systems.push(SystemDefinition {
        id: system.clone(),
        trigger: SystemTrigger::Execute { entry },
        before: Vec::new(),
        after: Vec::new(),
    });

    let mut builder = BevySimulationBuilder::new();
    let error = builder
        .register_module(module(declared.clone(), |_| Ok(())))
        .expect_err("missing native binding must fail");
    assert!(matches!(
        error,
        SimulationBuildError::NativeRegistrationFailed {
            module: Some(ref id),
            ..
        } if id == &declared.id
    ));

    builder
        .register_module(module(declared, move |registrar| {
            registrar.add_system(&system, noop)
        }))
        .expect("failed staging must not reserve the module or system IDs");
    builder
        .activate()
        .expect("builder remains usable after registration failure");
}

struct DescriptorPanic;

impl BevyModule for DescriptorPanic {
    fn descriptor(&self) -> ModuleDescriptor {
        panic!("secret descriptor panic payload")
    }

    fn register(
        self: Box<Self>,
        _registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError> {
        Ok(())
    }
}

#[test]
fn module_panics_are_redacted_and_do_not_pollute_the_builder() {
    let mut builder = BevySimulationBuilder::new();
    let descriptor_error = builder
        .register_module(DescriptorPanic)
        .expect_err("descriptor panic must be caught");
    assert!(matches!(
        descriptor_error,
        SimulationBuildError::NativeRegistrationFailed {
            module: None,
            ref code,
            ref message,
        } if code == "armillae.simulate/native_module_panicked"
            && message == "native module panicked"
    ));

    let panicking = descriptor("test/register-panic");
    let expected_id = panicking.id.clone();
    let register_error = builder
        .register_module(module(panicking.clone(), |_| {
            panic!("secret register panic payload")
        }))
        .expect_err("register panic must be caught");
    assert!(matches!(
        register_error,
        SimulationBuildError::NativeRegistrationFailed {
            module: Some(ref id),
            ref code,
            ref message,
        } if id == &expected_id
            && code == "armillae.simulate/native_module_panicked"
            && message == "native module panicked"
    ));

    builder
        .register_module(module(panicking, |_| Ok(())))
        .expect("panicking staging must not reserve its module ID");
    builder
        .activate()
        .expect("builder remains usable after native panic");
}

struct CountingModule {
    calls: Arc<AtomicUsize>,
    descriptor: ModuleDescriptor,
}

impl BevyModule for CountingModule {
    fn descriptor(&self) -> ModuleDescriptor {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.descriptor.clone()
    }

    fn register(
        self: Box<Self>,
        _registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError> {
        Ok(())
    }
}

#[test]
fn descriptor_is_called_exactly_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(CountingModule {
            calls: calls.clone(),
            descriptor: descriptor("test/descriptor-once"),
        })
        .expect("module registers");
    builder.activate().expect("module activates");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn activation_classifies_missing_dependencies_and_unknown_triggers() {
    let mut missing = descriptor("test/missing-dependency");
    missing.dependencies.push(ModuleDependency {
        id: module_id("test/not-registered"),
        version: VersionRequirement::new("^1").expect("test dependency requirement is valid"),
    });
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(missing, |_| Ok(())))
        .expect("dependency checks are activation-time checks");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::MissingDependency { .. })
    ));

    let mut unknown = descriptor("test/unknown-trigger");
    let system = system_id("test/unknown-trigger/system");
    unknown.systems.push(SystemDefinition {
        id: system.clone(),
        trigger: SystemTrigger::Execute {
            entry: execute_id("test/unknown-trigger/entry"),
        },
        before: Vec::new(),
        after: Vec::new(),
    });
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(unknown, move |registrar| {
            registrar.add_system(&system, noop)
        }))
        .expect("native system binding succeeds before graph validation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::UnknownTrigger { .. })
    ));
}

#[test]
fn activation_rejects_ordering_cycles_and_backend_mismatch() {
    let mut cyclic = descriptor("test/cycle");
    let entry = execute_id("test/cycle/entry");
    let first = system_id("test/cycle/first");
    let second = system_id("test/cycle/second");
    cyclic.execute_entries.push(ExecuteEntryDefinition {
        id: entry.clone(),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
    });
    cyclic.systems.extend([
        SystemDefinition {
            id: first.clone(),
            trigger: SystemTrigger::Execute {
                entry: entry.clone(),
            },
            before: vec![second.clone()],
            after: Vec::new(),
        },
        SystemDefinition {
            id: second.clone(),
            trigger: SystemTrigger::Execute { entry },
            before: vec![first.clone()],
            after: Vec::new(),
        },
    ]);
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(cyclic, move |registrar| {
            registrar.add_system(&first, noop)?;
            registrar.add_system(&second, noop)
        }))
        .expect("cycle is validated during activation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::OrderingCycle { .. })
    ));

    let mut mismatch = descriptor("test/backend-mismatch");
    mismatch.execution = ExecutionPlane::Native {
        backend: BackendId::new("test/other-backend").expect("test backend ID is valid"),
        adapter: VersionRequirement::new("*").expect("wildcard requirement is valid"),
    };
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(mismatch, |_| Ok(())))
        .expect("backend compatibility is checked during activation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::BackendMismatch { .. })
    ));
}

struct PanicLocal;

impl FromWorld for PanicLocal {
    fn from_world(_world: &mut bevy_ecs::world::World) -> Self {
        panic!("secret Local::FromWorld panic payload")
    }
}

fn local_panics(_local: Local<PanicLocal>) {}

#[test]
fn system_initialization_panics_have_the_frozen_build_error() {
    let mut declared = descriptor("test/local-panic");
    let entry = execute_id("test/local-panic/entry");
    let system = system_id("test/local-panic/system");
    declared.execute_entries.push(ExecuteEntryDefinition {
        id: entry.clone(),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
    });
    declared.systems.push(SystemDefinition {
        id: system.clone(),
        trigger: SystemTrigger::Execute { entry },
        before: Vec::new(),
        after: Vec::new(),
    });
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(declared, move |registrar| {
            registrar.add_system(&system, local_panics)
        }))
        .expect("system binding succeeds");
    let error = match builder.activate() {
        Ok(_) => panic!("Local initialization panic must abort activation"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        SimulationBuildError::SystemGraphBuildFailed {
            ref code,
            ref message,
            ..
        } if code == "armillae.simulate/bevy_system_initialization_panicked"
            && message == "Bevy system initialization panicked"
    ));
}

#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
struct BoundClock(i64);

impl armillae_simulate::Clock for BoundClock {
    type Step = i64;

    fn advance(&self, step: &Self::Step) -> Result<Self, armillae_simulate::ClockTransitionError> {
        Ok(Self(self.0 + step))
    }
}

#[test]
fn a_rust_clock_type_cannot_be_rebound_across_modules() {
    let clock_a = clock_id("test/rebind/a");
    let mut first = descriptor("test/rebind/first");
    first
        .clocks
        .push(ClockDefinition::for_clock::<BoundClock>(clock_a.clone()));
    let clock_b = clock_id("test/rebind/b");
    let mut second = descriptor("test/rebind/second");
    second
        .clocks
        .push(ClockDefinition::for_clock::<BoundClock>(clock_b.clone()));

    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(first, move |registrar| {
            registrar.bind_clock::<BoundClock>(&clock_a)
        }))
        .expect("first Rust clock binding succeeds");
    let error = builder
        .register_module(module(second.clone(), move |registrar| {
            registrar.bind_clock::<BoundClock>(&clock_b)
        }))
        .expect_err("same Rust clock type cannot bind to another logical clock");
    assert!(matches!(
        error,
        SimulationBuildError::NativeRegistrationFailed {
            module: Some(ref id),
            ref code,
            ..
        } if id == &second.id && code == "armillae.simulate/rust_clock_type_rebound"
    ));
}

#[test]
fn activation_rejects_unsupported_planes_capabilities_and_adapter_versions() {
    let mut hosted = descriptor("test/hosted");
    hosted.execution = ExecutionPlane::Hosted;
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(hosted, |_| Ok(())))
        .expect("execution plane is checked during activation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::UnsupportedExecutionPlane { .. })
    ));

    let mut unsupported = descriptor("test/unsupported-capability");
    unsupported.required_capabilities.insert(
        CapabilityId::new("armillae.simulate/hosted_modules").expect("test capability ID is valid"),
    );
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(unsupported, |_| Ok(())))
        .expect("capabilities are checked during activation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::UnsupportedCapability { .. })
    ));

    let mut incompatible = descriptor("test/incompatible-adapter");
    incompatible.execution = ExecutionPlane::Native {
        backend: BackendId::new(BEVY_BACKEND_ID).expect("Bevy backend ID is valid"),
        adapter: VersionRequirement::new(">=99.0.0").expect("test adapter requirement is valid"),
    };
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(incompatible, |_| Ok(())))
        .expect("adapter version is checked during activation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::IncompatibleAdapter { .. })
    ));
}

#[test]
fn cross_trigger_ordering_is_rejected() {
    let mut declared = descriptor("test/cross-trigger");
    let entry = execute_id("test/cross-trigger/entry");
    let clock = clock_id("test/cross-trigger/clock");
    let execute_system = system_id("test/cross-trigger/execute-system");
    let advance_system = system_id("test/cross-trigger/advance-system");
    declared.execute_entries.push(ExecuteEntryDefinition {
        id: entry.clone(),
        input_schema: json!({ "type": "object" }),
        output_schema: None,
    });
    declared
        .clocks
        .push(ClockDefinition::for_clock::<BoundClock>(clock.clone()));
    declared.systems.extend([
        SystemDefinition {
            id: execute_system.clone(),
            trigger: SystemTrigger::Execute { entry },
            before: vec![advance_system.clone()],
            after: Vec::new(),
        },
        SystemDefinition {
            id: advance_system.clone(),
            trigger: SystemTrigger::Advance {
                clock_type: clock.clone(),
            },
            before: Vec::new(),
            after: Vec::new(),
        },
    ]);
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module(declared, move |registrar| {
            registrar.bind_clock::<BoundClock>(&clock)?;
            registrar.add_system(&execute_system, noop)?;
            registrar.add_system(&advance_system, noop)
        }))
        .expect("native bindings succeed before graph validation");
    assert!(matches!(
        builder.activate(),
        Err(SimulationBuildError::InvalidOrdering {
            reason: OrderingError::DifferentTrigger,
            ..
        })
    ));
}

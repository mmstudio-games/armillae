#![allow(clippy::result_large_err)]

use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use armillae_simulate::{
    AdvanceRequest, AdvanceTarget, BackendId, Clock, ClockDefinition, ClockErrorCode,
    ClockInstanceId, ClockKey, ClockTransitionError, ClockTypeId, ExecuteEntryDefinition,
    ExecuteEntryId, ExecuteRequest, ExecutionPlane, ModuleDescriptor, ModuleId,
    SIMULATE_API_VERSION, SemanticVersion, Simulation, SimulationBuildError, SimulationError,
    SimulationOperation, SimulationStatus, SystemDefinition, SystemErrorCode, SystemExecutionError,
    SystemExecutionResult, SystemId, SystemTrigger, TypedAdvanceRequest, TypedAdvanceTarget,
    VersionRequirement,
};
use armillae_simulate_bevy::{
    AdvanceContext, BEVY_BACKEND_ID, BevyModule, BevyModuleRegistrar, BevySimulation,
    BevySimulationBuilder, ClockComponent, ExecuteContext,
};
use bevy_ecs::{
    error::FallbackErrorHandler,
    prelude::{Commands, Local, Res, ResMut, Resource},
    system::Single,
    world::FromWorld,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, Serializer, ser::Error as _};
use serde_json::{Value, json};

type Register = Box<
    dyn for<'a> FnOnce(&mut BevyModuleRegistrar<'a>) -> Result<(), SimulationBuildError> + Send,
>;

type OutputSystem = fn(Res<ExecuteContext>);
type ErrorMatcher = fn(&SimulationError) -> bool;
type OutputCase = (&'static str, Option<Value>, OutputSystem, ErrorMatcher);

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

fn id<T>(
    value: &str,
    create: impl FnOnce(String) -> Result<T, armillae_simulate::InvalidIdentifier>,
) -> T {
    create(value.to_owned()).expect("test identifier is valid")
}

fn module_id(value: &str) -> ModuleId {
    id(value, ModuleId::new)
}

fn execute_id(value: &str) -> ExecuteEntryId {
    id(value, ExecuteEntryId::new)
}

fn clock_id(value: &str) -> ClockTypeId {
    id(value, ClockTypeId::new)
}

fn instance_id(value: &str) -> ClockInstanceId {
    id(value, ClockInstanceId::new)
}

fn system_id(value: &str) -> SystemId {
    id(value, SystemId::new)
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

fn activate(module: TestModule) -> BevySimulation {
    let mut builder = BevySimulationBuilder::new();
    builder
        .register_module(module)
        .expect("test module registers");
    builder.activate().expect("test module activates")
}

fn execute_descriptor(
    id: &str,
    output_schema: Option<Value>,
    systems: Vec<SystemDefinition>,
) -> (ModuleDescriptor, ExecuteEntryId) {
    let mut descriptor = descriptor(id);
    let entry = execute_id(&format!("{id}/entry"));
    descriptor.execute_entries.push(ExecuteEntryDefinition {
        id: entry.clone(),
        input_schema: json!({ "type": "object" }),
        output_schema,
    });
    descriptor.systems = systems;
    (descriptor, entry)
}

fn request(entry: &ExecuteEntryId) -> ExecuteRequest {
    ExecuteRequest {
        entry: entry.clone(),
        input: json!({}),
    }
}

fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "value": { "type": "integer" } },
        "required": ["value"]
    })
}

fn set_valid_output(context: Res<ExecuteContext>) {
    let _ = context.set_output(&json!({ "value": 1 }));
}

fn set_invalid_output(context: Res<ExecuteContext>) {
    let _ = context.set_output(&json!({ "value": "invalid" }));
}

struct EncodingFails;

impl Serialize for EncodingFails {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(S::Error::custom("secret test encoding payload"))
    }
}

fn set_unencodable_output(context: Res<ExecuteContext>) {
    let _ = context.set_output(&EncodingFails);
}

fn set_output_twice(context: Res<ExecuteContext>) {
    let _ = context.set_output(&json!({ "value": 1 }));
    let _ = context.set_output(&EncodingFails);
}

fn declared_system(entry: &ExecuteEntryId, system: &SystemId) -> SystemDefinition {
    SystemDefinition {
        id: system.clone(),
        trigger: SystemTrigger::Execute {
            entry: entry.clone(),
        },
        before: Vec::new(),
        after: Vec::new(),
    }
}

fn assert_faulted_after_execute(
    simulation: &mut BevySimulation,
    entry: &ExecuteEntryId,
    expected: impl FnOnce(&SimulationError) -> bool,
) {
    let error = simulation
        .execute(request(entry))
        .expect_err("execute must fail");
    assert!(expected(&error), "unexpected execute error: {error:?}");
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
    assert!(matches!(
        simulation.execute(request(entry)),
        Err(SimulationError::Faulted {
            operation: SimulationOperation::Execute
        })
    ));
}

#[test]
fn execute_output_sink_enforces_all_terminal_error_classes_and_priority() {
    let cases: [OutputCase; 4] = [
        ("test/output-unexpected", None, set_valid_output, |error| {
            matches!(error, SimulationError::UnexpectedExecuteOutput { .. })
        }),
        (
            "test/output-encoding",
            Some(output_schema()),
            set_unencodable_output,
            |error| matches!(error, SimulationError::ExecuteOutputEncodingFailed { .. }),
        ),
        (
            "test/output-conflict",
            Some(output_schema()),
            set_output_twice,
            |error| matches!(error, SimulationError::ConflictingExecuteOutput { .. }),
        ),
        (
            "test/output-invalid",
            Some(output_schema()),
            set_invalid_output,
            |error| matches!(error, SimulationError::InvalidExecuteOutput { .. }),
        ),
    ];

    for (module_name, schema, implementation, expected) in cases {
        let entry = execute_id(&format!("{module_name}/entry"));
        let system = system_id(&format!("{module_name}/system"));
        let definition = declared_system(&entry, &system);
        let (descriptor, actual_entry) = execute_descriptor(module_name, schema, vec![definition]);
        let mut simulation = activate(module(descriptor, move |registrar| {
            registrar.add_system(&system, implementation)
        }));
        assert_faulted_after_execute(&mut simulation, &actual_entry, expected);
    }

    let (descriptor, entry) =
        execute_descriptor("test/output-missing", Some(output_schema()), Vec::new());
    let mut simulation = activate(module(descriptor, |_| Ok(())));
    assert_faulted_after_execute(&mut simulation, &entry, |error| {
        matches!(error, SimulationError::MissingExecuteOutput { .. })
    });
}

fn explicit_failure() -> SystemExecutionResult {
    Err(SystemExecutionError {
        code: SystemErrorCode::new("test/explicit-failure").expect("test error code is valid"),
        message: "explicit test failure".to_owned(),
    })
}

#[test]
fn system_failure_precedes_missing_output() {
    let module_name = "test/system-before-missing";
    let entry = execute_id(&format!("{module_name}/entry"));
    let system = system_id(&format!("{module_name}/system"));
    let (descriptor, actual_entry) = execute_descriptor(
        module_name,
        Some(output_schema()),
        vec![declared_system(&entry, &system)],
    );
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_fallible_system(&system, explicit_failure)
    }));
    assert_faulted_after_execute(&mut simulation, &actual_entry, |error| {
        matches!(error, SimulationError::SystemFailed { .. })
    });
}

#[test]
fn execute_context_is_scoped_to_the_operation() {
    let (descriptor, entry) = execute_descriptor("test/context-scope", None, Vec::new());
    let mut simulation = activate(module(descriptor, |_| Ok(())));
    let outcome = simulation
        .execute(request(&entry))
        .expect("no-op execute succeeds");
    assert_eq!(outcome.output, None);
    assert!(
        simulation
            .inspect_world(|world| !world.contains_resource::<ExecuteContext>())
            .expect("world inspection succeeds")
    );
}

#[test]
fn schema_violations_are_stably_sorted_and_deduplicated() {
    let (mut descriptor, entry) = execute_descriptor("test/schema-order", None, Vec::new());
    descriptor.execute_entries[0].input_schema = json!({
        "type": "object",
        "properties": {
            "z": { "type": "integer", "minimum": 10 },
            "a": { "type": "integer", "minimum": 10 }
        },
        "required": ["z", "a"]
    });
    let mut simulation = activate(module(descriptor, |_| Ok(())));
    let error = simulation
        .execute(ExecuteRequest {
            entry,
            input: json!({ "z": 0, "a": 0 }),
        })
        .expect_err("invalid input must be rejected");
    let SimulationError::InvalidExecuteInput { violations, .. } = error else {
        panic!("unexpected schema error: {error:?}");
    };
    assert!(violations.len() >= 2);
    assert!(violations.windows(2).all(|pair| {
        let left = &pair[0];
        let right = &pair[1];
        (
            left.instance_path.as_str(),
            left.schema_path.as_str(),
            left.keyword.as_deref().unwrap_or(""),
        ) < (
            right.instance_path.as_str(),
            right.schema_path.as_str(),
            right.keyword.as_deref().unwrap_or(""),
        )
    }));
    assert_eq!(simulation.status(), SimulationStatus::Active);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct TestClock {
    value: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct Step {
    delta: i64,
}

impl Clock for TestClock {
    type Step = Step;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        if step.delta == 999 {
            return Err(ClockTransitionError {
                code: ClockErrorCode::new("test/rejected-step")
                    .expect("test clock error code is valid"),
                message: "test transition rejected".to_owned(),
            });
        }
        self.value
            .checked_add(step.delta)
            .map(|value| Self { value })
            .ok_or_else(|| ClockTransitionError {
                code: ClockErrorCode::new("test/overflow").expect("test clock error code is valid"),
                message: "test clock overflow".to_owned(),
            })
    }
}

fn clock_simulation() -> (BevySimulation, ClockTypeId) {
    let mut descriptor = descriptor("test/clock");
    let clock_type = clock_id("test/clock/type");
    descriptor
        .clocks
        .push(ClockDefinition::for_clock::<TestClock>(clock_type.clone()));
    let bound = clock_type.clone();
    (
        activate(module(descriptor, move |registrar| {
            registrar.bind_clock::<TestClock>(&bound)
        })),
        clock_type,
    )
}

#[test]
fn typed_and_json_clock_apis_share_entities_and_schedules() {
    let (mut simulation, clock_type) = clock_simulation();
    let instance = instance_id("clock");
    simulation
        .insert_clock_typed(instance.clone(), TestClock { value: 1 })
        .expect("typed insert succeeds");
    let key = ClockKey {
        clock_type: clock_type.clone(),
        instance: instance.clone(),
    };
    assert_eq!(
        simulation
            .read_clock(&key)
            .expect("JSON read succeeds")
            .value,
        json!({ "value": 1 })
    );

    simulation
        .advance(AdvanceRequest {
            clock_type: clock_type.clone(),
            targets: vec![AdvanceTarget {
                instance: instance.clone(),
                step: json!({ "delta": 2 }),
            }],
        })
        .expect("JSON advance succeeds");
    assert_eq!(
        simulation
            .read_clock_typed::<TestClock>(&instance)
            .expect("typed read sees JSON update"),
        TestClock { value: 3 }
    );

    simulation
        .advance_typed::<TestClock>(TypedAdvanceRequest {
            targets: vec![TypedAdvanceTarget {
                instance: instance.clone(),
                step: Step { delta: 4 },
            }],
        })
        .expect("typed advance succeeds");
    assert_eq!(
        simulation
            .read_clock(&key)
            .expect("JSON read sees typed update")
            .value,
        json!({ "value": 7 })
    );
    assert!(
        simulation
            .inspect_world(|world| !world.contains_resource::<AdvanceContext<TestClock>>())
            .expect("world inspection succeeds")
    );
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct UnboundClock(i64);

impl Clock for UnboundClock {
    type Step = i64;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        Ok(Self(self.0 + step))
    }
}

#[test]
fn unbound_typed_clock_is_a_non_fatal_rejection() {
    let (mut simulation, _) = clock_simulation();
    let error = simulation
        .insert_clock_typed(instance_id("unbound"), UnboundClock(1))
        .expect_err("unbound Rust type must be rejected");
    assert!(matches!(
        error,
        SimulationError::NativeClockTypeNotBound { .. }
    ));
    assert_eq!(simulation.status(), SimulationStatus::Active);
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct PriorityClock {
    value: i64,
    panic_on_validate: bool,
}

impl Clock for PriorityClock {
    type Step = i64;

    fn validate(&self) -> Result<(), ClockTransitionError> {
        assert!(!self.panic_on_validate, "duplicate validation must not run");
        Ok(())
    }

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        Ok(Self {
            value: self.value + step,
            panic_on_validate: self.panic_on_validate,
        })
    }
}

#[test]
fn typed_insert_checks_duplicate_before_user_validation() {
    let mut descriptor = descriptor("test/insert-priority");
    let clock_type = clock_id("test/insert-priority/type");
    descriptor
        .clocks
        .push(ClockDefinition::for_clock::<PriorityClock>(
            clock_type.clone(),
        ));
    let bound = clock_type;
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.bind_clock::<PriorityClock>(&bound)
    }));
    let instance = instance_id("same");
    simulation
        .insert_clock_typed(
            instance.clone(),
            PriorityClock {
                value: 1,
                panic_on_validate: false,
            },
        )
        .expect("first insert succeeds");
    assert!(matches!(
        simulation.insert_clock_typed(
            instance,
            PriorityClock {
                value: 2,
                panic_on_validate: true,
            }
        ),
        Err(SimulationError::DuplicateClockInstance { .. })
    ));
    assert_eq!(simulation.status(), SimulationStatus::Active);

    let error = simulation
        .insert_clock_typed(
            instance_id("different"),
            PriorityClock {
                value: 3,
                panic_on_validate: true,
            },
        )
        .expect_err("user Clock panic must be caught");
    assert!(matches!(
        error,
        SimulationError::BackendPanicked {
            operation: SimulationOperation::InsertClock,
            ..
        }
    ));
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
}

#[test]
fn json_advance_reports_the_first_targets_transition_before_later_target_errors() {
    let (mut simulation, clock_type) = clock_simulation();
    let existing = instance_id("existing");
    simulation
        .insert_clock_typed(existing.clone(), TestClock { value: 1 })
        .expect("clock insert succeeds");
    let error = simulation
        .advance(AdvanceRequest {
            clock_type,
            targets: vec![
                AdvanceTarget {
                    instance: existing.clone(),
                    step: json!({ "delta": 999 }),
                },
                AdvanceTarget {
                    instance: instance_id("missing"),
                    step: json!({ "delta": 1 }),
                },
            ],
        })
        .expect_err("first target transition must fail");
    assert!(matches!(
        error,
        SimulationError::ClockTransitionFailed { ref instance, .. } if instance == &existing
    ));
    assert_eq!(
        simulation
            .read_clock_typed::<TestClock>(&existing)
            .expect("failed batch leaves clock unchanged"),
        TestClock { value: 1 }
    );
    assert_eq!(simulation.status(), SimulationStatus::Active);
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
struct CodecClock {
    value: i64,
    fail_encode: bool,
}

impl Serialize for CodecClock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.fail_encode {
            return Err(S::Error::custom("secret clock encoding payload"));
        }
        #[derive(Serialize)]
        struct Encoded {
            value: i64,
            fail_encode: bool,
        }
        Encoded {
            value: self.value,
            fail_encode: self.fail_encode,
        }
        .serialize(serializer)
    }
}

impl Clock for CodecClock {
    type Step = i64;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        Ok(Self {
            value: self.value + step,
            fail_encode: self.fail_encode,
        })
    }
}

#[test]
fn codec_failure_does_not_remove_the_clock_or_fault_the_simulation() {
    let mut descriptor = descriptor("test/codec");
    let clock_type = clock_id("test/codec/type");
    descriptor
        .clocks
        .push(ClockDefinition::for_clock::<CodecClock>(clock_type.clone()));
    let bound = clock_type.clone();
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.bind_clock::<CodecClock>(&bound)
    }));
    let instance = instance_id("codec");
    simulation
        .insert_clock_typed(
            instance.clone(),
            CodecClock {
                value: 1,
                fail_encode: false,
            },
        )
        .expect("clock insert succeeds");
    simulation
        .write_world(|world| {
            let mut query = world.query::<&mut ClockComponent<CodecClock>>();
            let mut component = query.single_mut(world).expect("managed clock exists");
            component.value_mut().fail_encode = true;
        })
        .expect("test mutates the native clock");
    let key = ClockKey {
        clock_type,
        instance: instance.clone(),
    };
    assert!(matches!(
        simulation.remove_clock(&key),
        Err(SimulationError::ClockValueRejected { ref code, .. })
            if code.as_str() == "armillae.simulate/codec"
    ));
    assert_eq!(simulation.status(), SimulationStatus::Active);
    assert!(
        simulation
            .inspect_world(|world| world
                .iter_entities()
                .any(|entity| { entity.contains::<ClockComponent<CodecClock>>() }))
            .expect("managed clock remains after failed encoding")
    );
}

#[derive(Resource)]
struct MissingResource;

fn missing_resource(_missing: Res<MissingResource>) {}

#[test]
fn unhandled_bevy_errors_are_redacted_and_fault_the_simulation() {
    let module_name = "test/fallback";
    let entry = execute_id(&format!("{module_name}/entry"));
    let system = system_id(&format!("{module_name}/system"));
    let (descriptor, actual_entry) =
        execute_descriptor(module_name, None, vec![declared_system(&entry, &system)]);
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_system(&system, missing_resource)
    }));
    simulation
        .write_world(|world| world.remove_resource::<FallbackErrorHandler>())
        .expect("application may remove the private handler between operations");
    assert_faulted_after_execute(&mut simulation, &actual_entry, |error| {
        matches!(
            error,
            SimulationError::BackendFailure {
                code,
                message,
                ..
            } if code == "armillae.simulate/unhandled_bevy_error"
                && message == "unhandled Bevy execution error"
        )
    });
}

#[derive(bevy_ecs::prelude::Component)]
struct SkippedMarker;

fn skipped_single(_single: Single<&SkippedMarker>) {}

#[test]
fn explicitly_skipped_system_params_remain_a_normal_no_op() {
    let module_name = "test/skipped-param";
    let entry = execute_id(&format!("{module_name}/entry"));
    let system = system_id(&format!("{module_name}/system"));
    let (descriptor, actual_entry) =
        execute_descriptor(module_name, None, vec![declared_system(&entry, &system)]);
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_system(&system, skipped_single)
    }));
    let outcome = simulation
        .execute(request(&actual_entry))
        .expect("skipped Single parameter is a normal no-op");
    assert_eq!(outcome.output, None);
    assert_eq!(simulation.status(), SimulationStatus::Active);
}

fn panicking_system() {
    panic!("secret system panic payload")
}

#[test]
fn native_panics_are_caught_and_fault_the_simulation() {
    let module_name = "test/system-panic";
    let entry = execute_id(&format!("{module_name}/entry"));
    let system = system_id(&format!("{module_name}/system"));
    let (descriptor, actual_entry) =
        execute_descriptor(module_name, None, vec![declared_system(&entry, &system)]);
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_system(&system, panicking_system)
    }));
    assert_faulted_after_execute(&mut simulation, &actual_entry, |error| {
        matches!(
            error,
            SimulationError::BackendPanicked {
                operation: SimulationOperation::Execute,
                ..
            }
        )
    });

    let mut empty = BevySimulationBuilder::new()
        .activate()
        .expect("empty simulation activates");
    let error = empty
        .write_world::<()>(|_| panic!("secret world panic payload"))
        .expect_err("write closure panic must be caught");
    assert!(matches!(
        error,
        SimulationError::BackendPanicked {
            operation: SimulationOperation::WriteWorld,
            ..
        }
    ));
    assert_eq!(empty.status(), SimulationStatus::Faulted);
}

#[derive(Resource)]
struct FailureCounter(Arc<AtomicUsize>);

fn fail_a(counter: Res<FailureCounter>) -> SystemExecutionResult {
    counter.0.fetch_add(1, Ordering::SeqCst);
    Err(SystemExecutionError {
        code: SystemErrorCode::new("test/failure-a").expect("test error code is valid"),
        message: "failure a".to_owned(),
    })
}

fn fail_z(counter: Res<FailureCounter>) -> SystemExecutionResult {
    counter.0.fetch_add(1, Ordering::SeqCst);
    Err(SystemExecutionError {
        code: SystemErrorCode::new("test/failure-z").expect("test error code is valid"),
        message: "failure z".to_owned(),
    })
}

#[test]
fn fallible_systems_all_run_and_choose_the_smallest_logical_id() {
    let module_name = "test/multiple-failures";
    let entry = execute_id(&format!("{module_name}/entry"));
    let a = system_id(&format!("{module_name}/a"));
    let z = system_id(&format!("{module_name}/z"));
    let (descriptor, actual_entry) = execute_descriptor(
        module_name,
        None,
        vec![declared_system(&entry, &z), declared_system(&entry, &a)],
    );
    let expected = a.clone();
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_fallible_system(&z, fail_z)?;
        registrar.add_fallible_system(&a, fail_a)
    }));
    let counter = Arc::new(AtomicUsize::new(0));
    let inserted = counter.clone();
    simulation
        .write_world(move |world| world.insert_resource(FailureCounter(inserted)))
        .expect("test failure counter is inserted");
    let error = simulation
        .execute(request(&actual_entry))
        .expect_err("fallible systems must fail the operation");
    assert!(matches!(
        error,
        SimulationError::SystemFailed { ref system, .. } if system == &expected
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 2);
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
}

#[derive(Resource, Default)]
struct ExecutionLog(Vec<&'static str>);

fn log_first(mut log: ResMut<ExecutionLog>) {
    log.0.push("first");
}

fn log_second(mut log: ResMut<ExecutionLog>) {
    log.0.push("second");
}

#[derive(bevy_ecs::prelude::Component)]
struct DeferredMarker;

fn spawn_deferred(mut commands: Commands) {
    commands.spawn(DeferredMarker);
}

#[test]
fn ordering_and_final_deferred_are_applied_before_success_returns() {
    let module_name = "test/order";
    let entry = execute_id(&format!("{module_name}/entry"));
    let first = system_id(&format!("{module_name}/first"));
    let second = system_id(&format!("{module_name}/second"));
    let deferred = system_id(&format!("{module_name}/deferred"));
    let mut first_definition = declared_system(&entry, &first);
    first_definition.before.push(second.clone());
    let (descriptor, actual_entry) = execute_descriptor(
        module_name,
        None,
        vec![
            declared_system(&entry, &second),
            first_definition,
            declared_system(&entry, &deferred),
        ],
    );
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_system(&second, log_second)?;
        registrar.add_system(&first, log_first)?;
        registrar.add_system(&deferred, spawn_deferred)
    }));
    simulation
        .write_world(|world| world.insert_resource(ExecutionLog::default()))
        .expect("execution log is inserted");
    simulation
        .execute(request(&actual_entry))
        .expect("ordered execute succeeds");
    let (log, deferred_count) = simulation
        .inspect_world(|world| {
            let log = world.resource::<ExecutionLog>().0.clone();
            let deferred_count = world
                .iter_entities()
                .filter(|entity| entity.contains::<DeferredMarker>())
                .count();
            (log, deferred_count)
        })
        .expect("world inspection succeeds");
    assert_eq!(log, vec!["first", "second"]);
    assert_eq!(deferred_count, 1);
}

#[derive(Resource)]
struct AppReady;

#[derive(Resource)]
struct InitObservation(Arc<AtomicBool>);

struct InitSawApp(bool);

impl FromWorld for InitSawApp {
    fn from_world(world: &mut bevy_ecs::world::World) -> Self {
        Self(world.contains_resource::<AppReady>())
    }
}

fn observe_initialization(local: Local<InitSawApp>, observation: Res<InitObservation>) {
    observation.0.store(local.0, Ordering::SeqCst);
}

#[test]
fn local_initialization_only_sees_the_adapter_initialization_world() {
    let module_name = "test/local-timing";
    let entry = execute_id(&format!("{module_name}/entry"));
    let system = system_id(&format!("{module_name}/system"));
    let (descriptor, actual_entry) =
        execute_descriptor(module_name, None, vec![declared_system(&entry, &system)]);
    let mut simulation = activate(module(descriptor, move |registrar| {
        registrar.add_system(&system, observe_initialization)
    }));
    let observation = Arc::new(AtomicBool::new(true));
    let external = observation.clone();
    simulation
        .write_world(move |world| {
            world.insert_resource(AppReady);
            world.insert_resource(InitObservation(external));
        })
        .expect("application resources are inserted after activation");
    simulation
        .execute(request(&actual_entry))
        .expect("observation system runs");
    assert!(!observation.load(Ordering::SeqCst));
}

#[test]
fn managed_clock_index_corruption_faults_instead_of_guessing_identity() {
    let (mut simulation, clock_type) = clock_simulation();
    let instance = instance_id("managed");
    simulation
        .insert_clock_typed(instance.clone(), TestClock { value: 1 })
        .expect("managed clock is inserted");
    simulation
        .write_world(|world| {
            let entity = world
                .iter_entities()
                .find(|entity| entity.contains::<ClockComponent<TestClock>>())
                .map(|entity| entity.id())
                .expect("managed clock entity exists");
            assert!(world.despawn(entity));
        })
        .expect("native escape hatch can corrupt the managed index");
    let error = simulation
        .read_clock(&ClockKey {
            clock_type,
            instance,
        })
        .expect_err("corrupt index must be detected");
    assert!(matches!(
        error,
        SimulationError::BackendFailure {
            operation: SimulationOperation::ReadClock,
            ..
        }
    ));
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
}

#[test]
fn non_send_data_is_usable_on_its_owner_thread_and_faults_on_the_wrong_thread() {
    let mut simulation = BevySimulationBuilder::new()
        .activate()
        .expect("empty simulation activates");
    simulation
        .write_world(|world| world.insert_non_send(Rc::new(7_u8)))
        .expect("non-send resource is inserted on the owner thread");
    assert_eq!(
        simulation
            .inspect_world(|world| **world.non_send::<Rc<u8>>())
            .expect("owner thread can inspect non-send data"),
        7
    );

    let (simulation, error) = std::thread::spawn(move || {
        let error = simulation
            .inspect_world(|world| **world.non_send::<Rc<u8>>())
            .expect_err("wrong-thread access must not succeed");
        (simulation, error)
    })
    .join()
    .expect("adapter catches the Bevy thread-affinity panic");
    assert!(matches!(
        error,
        SimulationError::BackendPanicked {
            operation: SimulationOperation::InspectWorld,
            ..
        }
    ));
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
}

use std::{
    any::{TypeId, type_name},
    cell::Cell,
    collections::{BTreeSet, HashMap, HashSet},
    panic::{AssertUnwindSafe, catch_unwind},
};

use armillae_simulate::*;
use bevy_ecs::{
    entity::Entity,
    error::FallbackErrorHandler,
    prelude::{IntoScheduleConfigs, World},
    schedule::Schedule,
};

#[cfg(any(not(feature = "parallel"), target_arch = "wasm32"))]
use bevy_ecs::schedule::SingleThreadedExecutor;

use crate::{
    AdvanceContext, ClockComponent, ExecuteContext,
    builder::{BevySimulationBuilder, ClockRegistration, RegisteredSystem},
    context::ExecuteOutputSink,
    support::{
        CompiledSchema, LogicalSystemSet, SystemFailureCollector, UnhandledBevyErrorMarker,
        backend_id, capabilities, redacting_fallback_handler,
    },
};

struct ExecuteRuntime {
    definition: ExecuteEntryDefinition,
    input: CompiledSchema,
    output: Option<CompiledSchema>,
}

struct ClockRuntime {
    registration: ClockRegistration,
    value: CompiledSchema,
    step: CompiledSchema,
}

pub struct BevySimulation {
    world: World,
    status: Cell<SimulationStatus>,
    capabilities: SimulationCapabilities,
    entries: HashMap<ExecuteEntryId, ExecuteRuntime>,
    clocks: HashMap<ClockTypeId, ClockRuntime>,
    rust_clocks: HashMap<TypeId, ClockTypeId>,
    clock_entities: HashMap<ClockKey, Entity>,
    execute_schedules: HashMap<ExecuteEntryId, Schedule>,
    advance_schedules: HashMap<ClockTypeId, Schedule>,
}

impl BevySimulation {
    pub(crate) fn from_builder(
        builder: BevySimulationBuilder,
    ) -> Result<Self, SimulationBuildError> {
        validate_modules(&builder)?;
        let capabilities = capabilities();
        let mut world = World::new();
        restore_internal_resources(&mut world);

        let mut entries = HashMap::new();
        let mut clocks = HashMap::new();
        let mut rust_clocks = HashMap::new();
        let mut grouped_systems: HashMap<SystemTrigger, Vec<RegisteredSystem>> = HashMap::new();

        for module in builder.modules {
            for entry in module.descriptor.execute_entries {
                let input = CompiledSchema::build(
                    &entry.input_schema,
                    Some(module.descriptor.id.clone()),
                    "armillae.simulate/execute_input_schema",
                )?;
                let output = entry
                    .output_schema
                    .as_ref()
                    .map(|schema| {
                        CompiledSchema::build(
                            schema,
                            Some(module.descriptor.id.clone()),
                            "armillae.simulate/execute_output_schema",
                        )
                    })
                    .transpose()?;
                entries.insert(
                    entry.id.clone(),
                    ExecuteRuntime {
                        definition: entry,
                        input,
                        output,
                    },
                );
            }
            let mut definitions: HashMap<_, _> = module
                .descriptor
                .clocks
                .into_iter()
                .map(|definition| (definition.id.clone(), definition))
                .collect();
            for registration in module.clocks {
                let definition = definitions
                    .remove(&registration.clock_type)
                    .ok_or_else(|| SimulationBuildError::NativeRegistrationFailed {
                        module: Some(module.descriptor.id.clone()),
                        code: "armillae.simulate/missing_clock_definition".to_owned(),
                        message: "native clock definition is missing".to_owned(),
                    })?;
                let value = CompiledSchema::build(
                    &definition.value_schema,
                    Some(module.descriptor.id.clone()),
                    "armillae.simulate/clock_value_schema",
                )?;
                let step = CompiledSchema::build(
                    &definition.step_schema,
                    Some(module.descriptor.id.clone()),
                    "armillae.simulate/clock_step_schema",
                )?;
                if rust_clocks
                    .insert(registration.rust_type, registration.clock_type.clone())
                    .is_some()
                {
                    return Err(SimulationBuildError::NativeRegistrationFailed {
                        module: Some(module.descriptor.id.clone()),
                        code: "armillae.simulate/rust_clock_type_rebound".to_owned(),
                        message: "a Rust Clock type is bound more than once".to_owned(),
                    });
                }
                clocks.insert(
                    registration.clock_type.clone(),
                    ClockRuntime {
                        registration,
                        value,
                        step,
                    },
                );
            }
            for system in module.systems {
                grouped_systems
                    .entry(system.definition.trigger.clone())
                    .or_default()
                    .push(system);
            }
        }

        let mut execute_schedules = HashMap::new();
        for entry in entries.keys() {
            let trigger = SystemTrigger::Execute {
                entry: entry.clone(),
            };
            let systems = grouped_systems.remove(&trigger).unwrap_or_default();
            execute_schedules.insert(entry.clone(), build_schedule(trigger, systems, &mut world)?);
        }
        let mut advance_schedules = HashMap::new();
        for clock_type in clocks.keys() {
            let trigger = SystemTrigger::Advance {
                clock_type: clock_type.clone(),
            };
            let systems = grouped_systems.remove(&trigger).unwrap_or_default();
            advance_schedules.insert(
                clock_type.clone(),
                build_schedule(trigger, systems, &mut world)?,
            );
        }

        Ok(Self {
            world,
            status: Cell::new(SimulationStatus::Active),
            capabilities,
            entries,
            clocks,
            rust_clocks,
            clock_entities: HashMap::new(),
            execute_schedules,
            advance_schedules,
        })
    }

    fn check(
        &self,
        operation: SimulationOperation,
        stopped_read: bool,
    ) -> Result<(), SimulationError> {
        let status = self.status.get();
        match status {
            SimulationStatus::Active => Ok(()),
            SimulationStatus::Stopped if stopped_read => Ok(()),
            SimulationStatus::Stopped => Err(SimulationError::InvalidState {
                operation,
                status: SimulationStatus::Stopped,
            }),
            SimulationStatus::Faulted => Err(SimulationError::Faulted { operation }),
            _ => Err(SimulationError::InvalidState { operation, status }),
        }
    }

    fn write_boundary<T>(
        &mut self,
        operation: SimulationOperation,
        run: impl FnOnce(&mut Self) -> Result<T, SimulationError>,
    ) -> Result<T, SimulationError> {
        restore_internal_resources(&mut self.world);
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(result) => {
                if result.as_ref().err().is_some_and(is_fatal) {
                    self.status.set(SimulationStatus::Faulted);
                }
                result
            }
            Err(payload) => {
                self.status.set(SimulationStatus::Faulted);
                if payload.is::<UnhandledBevyErrorMarker>() {
                    Err(SimulationError::BackendFailure {
                        backend: backend_id(),
                        operation,
                        code: "armillae.simulate/unhandled_bevy_error".to_owned(),
                        message: "unhandled Bevy execution error".to_owned(),
                    })
                } else {
                    Err(SimulationError::BackendPanicked {
                        backend: backend_id(),
                        operation,
                    })
                }
            }
        }
    }

    fn read_boundary<T>(
        &self,
        operation: SimulationOperation,
        run: impl FnOnce(&Self) -> Result<T, SimulationError>,
    ) -> Result<T, SimulationError> {
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(result) => {
                if result.as_ref().err().is_some_and(is_fatal) {
                    self.status.set(SimulationStatus::Faulted);
                }
                result
            }
            Err(_) => {
                self.status.set(SimulationStatus::Faulted);
                Err(SimulationError::BackendPanicked {
                    backend: backend_id(),
                    operation,
                })
            }
        }
    }

    fn run_schedule(
        &mut self,
        trigger: &SystemTrigger,
        operation: SimulationOperation,
    ) -> Result<(), SimulationError> {
        self.world.resource::<SystemFailureCollector>().clear();
        let mut schedule = match trigger {
            SystemTrigger::Execute { entry } => self.execute_schedules.remove(entry),
            SystemTrigger::Advance { clock_type } => self.advance_schedules.remove(clock_type),
            _ => return Err(backend_failure(operation, "unsupported_trigger")),
        }
        .ok_or_else(|| backend_failure(operation, "schedule_missing"))?;
        schedule.run(&mut self.world);
        match trigger {
            SystemTrigger::Execute { entry } => {
                self.execute_schedules.insert(entry.clone(), schedule);
            }
            SystemTrigger::Advance { clock_type } => {
                self.advance_schedules.insert(clock_type.clone(), schedule);
            }
            _ => return Err(backend_failure(operation, "unsupported_trigger")),
        }
        if let Some((system, error)) = self.world.resource::<SystemFailureCollector>().first() {
            return Err(SimulationError::SystemFailed {
                system,
                trigger: trigger.clone(),
                code: error.code,
                message: error.message,
            });
        }
        Ok(())
    }

    pub fn insert_clock_typed<C>(
        &mut self,
        instance: ClockInstanceId,
        value: C,
    ) -> Result<(), SimulationError>
    where
        C: Clock,
    {
        self.check(SimulationOperation::InsertClock, false)?;
        let clock_type = self.bound_clock::<C>()?;
        self.write_boundary(SimulationOperation::InsertClock, move |simulation| {
            let key = ClockKey {
                clock_type: clock_type.clone(),
                instance: instance.clone(),
            };
            if simulation.clock_entities.contains_key(&key) {
                return Err(SimulationError::DuplicateClockInstance { key });
            }
            value
                .validate()
                .map_err(|error| SimulationError::ClockValueRejected {
                    key,
                    code: error.code,
                    message: error.message,
                })?;
            simulation.insert_typed_value(clock_type, instance, value)
        })
    }

    pub fn read_clock_typed<C>(&self, instance: &ClockInstanceId) -> Result<C, SimulationError>
    where
        C: Clock,
    {
        self.check(SimulationOperation::ReadClock, true)?;
        let clock_type = self.bound_clock::<C>()?;
        let key = ClockKey {
            clock_type,
            instance: instance.clone(),
        };
        self.read_boundary(SimulationOperation::ReadClock, |simulation| {
            simulation.read_typed_value::<C>(&key, SimulationOperation::ReadClock)
        })
    }

    pub fn remove_clock_typed<C>(
        &mut self,
        instance: &ClockInstanceId,
    ) -> Result<C, SimulationError>
    where
        C: Clock,
    {
        self.check(SimulationOperation::RemoveClock, false)?;
        let clock_type = self.bound_clock::<C>()?;
        let key = ClockKey {
            clock_type,
            instance: instance.clone(),
        };
        self.write_boundary(SimulationOperation::RemoveClock, |simulation| {
            let value = simulation.read_typed_value::<C>(&key, SimulationOperation::RemoveClock)?;
            simulation.despawn_clock(&key)?;
            Ok(value)
        })
    }

    pub fn advance_typed<C>(
        &mut self,
        request: TypedAdvanceRequest<C::Step>,
    ) -> Result<TypedAdvanceOutcome<C>, SimulationError>
    where
        C: Clock,
    {
        self.check(SimulationOperation::Advance, false)?;
        let clock_type = self.bound_clock::<C>()?;
        validate_typed_targets(&clock_type, &request.targets)?;
        self.write_boundary(SimulationOperation::Advance, move |simulation| {
            let prepared = simulation.prepare_typed::<C>(&clock_type, request.targets)?;
            simulation.apply_and_run(&clock_type, &prepared)?;
            Ok(TypedAdvanceOutcome {
                clock_type,
                transitions: prepared
                    .into_iter()
                    .map(|prepared| prepared.transition)
                    .collect(),
            })
        })
    }

    pub fn inspect_world<R>(
        &self,
        inspect: impl for<'w> FnOnce(&'w World) -> R,
    ) -> Result<R, SimulationError> {
        self.check(SimulationOperation::InspectWorld, true)?;
        self.read_boundary(SimulationOperation::InspectWorld, |simulation| {
            Ok(inspect(&simulation.world))
        })
    }

    pub fn write_world<R>(
        &mut self,
        write: impl for<'w> FnOnce(&'w mut World) -> R,
    ) -> Result<R, SimulationError> {
        self.check(SimulationOperation::WriteWorld, false)?;
        self.write_boundary(SimulationOperation::WriteWorld, |simulation| {
            let output = write(&mut simulation.world);
            restore_internal_resources(&mut simulation.world);
            Ok(output)
        })
    }

    fn bound_clock<C: Clock>(&self) -> Result<ClockTypeId, SimulationError> {
        self.rust_clocks.get(&TypeId::of::<C>()).cloned().ok_or(
            SimulationError::NativeClockTypeNotBound {
                rust_type: type_name::<C>(),
            },
        )
    }

    fn insert_typed_value<C: Clock>(
        &mut self,
        clock_type: ClockTypeId,
        instance: ClockInstanceId,
        value: C,
    ) -> Result<(), SimulationError> {
        let key = ClockKey {
            clock_type,
            instance: instance.clone(),
        };
        if self.clock_entities.contains_key(&key) {
            return Err(SimulationError::DuplicateClockInstance { key });
        }
        let entity = self.world.spawn(ClockComponent::new(instance, value)).id();
        self.clock_entities.insert(key, entity);
        Ok(())
    }

    fn read_typed_value<C: Clock>(
        &self,
        key: &ClockKey,
        operation: SimulationOperation,
    ) -> Result<C, SimulationError> {
        let entity = self
            .clock_entities
            .get(key)
            .copied()
            .ok_or_else(|| SimulationError::UnknownClockInstance { key: key.clone() })?;
        self.world
            .get::<ClockComponent<C>>(entity)
            .map(|component| component.value().clone())
            .ok_or_else(|| backend_failure(operation, "clock_index_corrupt"))
    }

    fn despawn_clock(&mut self, key: &ClockKey) -> Result<(), SimulationError> {
        let entity = self
            .clock_entities
            .remove(key)
            .ok_or_else(|| SimulationError::UnknownClockInstance { key: key.clone() })?;
        if !self.world.despawn(entity) {
            return Err(backend_failure(
                SimulationOperation::RemoveClock,
                "clock_index_corrupt",
            ));
        }
        Ok(())
    }

    fn prepare_typed<C: Clock>(
        &self,
        clock_type: &ClockTypeId,
        targets: Vec<TypedAdvanceTarget<C::Step>>,
    ) -> Result<Vec<PreparedClock<C>>, SimulationError> {
        let mut prepared = Vec::with_capacity(targets.len());
        for target in targets {
            prepared.push(self.prepare_target::<C>(clock_type, target)?);
        }
        Ok(prepared)
    }

    fn prepare_target<C: Clock>(
        &self,
        clock_type: &ClockTypeId,
        target: TypedAdvanceTarget<C::Step>,
    ) -> Result<PreparedClock<C>, SimulationError> {
        let key = ClockKey {
            clock_type: clock_type.clone(),
            instance: target.instance.clone(),
        };
        let entity = self
            .clock_entities
            .get(&key)
            .copied()
            .ok_or_else(|| SimulationError::UnknownClockInstance { key: key.clone() })?;
        let before = self
            .world
            .get::<ClockComponent<C>>(entity)
            .ok_or_else(|| backend_failure(SimulationOperation::Advance, "clock_index_corrupt"))?
            .value()
            .clone();
        before
            .validate()
            .map_err(|error| SimulationError::ClockTransitionFailed {
                clock_type: clock_type.clone(),
                instance: target.instance.clone(),
                code: error.code,
                message: error.message,
            })?;
        let after = before.advance(&target.step).map_err(|error| {
            SimulationError::ClockTransitionFailed {
                clock_type: clock_type.clone(),
                instance: target.instance.clone(),
                code: error.code,
                message: error.message,
            }
        })?;
        after
            .validate()
            .map_err(|error| SimulationError::ClockTransitionFailed {
                clock_type: clock_type.clone(),
                instance: target.instance.clone(),
                code: error.code,
                message: error.message,
            })?;
        Ok(PreparedClock {
            entity,
            transition: TypedClockTransition {
                instance: target.instance,
                before,
                step: target.step,
                after,
            },
        })
    }

    fn apply_and_run<C: Clock>(
        &mut self,
        clock_type: &ClockTypeId,
        prepared: &[PreparedClock<C>],
    ) -> Result<(), SimulationError> {
        for update in prepared {
            let mut component = self
                .world
                .get_mut::<ClockComponent<C>>(update.entity)
                .ok_or_else(|| {
                    backend_failure(SimulationOperation::Advance, "clock_index_corrupt")
                })?;
            *component.value_mut() = update.transition.after.clone();
        }
        self.world.insert_resource(AdvanceContext::new(
            clock_type.clone(),
            prepared
                .iter()
                .map(|prepared| prepared.transition.clone())
                .collect(),
        ));
        let trigger = SystemTrigger::Advance {
            clock_type: clock_type.clone(),
        };
        let result = self.run_schedule(&trigger, SimulationOperation::Advance);
        self.world.remove_resource::<AdvanceContext<C>>();
        result
    }
}

struct PreparedClock<C: Clock> {
    entity: Entity,
    transition: TypedClockTransition<C>,
}

impl Simulation for BevySimulation {
    fn status(&self) -> SimulationStatus {
        self.status.get()
    }

    fn capabilities(&self) -> SimulationCapabilities {
        self.capabilities.clone()
    }

    fn execute(&mut self, request: ExecuteRequest) -> Result<ExecuteOutcome, SimulationError> {
        self.check(SimulationOperation::Execute, false)?;
        self.write_boundary(SimulationOperation::Execute, move |simulation| {
            let entry = simulation.entries.get(&request.entry).ok_or_else(|| {
                SimulationError::UnknownExecuteEntry {
                    entry: request.entry.clone(),
                }
            })?;
            let violations = entry.input.violations(&request.input);
            if !violations.is_empty() {
                return Err(SimulationError::InvalidExecuteInput {
                    entry: request.entry,
                    violations,
                });
            }
            let entry_id = request.entry.clone();
            let output_declared = entry.definition.output_schema.is_some();
            let sink = ExecuteOutputSink::new(output_declared);
            simulation
                .world
                .insert_resource(ExecuteContext::new(request, sink.clone()));
            let trigger = SystemTrigger::Execute {
                entry: entry_id.clone(),
            };
            let schedule_result = simulation.run_schedule(&trigger, SimulationOperation::Execute);
            simulation.world.remove_resource::<ExecuteContext>();
            let output_state = sink.snapshot();
            if output_state.not_declared {
                return Err(SimulationError::UnexpectedExecuteOutput { entry: entry_id });
            }
            if output_state.attempts > 1 {
                return Err(SimulationError::ConflictingExecuteOutput { entry: entry_id });
            }
            if output_state.encoding_failed {
                return Err(SimulationError::ExecuteOutputEncodingFailed { entry: entry_id });
            }
            schedule_result?;
            let runtime = simulation
                .entries
                .get(&entry_id)
                .ok_or_else(|| backend_failure(SimulationOperation::Execute, "entry_missing"))?;
            let output = match (&runtime.output, output_state.output) {
                (None, None) => None,
                (Some(_), None) => {
                    return Err(SimulationError::MissingExecuteOutput { entry: entry_id });
                }
                (Some(schema), Some(output)) => {
                    let violations = schema.violations(&output);
                    if !violations.is_empty() {
                        return Err(SimulationError::InvalidExecuteOutput {
                            entry: entry_id,
                            violations,
                        });
                    }
                    Some(output)
                }
                (None, Some(_)) => {
                    return Err(SimulationError::UnexpectedExecuteOutput { entry: entry_id });
                }
            };
            Ok(ExecuteOutcome {
                entry: entry_id,
                output,
            })
        })
    }

    fn read_clock(&self, key: &ClockKey) -> Result<ClockState, SimulationError> {
        self.check(SimulationOperation::ReadClock, true)?;
        let read = self
            .clocks
            .get(&key.clock_type)
            .map(|clock| clock.registration.read_json)
            .ok_or_else(|| SimulationError::UnknownClockType {
                clock_type: key.clock_type.clone(),
            })?;
        self.read_boundary(SimulationOperation::ReadClock, |simulation| {
            read(simulation, key)
        })
    }

    fn insert_clock(&mut self, state: ClockState) -> Result<(), SimulationError> {
        self.check(SimulationOperation::InsertClock, false)?;
        let insert = self
            .clocks
            .get(&state.key.clock_type)
            .map(|clock| clock.registration.insert_json)
            .ok_or_else(|| SimulationError::UnknownClockType {
                clock_type: state.key.clock_type.clone(),
            })?;
        self.write_boundary(SimulationOperation::InsertClock, move |simulation| {
            insert(simulation, state)
        })
    }

    fn remove_clock(&mut self, key: &ClockKey) -> Result<ClockState, SimulationError> {
        self.check(SimulationOperation::RemoveClock, false)?;
        let remove = self
            .clocks
            .get(&key.clock_type)
            .map(|clock| clock.registration.remove_json)
            .ok_or_else(|| SimulationError::UnknownClockType {
                clock_type: key.clock_type.clone(),
            })?;
        let key = key.clone();
        self.write_boundary(SimulationOperation::RemoveClock, move |simulation| {
            remove(simulation, &key)
        })
    }

    fn advance(&mut self, request: AdvanceRequest) -> Result<AdvanceOutcome, SimulationError> {
        self.check(SimulationOperation::Advance, false)?;
        let advance = self
            .clocks
            .get(&request.clock_type)
            .map(|clock| clock.registration.advance_json)
            .ok_or_else(|| SimulationError::UnknownClockType {
                clock_type: request.clock_type.clone(),
            })?;
        validate_json_targets(&request)?;
        self.write_boundary(SimulationOperation::Advance, move |simulation| {
            advance(simulation, request)
        })
    }

    fn stop(&mut self) -> Result<(), SimulationError> {
        let status = self.status.get();
        match status {
            SimulationStatus::Active => {
                self.status.set(SimulationStatus::Stopped);
                Ok(())
            }
            SimulationStatus::Stopped => Ok(()),
            SimulationStatus::Faulted => Err(SimulationError::Faulted {
                operation: SimulationOperation::Stop,
            }),
            _ => Err(SimulationError::InvalidState {
                operation: SimulationOperation::Stop,
                status,
            }),
        }
    }
}

pub(crate) fn insert_json_for<C: Clock>(
    simulation: &mut BevySimulation,
    state: ClockState,
) -> Result<(), SimulationError> {
    if simulation.clock_entities.contains_key(&state.key) {
        return Err(SimulationError::DuplicateClockInstance { key: state.key });
    }
    let runtime = simulation
        .clocks
        .get(&state.key.clock_type)
        .ok_or_else(|| SimulationError::UnknownClockType {
            clock_type: state.key.clock_type.clone(),
        })?;
    let violations = runtime.value.violations(&state.value);
    if !violations.is_empty() {
        return Err(SimulationError::InvalidClockValue {
            key: state.key,
            violations,
        });
    }
    let value: C =
        serde_json::from_value(state.value).map_err(|_| codec_value_error(&state.key))?;
    value
        .validate()
        .map_err(|error| SimulationError::ClockValueRejected {
            key: state.key.clone(),
            code: error.code,
            message: error.message,
        })?;
    simulation.insert_typed_value(state.key.clock_type, state.key.instance, value)
}

pub(crate) fn read_json_for<C: Clock>(
    simulation: &BevySimulation,
    key: &ClockKey,
) -> Result<ClockState, SimulationError> {
    let value = simulation.read_typed_value::<C>(key, SimulationOperation::ReadClock)?;
    let encoded = serde_json::to_value(value).map_err(|_| codec_value_error(key))?;
    Ok(ClockState {
        key: key.clone(),
        value: encoded,
    })
}

pub(crate) fn remove_json_for<C: Clock>(
    simulation: &mut BevySimulation,
    key: &ClockKey,
) -> Result<ClockState, SimulationError> {
    let value = simulation.read_typed_value::<C>(key, SimulationOperation::RemoveClock)?;
    let encoded = serde_json::to_value(value).map_err(|_| codec_value_error(key))?;
    let state = ClockState {
        key: key.clone(),
        value: encoded,
    };
    simulation.despawn_clock(key)?;
    Ok(state)
}

pub(crate) fn advance_json_for<C: Clock>(
    simulation: &mut BevySimulation,
    request: AdvanceRequest,
) -> Result<AdvanceOutcome, SimulationError> {
    let AdvanceRequest {
        clock_type,
        targets,
    } = request;
    let runtime =
        simulation
            .clocks
            .get(&clock_type)
            .ok_or_else(|| SimulationError::UnknownClockType {
                clock_type: clock_type.clone(),
            })?;
    let mut prepared = Vec::with_capacity(targets.len());
    let mut transitions = Vec::with_capacity(targets.len());
    for target in targets {
        let key = ClockKey {
            clock_type: clock_type.clone(),
            instance: target.instance.clone(),
        };
        if !simulation.clock_entities.contains_key(&key) {
            return Err(SimulationError::UnknownClockInstance { key });
        }
        let violations = runtime.step.violations(&target.step);
        if !violations.is_empty() {
            return Err(SimulationError::InvalidClockStep {
                clock_type: clock_type.clone(),
                instance: target.instance.clone(),
                violations,
            });
        }
        let step: C::Step = serde_json::from_value(target.step).map_err(|_| {
            SimulationError::ClockTransitionFailed {
                clock_type: clock_type.clone(),
                instance: target.instance.clone(),
                code: codec_code(),
                message: "clock codec failed".to_owned(),
            }
        })?;
        let update = simulation.prepare_target::<C>(
            &clock_type,
            TypedAdvanceTarget {
                instance: target.instance,
                step,
            },
        )?;
        let before = serde_json::to_value(&update.transition.before)
            .map_err(|_| codec_transition_error(&clock_type, &update.transition.instance))?;
        let step = serde_json::to_value(&update.transition.step)
            .map_err(|_| codec_transition_error(&clock_type, &update.transition.instance))?;
        let after = serde_json::to_value(&update.transition.after)
            .map_err(|_| codec_transition_error(&clock_type, &update.transition.instance))?;
        let violations = runtime.value.violations(&after);
        if !violations.is_empty() {
            return Err(SimulationError::InvalidClockValue {
                key: ClockKey {
                    clock_type: clock_type.clone(),
                    instance: update.transition.instance.clone(),
                },
                violations,
            });
        }
        transitions.push(ClockTransition {
            instance: update.transition.instance.clone(),
            before,
            step,
            after,
        });
        prepared.push(update);
    }
    simulation.apply_and_run(&clock_type, &prepared)?;
    Ok(AdvanceOutcome {
        clock_type,
        transitions,
    })
}

fn restore_internal_resources(world: &mut World) {
    world.insert_resource(FallbackErrorHandler(redacting_fallback_handler));
    if !world.contains_resource::<SystemFailureCollector>() {
        world.insert_resource(SystemFailureCollector::default());
    }
}

fn build_schedule(
    trigger: SystemTrigger,
    systems: Vec<RegisteredSystem>,
    world: &mut World,
) -> Result<Schedule, SimulationBuildError> {
    let definitions: Vec<_> = systems
        .iter()
        .map(|system| system.definition.clone())
        .collect();
    let mut schedule = Schedule::default();
    configure_executor(&mut schedule);
    schedule.set_apply_final_deferred(true);
    for system in systems {
        let logical_set = LogicalSystemSet(system.definition.id);
        schedule.add_systems(system.implementation.in_set(logical_set));
    }
    for definition in definitions {
        let current = LogicalSystemSet(definition.id);
        for target in definition.before {
            schedule.configure_sets(current.clone().before(LogicalSystemSet(target)));
        }
        for target in definition.after {
            schedule.configure_sets(current.clone().after(LogicalSystemSet(target)));
        }
    }
    let initialized = catch_unwind(AssertUnwindSafe(|| schedule.initialize(world)));
    match initialized {
        Ok(Ok(_)) => Ok(schedule),
        Ok(Err(_)) => Err(SimulationBuildError::SystemGraphBuildFailed {
            trigger,
            code: "armillae.simulate/bevy_system_graph".to_owned(),
            message: "Bevy system graph initialization failed".to_owned(),
        }),
        Err(_) => Err(SimulationBuildError::SystemGraphBuildFailed {
            trigger,
            code: "armillae.simulate/bevy_system_initialization_panicked".to_owned(),
            message: "Bevy system initialization panicked".to_owned(),
        }),
    }
}

fn configure_executor(schedule: &mut Schedule) {
    #[cfg(all(feature = "parallel", not(target_arch = "wasm32")))]
    schedule.set_executor(bevy_ecs::schedule::MultiThreadedExecutor::new());
    #[cfg(any(not(feature = "parallel"), target_arch = "wasm32"))]
    schedule.set_executor(SingleThreadedExecutor::new());
}

fn validate_modules(builder: &BevySimulationBuilder) -> Result<(), SimulationBuildError> {
    let capabilities = capabilities();
    let modules: HashMap<_, _> = builder
        .modules
        .iter()
        .map(|module| (module.descriptor.id.clone(), &module.descriptor))
        .collect();
    let entries: HashMap<_, _> = builder
        .modules
        .iter()
        .flat_map(|module| {
            module
                .descriptor
                .execute_entries
                .iter()
                .map(move |entry| (entry.id.clone(), module.descriptor.id.clone()))
        })
        .collect();
    let clocks: HashMap<_, _> = builder
        .modules
        .iter()
        .flat_map(|module| {
            module
                .descriptor
                .clocks
                .iter()
                .map(move |clock| (clock.id.clone(), module.descriptor.id.clone()))
        })
        .collect();
    let systems: HashMap<_, _> = builder
        .modules
        .iter()
        .flat_map(|module| {
            module
                .descriptor
                .systems
                .iter()
                .map(move |system| (system.id.clone(), (module.descriptor.id.clone(), system)))
        })
        .collect();

    for module in &builder.modules {
        match &module.descriptor.execution {
            ExecutionPlane::Hosted => {
                return Err(SimulationBuildError::UnsupportedExecutionPlane {
                    module: module.descriptor.id.clone(),
                    execution: module.descriptor.execution.clone(),
                });
            }
            ExecutionPlane::Native { backend, adapter } => {
                if backend != &capabilities.backend.id {
                    return Err(SimulationBuildError::BackendMismatch {
                        module: module.descriptor.id.clone(),
                        required: backend.clone(),
                        actual: capabilities.backend.id.clone(),
                    });
                }
                if !adapter.matches(&capabilities.backend.adapter_version) {
                    return Err(SimulationBuildError::IncompatibleAdapter {
                        module: module.descriptor.id.clone(),
                        backend: backend.clone(),
                        required: adapter.clone(),
                        found: capabilities.backend.adapter_version.clone(),
                    });
                }
            }
            _ => {
                return Err(SimulationBuildError::UnsupportedExecutionPlane {
                    module: module.descriptor.id.clone(),
                    execution: module.descriptor.execution.clone(),
                });
            }
        }
        for capability in &module.descriptor.required_capabilities {
            if !capabilities.supports(capability) {
                return Err(SimulationBuildError::UnsupportedCapability {
                    module: module.descriptor.id.clone(),
                    capability: capability.clone(),
                });
            }
        }
        for dependency in &module.descriptor.dependencies {
            let found = modules.get(&dependency.id).ok_or_else(|| {
                SimulationBuildError::MissingDependency {
                    module: module.descriptor.id.clone(),
                    dependency: dependency.id.clone(),
                }
            })?;
            if !dependency.version.matches(&found.version) {
                return Err(SimulationBuildError::IncompatibleDependency {
                    module: module.descriptor.id.clone(),
                    dependency: dependency.id.clone(),
                    required: dependency.version.clone(),
                    found: found.version.clone(),
                });
            }
        }
        let dependency_ids: HashSet<_> = module
            .descriptor
            .dependencies
            .iter()
            .map(|dependency| dependency.id.clone())
            .collect();
        for system in &module.descriptor.systems {
            let trigger_owner = match &system.trigger {
                SystemTrigger::Execute { entry } => entries.get(entry),
                SystemTrigger::Advance { clock_type } => clocks.get(clock_type),
                _ => None,
            }
            .ok_or_else(|| SimulationBuildError::UnknownTrigger {
                module: module.descriptor.id.clone(),
                system: system.id.clone(),
                trigger: system.trigger.clone(),
            })?;
            if trigger_owner != &module.descriptor.id && !dependency_ids.contains(trigger_owner) {
                return Err(SimulationBuildError::UnknownTrigger {
                    module: module.descriptor.id.clone(),
                    system: system.id.clone(),
                    trigger: system.trigger.clone(),
                });
            }
            for target in system.before.iter().chain(&system.after) {
                if target == &system.id {
                    return Err(SimulationBuildError::InvalidOrdering {
                        system: system.id.clone(),
                        target: target.clone(),
                        reason: OrderingError::SelfReference,
                    });
                }
                let (owner, target_system) =
                    systems
                        .get(target)
                        .ok_or_else(|| SimulationBuildError::InvalidOrdering {
                            system: system.id.clone(),
                            target: target.clone(),
                            reason: OrderingError::UnknownSystem,
                        })?;
                if target_system.trigger != system.trigger {
                    return Err(SimulationBuildError::InvalidOrdering {
                        system: system.id.clone(),
                        target: target.clone(),
                        reason: OrderingError::DifferentTrigger,
                    });
                }
                if owner != &module.descriptor.id && !dependency_ids.contains(owner) {
                    return Err(SimulationBuildError::InvalidOrdering {
                        system: system.id.clone(),
                        target: target.clone(),
                        reason: OrderingError::UnknownSystem,
                    });
                }
            }
        }
    }
    validate_cycles(&systems)
}

fn validate_cycles(
    systems: &HashMap<SystemId, (ModuleId, &SystemDefinition)>,
) -> Result<(), SimulationBuildError> {
    let mut by_trigger: HashMap<SystemTrigger, Vec<SystemId>> = HashMap::new();
    for system in systems.values().map(|(_, system)| *system) {
        by_trigger
            .entry(system.trigger.clone())
            .or_default()
            .push(system.id.clone());
    }
    for (trigger, mut ids) in by_trigger {
        ids.sort();
        let mut edges: HashMap<SystemId, Vec<SystemId>> = HashMap::new();
        let mut indegree: HashMap<SystemId, usize> =
            ids.iter().cloned().map(|id| (id, 0)).collect();
        for id in &ids {
            let (_, system) = systems
                .get(id)
                .expect("system IDs originate from the same complete map");
            for target in &system.before {
                edges.entry(id.clone()).or_default().push(target.clone());
            }
            for target in &system.after {
                edges.entry(target.clone()).or_default().push(id.clone());
            }
        }
        for targets in edges.values_mut() {
            targets.sort();
            targets.dedup();
            for target in targets.iter() {
                let degree = indegree
                    .get_mut(target)
                    .expect("validated ordering targets share the same trigger");
                *degree += 1;
            }
        }
        let mut ready: BTreeSet<_> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut removed = 0;
        while let Some(id) = ready.pop_first() {
            removed += 1;
            if let Some(targets) = edges.get(&id) {
                for target in targets {
                    let degree = indegree
                        .get_mut(target)
                        .expect("validated ordering targets share the same trigger");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.insert(target.clone());
                    }
                }
            }
        }
        if removed != ids.len() {
            let unresolved = ids
                .into_iter()
                .filter(|id| indegree.get(id).is_some_and(|degree| *degree > 0))
                .collect();
            return Err(SimulationBuildError::OrderingCycle {
                trigger,
                systems: unresolved,
            });
        }
    }
    Ok(())
}

fn validate_json_targets(request: &AdvanceRequest) -> Result<(), SimulationError> {
    if request.targets.is_empty() {
        return Err(SimulationError::InvalidAdvanceRequest {
            clock_type: request.clock_type.clone(),
            reason: AdvanceRequestViolation::EmptyTargets,
        });
    }
    let mut seen = HashSet::new();
    for target in &request.targets {
        if !seen.insert(target.instance.clone()) {
            return Err(SimulationError::InvalidAdvanceRequest {
                clock_type: request.clock_type.clone(),
                reason: AdvanceRequestViolation::DuplicateInstance {
                    instance: target.instance.clone(),
                },
            });
        }
    }
    Ok(())
}

fn validate_typed_targets<S>(
    clock_type: &ClockTypeId,
    targets: &[TypedAdvanceTarget<S>],
) -> Result<(), SimulationError> {
    if targets.is_empty() {
        return Err(SimulationError::InvalidAdvanceRequest {
            clock_type: clock_type.clone(),
            reason: AdvanceRequestViolation::EmptyTargets,
        });
    }
    let mut seen = HashSet::new();
    for target in targets {
        if !seen.insert(target.instance.clone()) {
            return Err(SimulationError::InvalidAdvanceRequest {
                clock_type: clock_type.clone(),
                reason: AdvanceRequestViolation::DuplicateInstance {
                    instance: target.instance.clone(),
                },
            });
        }
    }
    Ok(())
}

fn is_fatal(error: &SimulationError) -> bool {
    matches!(
        error,
        SimulationError::UnexpectedExecuteOutput { .. }
            | SimulationError::ExecuteOutputEncodingFailed { .. }
            | SimulationError::MissingExecuteOutput { .. }
            | SimulationError::ConflictingExecuteOutput { .. }
            | SimulationError::InvalidExecuteOutput { .. }
            | SimulationError::SystemFailed { .. }
            | SimulationError::BackendFailure { .. }
            | SimulationError::BackendPanicked { .. }
    )
}

fn backend_failure(operation: SimulationOperation, code: &str) -> SimulationError {
    SimulationError::BackendFailure {
        backend: backend_id(),
        operation,
        code: format!("armillae.simulate/{code}"),
        message: "Bevy backend invariant failed".to_owned(),
    }
}

fn codec_code() -> ClockErrorCode {
    ClockErrorCode::new("armillae.simulate/codec")
        .expect("hard-coded clock codec error code is valid visible ASCII")
}

fn codec_value_error(key: &ClockKey) -> SimulationError {
    SimulationError::ClockValueRejected {
        key: key.clone(),
        code: codec_code(),
        message: "clock codec failed".to_owned(),
    }
}

fn codec_transition_error(clock_type: &ClockTypeId, instance: &ClockInstanceId) -> SimulationError {
    SimulationError::ClockTransitionFailed {
        clock_type: clock_type.clone(),
        instance: instance.clone(),
        code: codec_code(),
        message: "clock codec failed".to_owned(),
    }
}

use std::{
    any::TypeId,
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
};

use armillae_simulate::{
    Clock, ClockTypeId, ModuleDescriptor, SIMULATE_API_VERSION, SimulationBuildError,
    SystemDefinition, SystemExecutionResult, SystemId,
};
use bevy_ecs::{
    prelude::{In, Res},
    system::{BoxedSystem, IntoSystem},
};

use crate::{
    BevySimulation,
    runtime::{advance_json_for, insert_json_for, read_json_for, remove_json_for},
    support::{CompiledSchema, SystemFailureCollector},
};

pub trait BevyModule: Send + 'static {
    fn descriptor(&self) -> ModuleDescriptor;

    fn register(
        self: Box<Self>,
        registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError>;
}

pub(crate) type InsertJsonFn = fn(
    &mut BevySimulation,
    armillae_simulate::ClockState,
) -> Result<(), armillae_simulate::SimulationError>;
pub(crate) type ReadJsonFn =
    fn(
        &BevySimulation,
        &armillae_simulate::ClockKey,
    ) -> Result<armillae_simulate::ClockState, armillae_simulate::SimulationError>;
pub(crate) type RemoveJsonFn =
    fn(
        &mut BevySimulation,
        &armillae_simulate::ClockKey,
    ) -> Result<armillae_simulate::ClockState, armillae_simulate::SimulationError>;
pub(crate) type AdvanceJsonFn =
    fn(
        &mut BevySimulation,
        armillae_simulate::AdvanceRequest,
    ) -> Result<armillae_simulate::AdvanceOutcome, armillae_simulate::SimulationError>;

pub(crate) struct ClockRegistration {
    pub(crate) clock_type: ClockTypeId,
    pub(crate) rust_type: TypeId,
    pub(crate) insert_json: InsertJsonFn,
    pub(crate) read_json: ReadJsonFn,
    pub(crate) remove_json: RemoveJsonFn,
    pub(crate) advance_json: AdvanceJsonFn,
}

pub(crate) struct RegisteredSystem {
    pub(crate) definition: SystemDefinition,
    pub(crate) implementation: BoxedSystem<(), ()>,
}

pub(crate) struct RegisteredModule {
    pub(crate) descriptor: ModuleDescriptor,
    pub(crate) clocks: Vec<ClockRegistration>,
    pub(crate) systems: Vec<RegisteredSystem>,
}

pub struct BevyModuleRegistrar<'a> {
    descriptor: &'a ModuleDescriptor,
    clocks: Vec<ClockRegistration>,
    systems: Vec<RegisteredSystem>,
    bound_clock_ids: HashSet<ClockTypeId>,
    bound_clock_types: HashSet<TypeId>,
    bound_systems: HashSet<SystemId>,
}

impl<'a> BevyModuleRegistrar<'a> {
    fn new(descriptor: &'a ModuleDescriptor) -> Self {
        Self {
            descriptor,
            clocks: Vec::new(),
            systems: Vec::new(),
            bound_clock_ids: HashSet::new(),
            bound_clock_types: HashSet::new(),
            bound_systems: HashSet::new(),
        }
    }

    fn registration_error(&self, code: &str, message: &str) -> SimulationBuildError {
        SimulationBuildError::NativeRegistrationFailed {
            module: Some(self.descriptor.id.clone()),
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    pub fn bind_clock<C>(&mut self, clock_type: &ClockTypeId) -> Result<(), SimulationBuildError>
    where
        C: Clock,
    {
        if !self
            .descriptor
            .clocks
            .iter()
            .any(|definition| &definition.id == clock_type)
        {
            return Err(self.registration_error(
                "armillae.simulate/undeclared_clock_binding",
                "native clock binding is not declared by the module",
            ));
        }
        if !self.bound_clock_ids.insert(clock_type.clone())
            || !self.bound_clock_types.insert(TypeId::of::<C>())
        {
            return Err(self.registration_error(
                "armillae.simulate/duplicate_clock_binding",
                "native clock binding is duplicated",
            ));
        }
        self.clocks.push(ClockRegistration {
            clock_type: clock_type.clone(),
            rust_type: TypeId::of::<C>(),
            insert_json: insert_json_for::<C>,
            read_json: read_json_for::<C>,
            remove_json: remove_json_for::<C>,
            advance_json: advance_json_for::<C>,
        });
        Ok(())
    }

    fn system_definition(
        &self,
        system: &SystemId,
    ) -> Result<SystemDefinition, SimulationBuildError> {
        self.descriptor
            .systems
            .iter()
            .find(|definition| &definition.id == system)
            .cloned()
            .ok_or_else(|| {
                self.registration_error(
                    "armillae.simulate/undeclared_system_binding",
                    "native system binding is not declared by the module",
                )
            })
    }

    fn reserve_system(
        &mut self,
        system: &SystemId,
    ) -> Result<SystemDefinition, SimulationBuildError> {
        let definition = self.system_definition(system)?;
        if !self.bound_systems.insert(system.clone()) {
            return Err(self.registration_error(
                "armillae.simulate/duplicate_system_binding",
                "native system binding is duplicated",
            ));
        }
        Ok(definition)
    }

    pub fn add_system<M, S>(
        &mut self,
        system: &SystemId,
        implementation: S,
    ) -> Result<(), SimulationBuildError>
    where
        S: IntoSystem<(), (), M> + 'static,
    {
        let definition = self.reserve_system(system)?;
        self.systems.push(RegisteredSystem {
            definition,
            implementation: Box::new(IntoSystem::into_system(implementation)),
        });
        Ok(())
    }

    pub fn add_fallible_system<M, S>(
        &mut self,
        system: &SystemId,
        implementation: S,
    ) -> Result<(), SimulationBuildError>
    where
        S: IntoSystem<(), SystemExecutionResult, M> + 'static,
    {
        let definition = self.reserve_system(system)?;
        let logical_id = system.clone();
        let piped = implementation.pipe(
            move |In(result): In<SystemExecutionResult>, collector: Res<SystemFailureCollector>| {
                if let Err(error) = result {
                    collector.push(logical_id.clone(), error);
                }
            },
        );
        self.systems.push(RegisteredSystem {
            definition,
            implementation: Box::new(IntoSystem::into_system(piped)),
        });
        Ok(())
    }

    fn finish(
        self,
    ) -> Result<(Vec<ClockRegistration>, Vec<RegisteredSystem>), SimulationBuildError> {
        if self.bound_clock_ids.len() != self.descriptor.clocks.len() {
            return Err(self.registration_error(
                "armillae.simulate/missing_clock_binding",
                "one or more declared clocks are not bound",
            ));
        }
        if self.bound_systems.len() != self.descriptor.systems.len() {
            return Err(self.registration_error(
                "armillae.simulate/missing_system_binding",
                "one or more declared systems are not bound",
            ));
        }
        Ok((self.clocks, self.systems))
    }
}

#[derive(Default)]
pub struct BevySimulationBuilder {
    pub(crate) modules: Vec<RegisteredModule>,
}

impl BevySimulationBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_module<M>(&mut self, module: M) -> Result<(), SimulationBuildError>
    where
        M: BevyModule,
    {
        self.register_boxed_module(Box::new(module))
    }

    pub fn register_boxed_module(
        &mut self,
        module: Box<dyn BevyModule>,
    ) -> Result<(), SimulationBuildError> {
        let descriptor = catch_unwind(AssertUnwindSafe(|| module.descriptor())).map_err(|_| {
            SimulationBuildError::NativeRegistrationFailed {
                module: None,
                code: "armillae.simulate/native_module_panicked".to_owned(),
                message: "native module panicked".to_owned(),
            }
        })?;
        validate_descriptor(&descriptor)?;
        let module_id = descriptor.id.clone();
        let mut registrar = BevyModuleRegistrar::new(&descriptor);
        let registration = catch_unwind(AssertUnwindSafe(|| module.register(&mut registrar)));
        match registration {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(SimulationBuildError::NativeRegistrationFailed {
                    module: Some(module_id),
                    code: "armillae.simulate/native_module_panicked".to_owned(),
                    message: "native module panicked".to_owned(),
                });
            }
        }
        let (clocks, systems) = registrar.finish()?;
        self.check_global_conflicts(&descriptor, &clocks)?;
        self.modules.push(RegisteredModule {
            descriptor,
            clocks,
            systems,
        });
        Ok(())
    }

    fn check_global_conflicts(
        &self,
        incoming: &ModuleDescriptor,
        incoming_clocks: &[ClockRegistration],
    ) -> Result<(), SimulationBuildError> {
        for existing in &self.modules {
            if existing.descriptor.id == incoming.id {
                return Err(SimulationBuildError::DuplicateModule {
                    module: incoming.id.clone(),
                });
            }
            for entry in &incoming.execute_entries {
                if existing
                    .descriptor
                    .execute_entries
                    .iter()
                    .any(|candidate| candidate.id == entry.id)
                {
                    return Err(SimulationBuildError::DuplicateExecuteEntry {
                        entry: entry.id.clone(),
                        first: existing.descriptor.id.clone(),
                        second: incoming.id.clone(),
                    });
                }
            }
            for clock in &incoming.clocks {
                if existing
                    .descriptor
                    .clocks
                    .iter()
                    .any(|candidate| candidate.id == clock.id)
                {
                    return Err(SimulationBuildError::DuplicateClockType {
                        clock_type: clock.id.clone(),
                        first: existing.descriptor.id.clone(),
                        second: incoming.id.clone(),
                    });
                }
            }
            if incoming_clocks.iter().any(|incoming_clock| {
                existing
                    .clocks
                    .iter()
                    .any(|existing_clock| existing_clock.rust_type == incoming_clock.rust_type)
            }) {
                return Err(SimulationBuildError::NativeRegistrationFailed {
                    module: Some(incoming.id.clone()),
                    code: "armillae.simulate/rust_clock_type_rebound".to_owned(),
                    message: "a Rust Clock type is bound more than once".to_owned(),
                });
            }
            for system in &incoming.systems {
                if existing
                    .descriptor
                    .systems
                    .iter()
                    .any(|candidate| candidate.id == system.id)
                {
                    return Err(SimulationBuildError::DuplicateSystem {
                        system: system.id.clone(),
                        first: existing.descriptor.id.clone(),
                        second: incoming.id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn activate(self) -> Result<BevySimulation, SimulationBuildError> {
        BevySimulation::from_builder(self)
    }
}

fn duplicate<T>(values: impl IntoIterator<Item = T>) -> bool
where
    T: Eq + std::hash::Hash,
{
    let mut seen = HashSet::new();
    values.into_iter().any(|value| !seen.insert(value))
}

fn validate_descriptor(descriptor: &ModuleDescriptor) -> Result<(), SimulationBuildError> {
    let invalid = |code: &str, message: &str| SimulationBuildError::InvalidDescriptor {
        module: Some(descriptor.id.clone()),
        code: code.to_owned(),
        message: message.to_owned(),
    };
    if descriptor.api_version != SIMULATE_API_VERSION {
        return Err(invalid(
            "armillae.simulate/api_version",
            "module API version is not supported",
        ));
    }
    if duplicate(
        descriptor
            .dependencies
            .iter()
            .map(|dependency| dependency.id.clone()),
    ) || descriptor
        .dependencies
        .iter()
        .any(|dependency| dependency.id == descriptor.id)
    {
        return Err(invalid(
            "armillae.simulate/dependencies",
            "module dependencies contain a duplicate or self reference",
        ));
    }
    if duplicate(
        descriptor
            .execute_entries
            .iter()
            .map(|entry| entry.id.clone()),
    ) || duplicate(descriptor.clocks.iter().map(|clock| clock.id.clone()))
        || duplicate(descriptor.systems.iter().map(|system| system.id.clone()))
    {
        return Err(invalid(
            "armillae.simulate/duplicate_descriptor_id",
            "module descriptor contains duplicate logical IDs",
        ));
    }
    for entry in &descriptor.execute_entries {
        CompiledSchema::build(
            &entry.input_schema,
            Some(descriptor.id.clone()),
            "armillae.simulate/execute_input_schema",
        )?;
        if let Some(output) = &entry.output_schema {
            CompiledSchema::build(
                output,
                Some(descriptor.id.clone()),
                "armillae.simulate/execute_output_schema",
            )?;
        }
    }
    for clock in &descriptor.clocks {
        CompiledSchema::build(
            &clock.value_schema,
            Some(descriptor.id.clone()),
            "armillae.simulate/clock_value_schema",
        )?;
        CompiledSchema::build(
            &clock.step_schema,
            Some(descriptor.id.clone()),
            "armillae.simulate/clock_step_schema",
        )?;
    }
    Ok(())
}

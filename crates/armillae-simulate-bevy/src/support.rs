use std::sync::{Mutex, MutexGuard};

use armillae_simulate::{
    BackendId, CapabilityId, SchemaViolation, SemanticVersion, SimulationBuildError,
    SimulationCapabilities, SystemExecutionError, SystemId,
};
use bevy_ecs::{
    error::{BevyError, ErrorContext},
    prelude::{Resource, SystemSet},
};

pub const BEVY_BACKEND_ID: &str = "armillae.simulate/bevy";
pub const BEVY_ENGINE_NAME: &str = "bevy_ecs";

pub(crate) const NATIVE_MODULES_CAPABILITY: &str = "armillae.simulate/native_modules";
pub(crate) const BACKEND_NATIVE_ACCESS_CAPABILITY: &str = "armillae.simulate/backend_native_access";
pub(crate) const PARALLEL_SYSTEMS_CAPABILITY: &str = "armillae.simulate/parallel_systems";

pub(crate) fn backend_id() -> BackendId {
    BackendId::new(BEVY_BACKEND_ID).expect("hard-coded Bevy backend ID is valid visible ASCII")
}

pub(crate) fn capability_id(value: &str) -> CapabilityId {
    CapabilityId::new(value).expect("hard-coded capability ID is valid visible ASCII")
}

pub(crate) fn package_version() -> SemanticVersion {
    SemanticVersion::new(env!("CARGO_PKG_VERSION")).expect("Cargo package version is valid SemVer")
}

pub(crate) fn engine_version() -> SemanticVersion {
    SemanticVersion::new("0.19.1").expect("locked Bevy engine version is valid SemVer")
}

pub(crate) fn capabilities() -> SimulationCapabilities {
    let mut supported = std::collections::BTreeSet::from([
        capability_id(NATIVE_MODULES_CAPABILITY),
        capability_id(BACKEND_NATIVE_ACCESS_CAPABILITY),
    ]);
    if cfg!(all(feature = "parallel", not(target_arch = "wasm32"))) {
        supported.insert(capability_id(PARALLEL_SYSTEMS_CAPABILITY));
    }
    SimulationCapabilities {
        backend: armillae_simulate::BackendInfo {
            id: backend_id(),
            adapter_version: package_version(),
            engine: Some(armillae_simulate::EngineInfo {
                name: BEVY_ENGINE_NAME.to_owned(),
                version: engine_version(),
            }),
        },
        supported,
    }
}

pub(crate) struct CompiledSchema(jsonschema::Validator);

impl CompiledSchema {
    pub(crate) fn build(
        schema: &serde_json::Value,
        module: Option<armillae_simulate::ModuleId>,
        code: &str,
    ) -> Result<Self, SimulationBuildError> {
        if !schema.is_object() || !jsonschema::draft202012::meta::is_valid(schema) {
            return Err(SimulationBuildError::InvalidDescriptor {
                module,
                code: code.to_owned(),
                message: "JSON Schema must be a valid Draft 2020-12 object".to_owned(),
            });
        }
        jsonschema::draft202012::options()
            .build(schema)
            .map(Self)
            .map_err(|_| SimulationBuildError::InvalidDescriptor {
                module,
                code: code.to_owned(),
                message: "JSON Schema compilation failed".to_owned(),
            })
    }

    pub(crate) fn violations(&self, instance: &serde_json::Value) -> Vec<SchemaViolation> {
        let mut violations: Vec<_> = self
            .0
            .iter_errors(instance)
            .map(|error| {
                let schema_path = error.schema_path.to_string();
                let keyword = schema_path
                    .rsplit('/')
                    .next()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                SchemaViolation {
                    instance_path: error.instance_path.to_string(),
                    schema_path,
                    keyword,
                }
            })
            .collect();
        violations.sort_by(|left, right| {
            (
                left.instance_path.as_str(),
                left.schema_path.as_str(),
                left.keyword.as_deref().unwrap_or(""),
            )
                .cmp(&(
                    right.instance_path.as_str(),
                    right.schema_path.as_str(),
                    right.keyword.as_deref().unwrap_or(""),
                ))
        });
        violations.dedup();
        violations
    }
}

#[derive(Resource, Default)]
pub(crate) struct SystemFailureCollector(
    Mutex<Vec<(SystemId, armillae_simulate::SystemExecutionError)>>,
);

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl SystemFailureCollector {
    pub(crate) fn clear(&self) {
        lock_recover(&self.0).clear();
    }

    pub(crate) fn push(&self, system: SystemId, error: SystemExecutionError) {
        lock_recover(&self.0).push((system, error));
    }

    pub(crate) fn first(&self) -> Option<(SystemId, SystemExecutionError)> {
        let mut failures = lock_recover(&self.0).clone();
        failures.sort_by(|left, right| left.0.cmp(&right.0));
        failures.into_iter().next()
    }
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LogicalSystemSet(pub(crate) SystemId);

#[derive(Debug)]
pub(crate) struct UnhandledBevyErrorMarker;

pub(crate) fn redacting_fallback_handler(_error: BevyError, _context: ErrorContext) {
    std::panic::panic_any(UnhandledBevyErrorMarker);
}

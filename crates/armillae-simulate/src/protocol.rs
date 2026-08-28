use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    BackendId, CapabilityId, Clock, ClockInstanceId, ClockTypeId, ExecuteEntryId, ModuleId,
    SemanticVersion, SystemId, VersionRequirement,
};

pub const SIMULATE_API_VERSION: &str = "armillae.simulate/v1alpha1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteRequest {
    pub entry: ExecuteEntryId,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteOutcome {
    pub entry: ExecuteEntryId,
    pub output: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ClockKey {
    pub clock_type: ClockTypeId,
    pub instance: ClockInstanceId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockState {
    pub key: ClockKey,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceTarget {
    pub instance: ClockInstanceId,
    pub step: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceRequest {
    pub clock_type: ClockTypeId,
    #[serde(default)]
    pub targets: Vec<AdvanceTarget>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockTransition {
    pub instance: ClockInstanceId,
    pub before: serde_json::Value,
    pub step: serde_json::Value,
    pub after: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceOutcome {
    pub clock_type: ClockTypeId,
    #[serde(default)]
    pub transitions: Vec<ClockTransition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDescriptor {
    pub api_version: String,
    pub id: ModuleId,
    pub version: SemanticVersion,
    #[serde(default)]
    pub dependencies: Vec<ModuleDependency>,
    pub execution: ExecutionPlane,
    #[serde(default)]
    pub required_capabilities: BTreeSet<CapabilityId>,
    #[serde(default)]
    pub execute_entries: Vec<ExecuteEntryDefinition>,
    #[serde(default)]
    pub clocks: Vec<ClockDefinition>,
    #[serde(default)]
    pub systems: Vec<SystemDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDependency {
    pub id: ModuleId,
    pub version: VersionRequirement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExecutionPlane {
    Native {
        backend: BackendId,
        adapter: VersionRequirement,
    },
    Hosted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteEntryDefinition {
    pub id: ExecuteEntryId,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
}

impl ExecuteEntryDefinition {
    pub fn for_input<I: JsonSchema>(id: ExecuteEntryId) -> Self {
        Self {
            id,
            input_schema: schema_value::<I>(),
            output_schema: None,
        }
    }

    pub fn for_input_output<I: JsonSchema, O: JsonSchema>(id: ExecuteEntryId) -> Self {
        Self {
            id,
            input_schema: schema_value::<I>(),
            output_schema: Some(schema_value::<O>()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockDefinition {
    pub id: ClockTypeId,
    pub value_schema: serde_json::Value,
    pub step_schema: serde_json::Value,
}

impl ClockDefinition {
    pub fn for_clock<C: Clock>(id: ClockTypeId) -> Self {
        Self {
            id,
            value_schema: schema_value::<C>(),
            step_schema: schema_value::<C::Step>(),
        }
    }
}

fn schema_value<T: JsonSchema>() -> serde_json::Value {
    schemars::schema_for!(T).to_value()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SystemTrigger {
    Execute { entry: ExecuteEntryId },
    Advance { clock_type: ClockTypeId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemDefinition {
    pub id: SystemId,
    pub trigger: SystemTrigger,
    #[serde(default)]
    pub before: Vec<SystemId>,
    #[serde(default)]
    pub after: Vec<SystemId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EngineInfo {
    pub name: String,
    pub version: SemanticVersion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BackendInfo {
    pub id: BackendId,
    pub adapter_version: SemanticVersion,
    pub engine: Option<EngineInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SimulationCapabilities {
    pub backend: BackendInfo,
    #[serde(default)]
    pub supported: BTreeSet<CapabilityId>,
}

impl SimulationCapabilities {
    pub fn supports(&self, capability: &CapabilityId) -> bool {
        self.supported.contains(capability)
    }
}

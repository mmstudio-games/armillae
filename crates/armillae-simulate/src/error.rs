use crate::{
    BackendId, CapabilityId, ClockErrorCode, ClockInstanceId, ClockKey, ClockTypeId,
    ExecuteEntryId, ExecutionPlane, ModuleId, SemanticVersion, SimulationStatus, SystemErrorCode,
    SystemId, SystemTrigger, VersionRequirement,
};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimulationBuildError {
    #[error("invalid module descriptor: {code}: {message}")]
    InvalidDescriptor {
        module: Option<ModuleId>,
        code: String,
        message: String,
    },
    #[error("duplicate module `{module}`")]
    DuplicateModule { module: ModuleId },
    #[error("duplicate execute entry `{entry}`")]
    DuplicateExecuteEntry {
        entry: ExecuteEntryId,
        first: ModuleId,
        second: ModuleId,
    },
    #[error("duplicate clock type `{clock_type}`")]
    DuplicateClockType {
        clock_type: ClockTypeId,
        first: ModuleId,
        second: ModuleId,
    },
    #[error("duplicate system `{system}`")]
    DuplicateSystem {
        system: SystemId,
        first: ModuleId,
        second: ModuleId,
    },
    #[error("module `{module}` requires missing module `{dependency}`")]
    MissingDependency {
        module: ModuleId,
        dependency: ModuleId,
    },
    #[error("module `{module}` has an incompatible dependency")]
    IncompatibleDependency {
        module: ModuleId,
        dependency: ModuleId,
        required: VersionRequirement,
        found: SemanticVersion,
    },
    #[error("system `{system}` references an unknown trigger")]
    UnknownTrigger {
        module: ModuleId,
        system: SystemId,
        trigger: SystemTrigger,
    },
    #[error("invalid ordering edge from `{system}` to `{target}`")]
    InvalidOrdering {
        system: SystemId,
        target: SystemId,
        reason: OrderingError,
    },
    #[error("system ordering contains a cycle")]
    OrderingCycle {
        trigger: SystemTrigger,
        systems: Vec<SystemId>,
    },
    #[error("module `{module}` requires unsupported capability `{capability}`")]
    UnsupportedCapability {
        module: ModuleId,
        capability: CapabilityId,
    },
    #[error("module `{module}` uses an unsupported execution plane")]
    UnsupportedExecutionPlane {
        module: ModuleId,
        execution: ExecutionPlane,
    },
    #[error("module `{module}` targets a different backend")]
    BackendMismatch {
        module: ModuleId,
        required: BackendId,
        actual: BackendId,
    },
    #[error("module `{module}` requires an incompatible adapter version")]
    IncompatibleAdapter {
        module: ModuleId,
        backend: BackendId,
        required: VersionRequirement,
        found: SemanticVersion,
    },
    #[error("native registration failed for module {module:?}: {code}")]
    NativeRegistrationFailed {
        module: Option<ModuleId>,
        code: String,
        message: String,
    },
    #[error("failed to build system graph")]
    SystemGraphBuildFailed {
        trigger: SystemTrigger,
        code: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderingError {
    SelfReference,
    UnknownSystem,
    DifferentTrigger,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("system execution failed: {code}: {message}")]
pub struct SystemExecutionError {
    pub code: SystemErrorCode,
    pub message: String,
}

pub type SystemExecutionResult = Result<(), SystemExecutionError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimulationOperation {
    Execute,
    ReadClock,
    InsertClock,
    RemoveClock,
    Advance,
    InspectWorld,
    WriteWorld,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaViolation {
    pub instance_path: String,
    pub schema_path: String,
    pub keyword: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdvanceRequestViolation {
    EmptyTargets,
    DuplicateInstance { instance: ClockInstanceId },
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimulationError {
    #[error("cannot perform {operation:?} while simulation is {status:?}")]
    InvalidState {
        operation: SimulationOperation,
        status: SimulationStatus,
    },
    #[error("cannot perform {operation:?} because simulation is faulted")]
    Faulted { operation: SimulationOperation },
    #[error("unknown execute entry `{entry}`")]
    UnknownExecuteEntry { entry: ExecuteEntryId },
    #[error("unknown clock type `{clock_type}`")]
    UnknownClockType { clock_type: ClockTypeId },
    #[error("native clock type `{rust_type}` is not bound")]
    NativeClockTypeNotBound { rust_type: &'static str },
    #[error("unknown clock instance")]
    UnknownClockInstance { key: ClockKey },
    #[error("clock instance already exists")]
    DuplicateClockInstance { key: ClockKey },
    #[error("invalid execute input for `{entry}`")]
    InvalidExecuteInput {
        entry: ExecuteEntryId,
        violations: Vec<SchemaViolation>,
    },
    #[error("execute entry `{entry}` does not declare output")]
    UnexpectedExecuteOutput { entry: ExecuteEntryId },
    #[error("failed to encode output for `{entry}`")]
    ExecuteOutputEncodingFailed { entry: ExecuteEntryId },
    #[error("execute entry `{entry}` did not produce required output")]
    MissingExecuteOutput { entry: ExecuteEntryId },
    #[error("execute entry `{entry}` produced output more than once")]
    ConflictingExecuteOutput { entry: ExecuteEntryId },
    #[error("invalid execute output for `{entry}`")]
    InvalidExecuteOutput {
        entry: ExecuteEntryId,
        violations: Vec<SchemaViolation>,
    },
    #[error("invalid clock value")]
    InvalidClockValue {
        key: ClockKey,
        violations: Vec<SchemaViolation>,
    },
    #[error("clock value rejected: {code}: {message}")]
    ClockValueRejected {
        key: ClockKey,
        code: ClockErrorCode,
        message: String,
    },
    #[error("invalid advance request")]
    InvalidAdvanceRequest {
        clock_type: ClockTypeId,
        reason: AdvanceRequestViolation,
    },
    #[error("invalid clock step")]
    InvalidClockStep {
        clock_type: ClockTypeId,
        instance: ClockInstanceId,
        violations: Vec<SchemaViolation>,
    },
    #[error("clock transition failed: {code}: {message}")]
    ClockTransitionFailed {
        clock_type: ClockTypeId,
        instance: ClockInstanceId,
        code: ClockErrorCode,
        message: String,
    },
    #[error("system `{system}` failed: {code}: {message}")]
    SystemFailed {
        system: SystemId,
        trigger: SystemTrigger,
        code: SystemErrorCode,
        message: String,
    },
    #[error("backend `{backend}` failed: {code}: {message}")]
    BackendFailure {
        backend: BackendId,
        operation: SimulationOperation,
        code: String,
        message: String,
    },
    #[error("backend `{backend}` panicked during {operation:?}")]
    BackendPanicked {
        backend: BackendId,
        operation: SimulationOperation,
    },
}

impl SimulationError {
    #[cfg(feature = "testing")]
    pub(crate) fn faults_simulation(&self) -> bool {
        matches!(
            self,
            Self::UnexpectedExecuteOutput { .. }
                | Self::ExecuteOutputEncodingFailed { .. }
                | Self::MissingExecuteOutput { .. }
                | Self::ConflictingExecuteOutput { .. }
                | Self::InvalidExecuteOutput { .. }
                | Self::SystemFailed { .. }
                | Self::BackendFailure { .. }
                | Self::BackendPanicked { .. }
        )
    }
}

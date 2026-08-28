use std::sync::{Arc, Mutex, MutexGuard};

use armillae_simulate::{
    Clock, ClockInstanceId, ClockTypeId, ExecuteEntryId, ExecuteRequest, SystemErrorCode,
    SystemExecutionError, TypedClockTransition,
};
use bevy_ecs::prelude::{Component, Resource};

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Default)]
pub(crate) struct ExecuteOutputState {
    pub(crate) attempts: usize,
    pub(crate) not_declared: bool,
    pub(crate) encoding_failed: bool,
    pub(crate) output: Option<serde_json::Value>,
}

pub(crate) struct ExecuteOutputSink {
    declared: bool,
    state: Mutex<ExecuteOutputState>,
}

impl ExecuteOutputSink {
    pub(crate) fn new(declared: bool) -> Arc<Self> {
        Arc::new(Self {
            declared,
            state: Mutex::new(ExecuteOutputState::default()),
        })
    }

    pub(crate) fn snapshot(&self) -> ExecuteOutputState {
        let state = lock_recover(&self.state);
        ExecuteOutputState {
            attempts: state.attempts,
            not_declared: state.not_declared,
            encoding_failed: state.encoding_failed,
            output: state.output.clone(),
        }
    }

    fn set<T>(&self, output: &T, entry: &ExecuteEntryId) -> Result<(), ExecuteOutputError>
    where
        T: serde::Serialize + ?Sized,
    {
        let mut state = lock_recover(&self.state);
        state.attempts += 1;
        if !self.declared {
            state.not_declared = true;
            return Err(ExecuteOutputError::NotDeclared {
                entry: entry.clone(),
            });
        }
        if state.attempts > 1 {
            return Err(ExecuteOutputError::AlreadySet {
                entry: entry.clone(),
            });
        }
        match serde_json::to_value(output) {
            Ok(value) => {
                state.output = Some(value);
                Ok(())
            }
            Err(_) => {
                state.encoding_failed = true;
                Err(ExecuteOutputError::Encoding {
                    entry: entry.clone(),
                })
            }
        }
    }
}

#[derive(Resource)]
pub struct ExecuteContext {
    request: ExecuteRequest,
    sink: Arc<ExecuteOutputSink>,
}

impl ExecuteContext {
    pub(crate) fn new(request: ExecuteRequest, sink: Arc<ExecuteOutputSink>) -> Self {
        Self { request, sink }
    }

    pub fn request(&self) -> &ExecuteRequest {
        &self.request
    }

    pub fn decode<T>(&self) -> Result<T, serde_json::Error>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_value(self.request.input.clone())
    }

    pub fn set_output<T>(&self, output: &T) -> Result<(), ExecuteOutputError>
    where
        T: serde::Serialize + ?Sized,
    {
        self.sink.set(output, &self.request.entry)
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecuteOutputError {
    #[error("execute entry `{entry}` does not declare output")]
    NotDeclared { entry: ExecuteEntryId },
    #[error("execute entry `{entry}` output is already set")]
    AlreadySet { entry: ExecuteEntryId },
    #[error("failed to encode output for `{entry}`")]
    Encoding { entry: ExecuteEntryId },
}

impl From<ExecuteOutputError> for SystemExecutionError {
    fn from(error: ExecuteOutputError) -> Self {
        let (code, message) = match error {
            ExecuteOutputError::NotDeclared { .. } => (
                "armillae.simulate/execute_output_not_declared",
                "execute output is not declared",
            ),
            ExecuteOutputError::AlreadySet { .. } => (
                "armillae.simulate/execute_output_already_set",
                "execute output is already set",
            ),
            ExecuteOutputError::Encoding { .. } => (
                "armillae.simulate/execute_output_encoding",
                "execute output encoding failed",
            ),
        };
        SystemExecutionError {
            code: SystemErrorCode::new(code)
                .expect("hard-coded execute output error code is valid visible ASCII"),
            message: message.to_owned(),
        }
    }
}

#[derive(Component)]
pub struct ClockComponent<C: Clock> {
    instance: ClockInstanceId,
    value: C,
}

impl<C: Clock> ClockComponent<C> {
    pub(crate) fn new(instance: ClockInstanceId, value: C) -> Self {
        Self { instance, value }
    }

    pub fn instance(&self) -> &ClockInstanceId {
        &self.instance
    }

    pub fn value(&self) -> &C {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut C {
        &mut self.value
    }
}

#[derive(Resource)]
pub struct AdvanceContext<C: Clock> {
    clock_type: ClockTypeId,
    transitions: Vec<TypedClockTransition<C>>,
}

impl<C: Clock> AdvanceContext<C> {
    pub(crate) fn new(clock_type: ClockTypeId, transitions: Vec<TypedClockTransition<C>>) -> Self {
        Self {
            clock_type,
            transitions,
        }
    }

    pub fn clock_type(&self) -> &ClockTypeId {
        &self.clock_type
    }

    pub fn transitions(&self) -> &[TypedClockTransition<C>] {
        &self.transitions
    }
}

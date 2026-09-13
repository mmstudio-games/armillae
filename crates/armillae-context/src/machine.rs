//! Internal three-state compression pipeline machine (spec §6.2, §6.3).
//!
//! The pipeline state machine is paradigm-owned per spec; this crate-internal
//! helper gives every paradigm the same transition semantics in one place
//! (RFC 0004 §7.2: shared helper modules mitigate paradigm duplication).

use crate::error::ContextError;
use crate::protocol::{CompressionState, CompressionTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Evaluated,
    Prepared,
}

impl State {
    fn as_protocol(self) -> CompressionState {
        match self {
            State::Idle => CompressionState::Idle,
            State::Evaluated => CompressionState::Evaluated,
            State::Prepared => CompressionState::Prepared,
        }
    }
}

/// Three-state compression pipeline machine implementing the spec §6.2
/// transition table. The evaluated target is remembered so that `prepare`
/// with a mismatched target is rejected (`InvalidOperation`).
pub(crate) struct CompressionMachine {
    state: State,
    evaluated: Option<CompressionTarget>,
}

impl CompressionMachine {
    pub(crate) fn new() -> Self {
        Self {
            state: State::Idle,
            evaluated: None,
        }
    }

    pub(crate) fn state(&self) -> CompressionState {
        self.state.as_protocol()
    }

    /// Guard for dialogue writes and manual operations: allowed only in
    /// `Idle`.
    pub(crate) fn require_idle(&self, operation: &'static str) -> Result<(), ContextError> {
        match self.state {
            State::Idle => Ok(()),
            other => Err(invalid_state(
                operation,
                CompressionState::Idle,
                other.as_protocol(),
            )),
        }
    }

    /// Evaluate from `Idle`: `Some` freezes at `Evaluated` and remembers the
    /// target; `None` leaves the pipeline `Idle`.
    pub(crate) fn on_evaluate(
        &mut self,
        target: Option<CompressionTarget>,
    ) -> Result<(), ContextError> {
        self.require_idle("evaluate_compression")?;
        if let Some(target) = target {
            self.evaluated = Some(target);
            self.state = State::Evaluated;
        }
        Ok(())
    }

    /// Prepare from `Evaluated` with a matching target; from `Idle` or
    /// `Prepared`, or with a mismatched target, is an error.
    pub(crate) fn on_prepare(&mut self, target: &CompressionTarget) -> Result<(), ContextError> {
        match self.state {
            State::Idle => Err(invalid_state(
                "prepare_compression",
                CompressionState::Evaluated,
                CompressionState::Idle,
            )),
            State::Evaluated => {
                if self.evaluated.as_ref() != Some(target) {
                    return Err(ContextError::InvalidOperation {
                        message: "prepare_compression target does not match the evaluated target"
                            .to_owned(),
                    });
                }
                self.state = State::Prepared;
                Ok(())
            }
            State::Prepared => Err(invalid_state(
                "prepare_compression",
                CompressionState::Evaluated,
                CompressionState::Prepared,
            )),
        }
    }

    /// Apply from `Prepared`, returning to `Idle`.
    pub(crate) fn on_apply(&mut self) -> Result<(), ContextError> {
        match self.state {
            State::Prepared => {
                self.state = State::Idle;
                self.evaluated = None;
                Ok(())
            }
            other => Err(invalid_state(
                "apply_compression_result",
                CompressionState::Prepared,
                other.as_protocol(),
            )),
        }
    }

    /// Abandon from any state; from `Idle` this is a no-op success.
    pub(crate) fn on_abandon(&mut self) -> Result<(), ContextError> {
        if self.state != State::Idle {
            self.state = State::Idle;
            self.evaluated = None;
        }
        Ok(())
    }
}

fn invalid_state(
    operation: &'static str,
    expected: CompressionState,
    actual: CompressionState,
) -> ContextError {
    ContextError::InvalidState {
        operation,
        expected,
        actual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: u64) -> CompressionTarget {
        CompressionTarget::Section { id }
    }

    fn assert_invalid_state(
        result: Result<(), ContextError>,
        operation: &'static str,
        expected: CompressionState,
        actual: CompressionState,
    ) {
        assert_eq!(
            result,
            Err(ContextError::InvalidState {
                operation,
                expected,
                actual,
            })
        );
    }

    #[test]
    fn fresh_machine_is_idle_and_accepts_writes() {
        let machine = CompressionMachine::new();
        assert_eq!(machine.state(), CompressionState::Idle);
        assert!(machine.require_idle("push_user_input").is_ok());
    }

    #[test]
    fn evaluate_none_stays_idle() {
        let mut machine = CompressionMachine::new();
        machine
            .on_evaluate(None)
            .expect("None leaves the pipeline idle");
        assert_eq!(machine.state(), CompressionState::Idle);
    }

    #[test]
    fn evaluate_some_freezes_and_rejects_writes() {
        let mut machine = CompressionMachine::new();
        machine
            .on_evaluate(Some(target(1)))
            .expect("evaluation from Idle succeeds");
        assert_eq!(machine.state(), CompressionState::Evaluated);
        assert_invalid_state(
            machine.require_idle("apply_model_output"),
            "apply_model_output",
            CompressionState::Idle,
            CompressionState::Evaluated,
        );
    }

    #[test]
    fn prepare_requires_evaluation() {
        let mut machine = CompressionMachine::new();
        assert_invalid_state(
            machine.on_prepare(&target(1)),
            "prepare_compression",
            CompressionState::Evaluated,
            CompressionState::Idle,
        );
    }

    #[test]
    fn prepare_requires_matching_target() {
        let mut machine = CompressionMachine::new();
        machine.on_evaluate(Some(target(1))).expect("evaluate");
        assert!(matches!(
            machine.on_prepare(&target(2)),
            Err(ContextError::InvalidOperation { .. })
        ));
    }

    #[test]
    fn prepare_then_apply_round_trips_to_idle() {
        let mut machine = CompressionMachine::new();
        machine.on_evaluate(Some(target(1))).expect("evaluate");
        machine.on_prepare(&target(1)).expect("prepare");
        assert_eq!(machine.state(), CompressionState::Prepared);
        machine.on_apply().expect("apply");
        assert_eq!(machine.state(), CompressionState::Idle);
    }

    #[test]
    fn prepare_twice_is_rejected() {
        let mut machine = CompressionMachine::new();
        machine.on_evaluate(Some(target(1))).expect("evaluate");
        machine.on_prepare(&target(1)).expect("prepare");
        assert_invalid_state(
            machine.on_prepare(&target(1)),
            "prepare_compression",
            CompressionState::Evaluated,
            CompressionState::Prepared,
        );
    }

    #[test]
    fn apply_without_prepare_is_rejected() {
        let mut machine = CompressionMachine::new();
        assert_invalid_state(
            machine.on_apply(),
            "apply_compression_result",
            CompressionState::Prepared,
            CompressionState::Idle,
        );
    }

    #[test]
    fn evaluate_twice_is_rejected() {
        let mut machine = CompressionMachine::new();
        machine.on_evaluate(Some(target(1))).expect("evaluate");
        assert_invalid_state(
            machine.on_evaluate(Some(target(2))),
            "evaluate_compression",
            CompressionState::Idle,
            CompressionState::Evaluated,
        );
    }

    #[test]
    fn abandon_is_noop_in_idle_and_resets_evaluated_and_prepared() {
        let mut machine = CompressionMachine::new();
        machine.on_abandon().expect("abandon from Idle is a no-op");
        assert_eq!(machine.state(), CompressionState::Idle);

        machine.on_evaluate(Some(target(1))).expect("evaluate");
        machine.on_abandon().expect("abandon from Evaluated");
        assert_eq!(machine.state(), CompressionState::Idle);

        machine.on_evaluate(Some(target(1))).expect("evaluate");
        machine.on_prepare(&target(1)).expect("prepare");
        machine.on_abandon().expect("abandon from Prepared");
        assert_eq!(machine.state(), CompressionState::Idle);
    }
}

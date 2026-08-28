#![cfg(feature = "testing")]

use std::collections::BTreeSet;

use armillae_simulate::{testing::*, *};
use serde_json::json;

fn id<T>(value: &str) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    value.parse().expect("test identifier is valid")
}

fn capabilities() -> SimulationCapabilities {
    SimulationCapabilities {
        backend: BackendInfo {
            id: id("armillae.test/scripted"),
            adapter_version: SemanticVersion::new("1.0.0").expect("valid version"),
            engine: None,
        },
        supported: BTreeSet::new(),
    }
}

#[test]
fn scripted_simulation_records_calls_and_enforces_stop_lifecycle() {
    let key = ClockKey {
        clock_type: id("armillae.test/clock"),
        instance: id("primary"),
    };
    let clock = ClockState {
        key: key.clone(),
        value: json!(1),
    };
    let mut simulation = ScriptedSimulation::new(
        capabilities(),
        [ScriptedReply::ReadClock(Ok(clock.clone()))],
    );

    simulation.stop().expect("stop succeeds");
    assert_eq!(simulation.status(), SimulationStatus::Stopped);
    assert_eq!(
        simulation.read_clock(&key).expect("stopped read succeeds"),
        clock
    );
    assert!(matches!(
        simulation.insert_clock(ClockState {
            key: key.clone(),
            value: json!(2),
        }),
        Err(SimulationError::InvalidState {
            status: SimulationStatus::Stopped,
            ..
        })
    ));
    assert_eq!(
        simulation.calls(),
        [
            RecordedSimulationCall::Stop,
            RecordedSimulationCall::ReadClock(key.clone()),
            RecordedSimulationCall::InsertClock(ClockState {
                key,
                value: json!(2),
            }),
        ]
    );
}

#[test]
fn script_mismatch_returns_stable_backend_failure_and_faults() {
    let request = ExecuteRequest {
        entry: id("armillae.test/execute"),
        input: json!({}),
    };
    let mut simulation = ScriptedSimulation::new(capabilities(), []);
    let error = simulation
        .execute(request.clone())
        .expect_err("exhausted script fails");
    assert!(matches!(
        error,
        SimulationError::BackendFailure { ref code, .. }
            if code == "armillae.simulate/mock_script_mismatch"
    ));
    assert_eq!(simulation.status(), SimulationStatus::Faulted);
    assert!(matches!(
        simulation.execute(request),
        Err(SimulationError::Faulted { .. })
    ));
}

#[test]
fn scripted_fatal_reply_faults_but_domain_rejection_does_not() {
    let request = ExecuteRequest {
        entry: id("armillae.test/execute"),
        input: json!({}),
    };
    let mut recoverable = ScriptedSimulation::new(
        capabilities(),
        [ScriptedReply::Execute(Err(
            SimulationError::UnknownExecuteEntry {
                entry: request.entry.clone(),
            },
        ))],
    );
    assert!(recoverable.execute(request.clone()).is_err());
    assert_eq!(recoverable.status(), SimulationStatus::Active);

    let mut fatal = ScriptedSimulation::new(
        capabilities(),
        [ScriptedReply::Execute(Err(
            SimulationError::MissingExecuteOutput {
                entry: request.entry.clone(),
            },
        ))],
    );
    assert!(fatal.execute(request).is_err());
    assert_eq!(fatal.status(), SimulationStatus::Faulted);
}

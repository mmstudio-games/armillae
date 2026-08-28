mod support;

use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
};

use armillae_simulate::{ClockInstanceId, TypedAdvanceRequest, TypedAdvanceTarget};
use armillae_simulate_bevy::BevySimulation;
use armillae_tools::ToolContext;
use support::{CounterClock, CounterStep, DemoModule};

#[derive(Clone)]
struct SimulationHandle(Arc<Mutex<BevySimulation>>);

fn apply_step_from_tool_context(
    context: ToolContext,
    instance: ClockInstanceId,
) -> Result<(), Box<dyn Error>> {
    let handle = context
        .get::<SimulationHandle>()
        .ok_or_else(|| io::Error::other("simulation handle is missing"))?;
    let mut simulation = handle
        .0
        .lock()
        .map_err(|_| io::Error::other("simulation handle is poisoned"))?;
    simulation.advance_typed::<CounterClock>(TypedAdvanceRequest {
        targets: vec![TypedAdvanceTarget {
            instance,
            step: CounterStep { delta: 1 },
        }],
    })?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut simulation = support::activate(DemoModule::clock_only())?;
    let instance = ClockInstanceId::new("tool-owned")?;
    simulation.insert_clock_typed(instance.clone(), CounterClock { value: 0 })?;

    let handle = SimulationHandle(Arc::new(Mutex::new(simulation)));
    let context = ToolContext::new().with_extension(handle.clone());
    apply_step_from_tool_context(context, instance.clone())?;

    let simulation = handle
        .0
        .lock()
        .map_err(|_| io::Error::other("simulation handle is poisoned"))?;
    let clock = simulation.read_clock_typed::<CounterClock>(&instance)?;
    println!("clock value after tool-owned write: {}", clock.value);
    Ok(())
}

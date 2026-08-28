mod support;

use std::error::Error;

use armillae_simulate::{
    ClockInstanceId, ExecuteRequest, Simulation, TypedAdvanceRequest, TypedAdvanceTarget,
};
use serde_json::json;
use support::{
    ActionTotal, AdvanceBatches, CounterClock, CounterStep, DemoModule, action_entry_id,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut simulation = support::activate(DemoModule::mixed())?;
    simulation.write_world(|world| {
        world.insert_resource(ActionTotal::default());
        world.insert_resource(AdvanceBatches::default());
    })?;
    let clock = ClockInstanceId::new("main")?;
    simulation.insert_clock_typed(clock.clone(), CounterClock { value: 10 })?;

    simulation.execute(ExecuteRequest {
        entry: action_entry_id(),
        input: json!({ "delta": 4 }),
    })?;
    let unchanged = simulation.read_clock_typed::<CounterClock>(&clock)?;

    simulation.advance_typed::<CounterClock>(TypedAdvanceRequest {
        targets: vec![TypedAdvanceTarget {
            instance: clock.clone(),
            step: CounterStep { delta: 2 },
        }],
    })?;
    let advanced = simulation.read_clock_typed::<CounterClock>(&clock)?;
    let batches = simulation.inspect_world(|world| world.resource::<AdvanceBatches>().0)?;

    println!(
        "after action={}, after advance={}, observed batches={}",
        unchanged.value, advanced.value, batches
    );
    Ok(())
}

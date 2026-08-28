mod support;

use std::error::Error;

use armillae_simulate::{ClockInstanceId, TypedAdvanceRequest, TypedAdvanceTarget};
use support::{CounterClock, CounterStep, DemoModule};

fn main() -> Result<(), Box<dyn Error>> {
    let mut simulation = support::activate(DemoModule::clock_only())?;
    let instance = ClockInstanceId::new("main")?;
    simulation.insert_clock_typed(instance.clone(), CounterClock { value: 0 })?;

    simulation.advance_typed::<CounterClock>(TypedAdvanceRequest {
        targets: vec![TypedAdvanceTarget {
            instance: instance.clone(),
            step: CounterStep { delta: 5 },
        }],
    })?;

    let clock = simulation.read_clock_typed::<CounterClock>(&instance)?;
    println!("single clock value: {}", clock.value);
    Ok(())
}

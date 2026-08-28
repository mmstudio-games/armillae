mod support;

use std::error::Error;

use armillae_simulate::{ClockInstanceId, TypedAdvanceRequest, TypedAdvanceTarget};
use support::{CounterClock, CounterStep, DemoModule};

fn main() -> Result<(), Box<dyn Error>> {
    let mut simulation = support::activate(DemoModule::clock_only())?;
    let primary = ClockInstanceId::new("primary")?;
    let secondary = ClockInstanceId::new("secondary")?;
    simulation.insert_clock_typed(primary.clone(), CounterClock { value: 0 })?;
    simulation.insert_clock_typed(secondary.clone(), CounterClock { value: 100 })?;

    simulation.advance_typed::<CounterClock>(TypedAdvanceRequest {
        targets: vec![
            TypedAdvanceTarget {
                instance: primary.clone(),
                step: CounterStep { delta: 2 },
            },
            TypedAdvanceTarget {
                instance: secondary.clone(),
                step: CounterStep { delta: 3 },
            },
        ],
    })?;

    let primary = simulation.read_clock_typed::<CounterClock>(&primary)?;
    let secondary = simulation.read_clock_typed::<CounterClock>(&secondary)?;
    println!("primary={}, secondary={}", primary.value, secondary.value);
    Ok(())
}

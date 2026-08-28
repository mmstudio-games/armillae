mod support;

use std::error::Error;

use armillae_simulate::{ExecuteRequest, Simulation};
use serde_json::json;
use support::{ActionTotal, DemoModule, action_entry_id};

fn main() -> Result<(), Box<dyn Error>> {
    let mut simulation = support::activate(DemoModule::action_only())?;
    simulation.write_world(|world| world.insert_resource(ActionTotal::default()))?;

    let outcome = simulation.execute(ExecuteRequest {
        entry: action_entry_id(),
        input: json!({ "delta": 3 }),
    })?;

    println!("action output: {:?}", outcome.output);
    Ok(())
}

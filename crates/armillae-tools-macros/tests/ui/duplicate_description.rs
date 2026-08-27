use armillae_tools_macros::tool;

#[tool(description = "first", description = "second")]
fn duplicate_description() -> Result<(), std::convert::Infallible> {
    Ok(())
}

fn main() {}

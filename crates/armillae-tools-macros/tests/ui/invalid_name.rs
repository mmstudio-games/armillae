use armillae_tools_macros::tool;

#[tool(name = "not valid")]
fn invalid_name() -> Result<(), std::convert::Infallible> {
    Ok(())
}

fn main() {}

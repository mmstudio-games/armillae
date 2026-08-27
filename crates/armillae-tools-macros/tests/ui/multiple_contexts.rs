use armillae_tools_macros::tool;

#[tool]
fn multiple_contexts(
    #[tool(context)] first: armillae_tools::ToolContext,
    #[tool(context)] second: armillae_tools::ToolContext,
) -> Result<(), std::convert::Infallible> {
    let _ = (first, second);
    Ok(())
}

fn main() {}

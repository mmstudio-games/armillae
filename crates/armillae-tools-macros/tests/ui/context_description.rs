use armillae_tools_macros::tool;

#[tool]
fn context_description(
    #[tool(context, description = "Host context")] context: armillae_tools::ToolContext,
) -> Result<(), std::convert::Infallible> {
    let _ = context;
    Ok(())
}

fn main() {}

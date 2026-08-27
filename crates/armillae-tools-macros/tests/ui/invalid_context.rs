use armillae_tools_macros::tool;

#[tool]
fn invalid_context(
    #[tool(context)] context: String,
) -> Result<String, std::convert::Infallible> {
    Ok(context)
}

fn main() {}

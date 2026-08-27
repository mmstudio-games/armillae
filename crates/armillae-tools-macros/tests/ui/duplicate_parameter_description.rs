use armillae_tools_macros::tool;

#[tool(params(value = "Function-level description"))]
fn duplicate_parameter_description(
    #[tool(description = "Parameter-level description")] value: i64,
) -> Result<i64, std::convert::Infallible> {
    Ok(value)
}

fn main() {}

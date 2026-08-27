use armillae_tools_macros::tool;

#[tool(params(missing = "Not a parameter"))]
fn named(value: i64) -> Result<i64, std::convert::Infallible> {
    Ok(value)
}

fn main() {}

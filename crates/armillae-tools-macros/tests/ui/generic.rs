use armillae_tools_macros::tool;

#[tool]
fn generic<T>(value: T) -> Result<T, std::convert::Infallible> {
    Ok(value)
}

fn main() {}

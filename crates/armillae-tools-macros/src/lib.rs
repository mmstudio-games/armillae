//! Function-like Tool authoring macros for Armillae.

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::{ItemFn, parse_macro_input};

mod args;
mod expand;

/// Converts a free function into an [`armillae_tools::Tool`] implementation.
///
/// The generated unit struct uses the function name converted to PascalCase.
/// Function and parameter doc comments provide descriptions by default. The
/// function-level `name`, `description`, and `params(...)` arguments or a
/// parameter-level `#[tool(description = "...")]` can override them.
///
/// ```ignore
/// use armillae_tools_macros::tool;
///
/// /// Add two integers.
/// #[tool]
/// async fn add(left: i64, right: i64) -> Result<i64, AddError> {
///     Ok(left + right)
/// }
/// ```
#[proc_macro_attribute]
pub fn tool(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as args::MacroArgs);
    let input = parse_macro_input!(input as ItemFn);

    expand::expand(args, input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

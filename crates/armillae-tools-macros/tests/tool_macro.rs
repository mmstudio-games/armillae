use std::fmt;

use armillae_tools::{
    DynTool, Tool, ToolContext, ToolExecutionError, ToolExecutor, ToolOutput, ToolRegistry,
};
use armillae_tools_macros::tool;
use futures_executor::block_on;
use serde_json::json;

#[derive(Debug)]
struct ArithmeticError;

impl fmt::Display for ArithmeticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("arithmetic failed")
    }
}

impl std::error::Error for ArithmeticError {}

/// Add two integers asynchronously.
#[tool(params(left = "Left operand override."))]
async fn add(
    left: i64,
    #[tool(description = "Right operand override.")] right: i64,
    label: Option<String>,
) -> Result<serde_json::Value, ArithmeticError> {
    Ok(json!({ "sum": left + right, "label": label }))
}

#[derive(Clone)]
struct Offset(i64);

#[tool(
    name = "sum-with-offset",
    description = "Add values with host context."
)]
fn add_with_context(
    left: i64,
    #[tool(context)] context: ToolContext,
    right: i64,
) -> Result<i64, ArithmeticError> {
    let offset = context.get::<Offset>().ok_or(ArithmeticError)?.0;
    Ok(left + right + offset)
}

#[tool(description = "Return a typed error.")]
fn fail() -> Result<(), ArithmeticError> {
    Err(ArithmeticError)
}

#[tool]
pub fn public_echo(value: i64) -> Result<i64, std::convert::Infallible> {
    Ok(value)
}

#[test]
fn async_function_generates_definition_schema_and_execution() {
    let tool: &dyn DynTool = &Add;
    let definition = tool.definition();

    assert_eq!(definition.name, "add");
    assert_eq!(definition.description, "Add two integers asynchronously.");
    assert_eq!(
        definition.input_schema["properties"]["left"]["description"],
        "Left operand override."
    );
    assert_eq!(
        definition.input_schema["properties"]["right"]["description"],
        "Right operand override."
    );
    assert_eq!(
        definition.input_schema["required"],
        json!(["left", "right"])
    );

    let output = block_on(tool.call_json(ToolContext::default(), json!({ "left": 2, "right": 3 })))
        .expect("valid macro-generated arguments must execute");
    assert_eq!(output, ToolOutput::json(json!({ "sum": 5, "label": null })));
}

#[test]
fn context_is_host_only_and_registry_executes_generated_tool() {
    let definition = (&AddWithContext as &dyn DynTool).definition();
    assert_eq!(definition.name, "sum-with-offset");
    assert_eq!(definition.description, "Add values with host context.");
    assert!(
        definition.input_schema["properties"]
            .get("context")
            .is_none()
    );

    let mut registry = ToolRegistry::new();
    registry
        .register(AddWithContext)
        .expect("generated tool has a unique name");
    let result = block_on(
        registry.execute(
            ToolContext::new().with_extension(Offset(4)),
            armillae_core::ToolCall {
                id: armillae_core::ToolCallId::new("macro-call")
                    .expect("fixture ToolCall ID is non-empty"),
                name: "sum-with-offset".to_owned(),
                arguments: json!({ "left": 2, "right": 3 }),
            },
        ),
    )
    .expect("macro-generated tool must execute through the registry");

    assert_eq!(result.call_id.as_str(), "macro-call");
    assert_eq!(result.content.len(), 1);
}

#[test]
fn direct_call_preserves_the_declared_error_type() {
    let typed = block_on(Fail.call(ToolContext::default(), __FailArguments {}));
    let typed_error: ArithmeticError = typed.expect_err("fixture tool must fail");
    assert_eq!(typed_error.to_string(), "arithmetic failed");

    let result = block_on((&Fail as &dyn DynTool).call_json(ToolContext::default(), json!({})));
    let error = result.expect_err("fixture tool must fail");
    assert_eq!(
        error,
        ToolExecutionError::ExecutionFailed {
            name: "fail".to_owned(),
            message: "arithmetic failed".to_owned(),
        }
    );
}

#[test]
fn generated_tool_preserves_function_visibility() {
    let typed =
        block_on(PublicEcho.call(ToolContext::default(), __PublicEchoArguments { value: 7 }))
            .expect("public generated argument fields must be constructible");
    assert_eq!(typed, 7);

    let output = block_on(
        (&PublicEcho as &dyn DynTool).call_json(ToolContext::default(), json!({ "value": 7 })),
    )
    .expect("public macro-generated tool must execute");
    assert_eq!(output, ToolOutput::json(json!(7)));
}

#[test]
fn invalid_macro_uses_are_compile_errors() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}

use std::{
    borrow::Cow,
    convert::Infallible,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use armillae_core::{ToolCall, ToolResultContent};
use armillae_tools::{
    DynTool, IntoToolOutput, Tool, ToolContext, ToolExecutionError, ToolExecutor, ToolOutput,
    ToolRegistry, ToolRegistryError,
};
use futures_executor::block_on;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, Serializer, ser::Error as _};
use serde_json::json;

#[derive(Debug, Deserialize, JsonSchema)]
struct AddArgs {
    /// First operand.
    left: i64,
    /// Second operand.
    right: i64,
}

#[derive(Debug, PartialEq, Serialize)]
struct Sum {
    value: i64,
}

struct Add;

impl Tool for Add {
    type Args = AddArgs;
    type Output = Sum;
    type Error = Infallible;

    const NAME: &'static str = "add";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Add two integers")
    }

    async fn call(
        &self,
        _context: ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(Sum {
            value: args.left + args.right,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MessageArgs {
    message: String,
}

struct MultiContent;

impl Tool for MultiContent {
    type Args = MessageArgs;
    type Output = ToolOutput;
    type Error = Infallible;

    const NAME: &'static str = "multi_content";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Return explicit ordered content")
    }

    async fn call(
        &self,
        _context: ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(ToolOutput::new(vec![
            ToolResultContent::Text { text: args.message },
            ToolResultContent::Json {
                value: json!({ "complete": true }),
            },
        ]))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EmptyArgs {}

#[derive(Debug)]
struct ExpectedFailure;

impl fmt::Display for ExpectedFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected tool failure")
    }
}

impl std::error::Error for ExpectedFailure {}

struct FailingTool {
    calls: Arc<AtomicUsize>,
}

impl Tool for FailingTool {
    type Args = EmptyArgs;
    type Output = Sum;
    type Error = ExpectedFailure;

    const NAME: &'static str = "failing";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Always fail once")
    }

    async fn call(
        &self,
        _context: ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ExpectedFailure)
    }
}

struct UnserializableOutput;

impl Serialize for UnserializableOutput {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(S::Error::custom("expected serialization failure"))
    }
}

struct SerializationFailure;

impl Tool for SerializationFailure {
    type Args = EmptyArgs;
    type Output = UnserializableOutput;
    type Error = Infallible;

    const NAME: &'static str = "serialization_failure";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Return an unserializable value")
    }

    async fn call(
        &self,
        _context: ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(UnserializableOutput)
    }
}

#[derive(Clone)]
struct Offset(i64);

struct AddWithContext;

impl Tool for AddWithContext {
    type Args = AddArgs;
    type Output = Sum;
    type Error = ExpectedFailure;

    const NAME: &'static str = "add_with_context";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Add two integers and a host-provided offset")
    }

    async fn call(
        &self,
        context: ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let offset = context.get::<Offset>().ok_or(ExpectedFailure)?;
        Ok(Sum {
            value: args.left + args.right + offset.0,
        })
    }
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments,
    }
}

fn registry_with<T>(tool: T) -> ToolRegistry
where
    T: DynTool + 'static,
{
    let mut registry = ToolRegistry::new();
    registry
        .register(tool)
        .expect("a new registry must accept one uniquely named tool");
    registry
}

#[test]
fn typed_tool_definition_and_json_output_are_automatic() {
    let tool: Arc<dyn DynTool> = Arc::new(Add);
    let definition = tool.definition();

    assert_eq!(definition.name, "add");
    assert_eq!(definition.description, "Add two integers");
    assert_eq!(definition.input_schema["type"], "object");
    assert_eq!(
        definition.input_schema["properties"]["left"]["type"],
        "integer"
    );
    assert_eq!(
        definition.input_schema["properties"]["right"]["type"],
        "integer"
    );
    assert_eq!(
        definition.input_schema["required"],
        json!(["left", "right"])
    );

    let output = block_on(tool.call_json(ToolContext::default(), json!({ "left": 2, "right": 3 })))
        .expect("valid typed arguments must execute");
    assert_eq!(
        output.content,
        [ToolResultContent::Json {
            value: json!({ "value": 5 }),
        }]
    );
}

#[test]
fn ordinary_strings_remain_structured_json_by_default() {
    let output = "plain string"
        .to_owned()
        .into_tool_output()
        .expect("a String must serialize as JSON");
    assert_eq!(
        output.content,
        [ToolResultContent::Json {
            value: json!("plain string"),
        }]
    );
    assert_eq!(
        ToolOutput::text("plain string").content,
        [ToolResultContent::Text {
            text: "plain string".to_owned(),
        }]
    );
}

#[test]
fn explicit_tool_output_preserves_ordered_multi_content() {
    let registry = registry_with(MultiContent);
    let result = block_on(registry.execute(
        ToolContext::default(),
        call("call-multi", "multi_content", json!({ "message": "done" })),
    ))
    .expect("explicit ToolOutput must execute");

    assert_eq!(result.call_id, "call-multi");
    assert!(!result.is_error);
    assert_eq!(
        result.content,
        [
            ToolResultContent::Text {
                text: "done".to_owned(),
            },
            ToolResultContent::Json {
                value: json!({ "complete": true }),
            },
        ]
    );
}

#[test]
fn invalid_arguments_are_classified_without_calling_the_tool() {
    let tool: Arc<dyn DynTool> = Arc::new(Add);

    for arguments in [
        json!({}),
        json!({ "left": "two", "right": 3 }),
        json!("not an argument object"),
    ] {
        let error = block_on(tool.call_json(ToolContext::default(), arguments))
            .expect_err("invalid arguments must fail before Tool::call");
        assert!(matches!(
            error,
            ToolExecutionError::InvalidArguments { ref name, .. } if name == "add"
        ));
    }
}

#[test]
fn unknown_execution_and_serialization_failures_remain_distinct() {
    let empty_registry = ToolRegistry::new();
    let unknown = block_on(empty_registry.execute(
        ToolContext::default(),
        call("call-missing", "missing", json!({})),
    ))
    .expect_err("an unknown Tool must return a host execution error");
    assert_eq!(
        unknown,
        ToolExecutionError::UnknownTool {
            name: "missing".to_owned(),
        }
    );

    let calls = Arc::new(AtomicUsize::new(0));
    let failing_registry = registry_with(FailingTool {
        calls: Arc::clone(&calls),
    });
    let execution = block_on(failing_registry.execute(
        ToolContext::default(),
        call("call-failing", "failing", json!({})),
    ))
    .expect_err("a Tool error must remain a host execution error");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        execution,
        ToolExecutionError::ExecutionFailed {
            name: "failing".to_owned(),
            message: "expected tool failure".to_owned(),
        }
    );

    let serialization_registry = registry_with(SerializationFailure);
    let serialization = block_on(serialization_registry.execute(
        ToolContext::default(),
        call("call-serialization", "serialization_failure", json!({})),
    ))
    .expect_err("output serialization must be fallible");
    assert!(matches!(
        serialization,
        ToolExecutionError::OutputSerialization { ref message }
            if message.contains("expected serialization failure")
    ));
}

#[test]
fn registry_rejects_duplicates_sorts_definitions_and_unregisters() {
    let duplicate = match ToolRegistry::builder().register(Add) {
        Ok(builder) => builder.register(Add),
        Err(error) => panic!("the first unique Tool must register: {error}"),
    }
    .err()
    .expect("the second Tool with the same name must be rejected");
    assert_eq!(
        duplicate,
        ToolRegistryError::DuplicateTool {
            name: "add".to_owned(),
        }
    );

    let mut registry = ToolRegistry::new();
    registry
        .register(MultiContent)
        .expect("multi-content Tool name is unique");
    registry.register(Add).expect("add Tool name is unique");
    registry
        .register(AddWithContext)
        .expect("context Tool name is unique");

    assert_eq!(
        registry
            .definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>(),
        ["add", "add_with_context", "multi_content"]
    );
    assert_eq!(registry.len(), 3);
    assert!(registry.contains("add"));
    assert!(registry.get("add").is_some());
    assert!(registry.unregister("add").is_some());
    assert!(!registry.contains("add"));
    assert_eq!(registry.len(), 2);
    assert!(registry.unregister("add").is_none());
}

#[test]
fn context_extensions_are_type_safe_cloneable_and_host_only() {
    let mut context = ToolContext::new().with_extension(Offset(4));
    context.insert::<String>("host-only-secret".to_owned());
    assert!(context.contains::<Offset>());
    assert_eq!(
        context.get::<String>().map(String::as_str),
        Some("host-only-secret")
    );

    let cloned = context.clone();
    let offset = cloned
        .get_arc::<Offset>()
        .expect("cloned Context must share extension values safely");
    assert_eq!(offset.0, 4);

    let registry = registry_with(AddWithContext);
    let result = block_on(registry.execute(
        cloned,
        call(
            "stable-call-id",
            "add_with_context",
            json!({ "left": 2, "right": 3 }),
        ),
    ))
    .expect("Tool must receive its host Context");
    assert_eq!(result.call_id, "stable-call-id");
    assert_eq!(
        result.content,
        [ToolResultContent::Json {
            value: json!({ "value": 9 }),
        }]
    );

    let removed = context
        .remove::<String>()
        .expect("existing extension must be removable");
    assert_eq!(removed.as_str(), "host-only-secret");
    assert!(!context.contains::<String>());
}

#[test]
fn debug_output_does_not_expose_tool_or_context_content() {
    let output = ToolOutput::new(vec![
        ToolResultContent::Text {
            text: "secret-text-output".to_owned(),
        },
        ToolResultContent::Json {
            value: json!({ "credential": "secret-json-output" }),
        },
    ]);
    let output_debug = format!("{output:?}");
    assert!(output_debug.contains("content_count: 2"));
    assert!(output_debug.contains("text"));
    assert!(output_debug.contains("json"));
    assert!(!output_debug.contains("secret-text-output"));
    assert!(!output_debug.contains("secret-json-output"));

    let context = ToolContext::new().with_extension("secret-context-value".to_owned());
    let context_debug = format!("{context:?}");
    assert!(context_debug.contains("extension_count: 1"));
    assert!(!context_debug.contains("secret-context-value"));
}

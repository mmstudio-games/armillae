use std::{borrow::Cow, convert::Infallible};

use armillae_core::{AssistantContent, CompletionRequest, Message, ToolChoice};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;
use armillae_tools::{Tool, ToolContext, ToolExecutor, ToolRegistry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
struct LookupArgs {
    query: String,
}

#[derive(Serialize)]
struct LookupOutput {
    answer: String,
}

struct Lookup;

impl Tool for Lookup {
    type Args = LookupArgs;
    type Output = LookupOutput;
    type Error = Infallible;

    const NAME: &'static str = "lookup";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Look up a deterministic example value")
    }

    async fn call(
        &self,
        _context: ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(LookupOutput {
            answer: format!("local result for {}", args.query),
        })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("rig", "openai", "gpt-4.1-mini")
        .credential(CredentialRef::Environment {
            name: "OPENAI_API_KEY".to_owned(),
        })
        .build()?;
    let resolved = config.resolve().await?;
    let bridge = RigBridgeFactory.create(resolved).await?;
    let tools = ToolRegistry::builder().register(Lookup)?.build();

    let user_message = Message::user("Use lookup for the value armillae, then report its result.");
    let first = bridge
        .complete(CompletionRequest {
            messages: vec![user_message.clone()],
            tools: tools.definitions(),
            tool_choice: Some(ToolChoice::Specific {
                name: Lookup::NAME.to_owned(),
            }),
            ..CompletionRequest::default()
        })
        .await?;

    let calls = first.tool_calls().cloned().collect::<Vec<_>>();
    if calls.is_empty() {
        return Err("the model returned no ToolCall".into());
    }
    let mut history = vec![user_message, first.as_assistant_message()];
    for call in calls {
        let result = tools.execute(ToolContext::default(), call).await?;
        history.push(Message::tool_result(result));
    }

    let final_response = bridge
        .complete(CompletionRequest {
            messages: history,
            tools: tools.definitions(),
            tool_choice: Some(ToolChoice::None),
            ..CompletionRequest::default()
        })
        .await?;
    for content in final_response.content {
        if let AssistantContent::Text(text) = content {
            println!("{}", text.text);
        }
    }

    Ok(())
}

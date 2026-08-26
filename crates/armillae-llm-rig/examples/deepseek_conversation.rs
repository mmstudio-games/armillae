use std::{
    borrow::Cow,
    convert::Infallible,
    io::{self, Write},
};

use armillae_core::{
    AssistantContent, CompletionRequest, ContentPart, GenerationOptions, Message, Role, ToolChoice,
};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;
use armillae_tools::{Tool, ToolContext, ToolExecutor, ToolRegistry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
struct LookupProjectFactArgs {
    topic: String,
}

#[derive(Serialize)]
struct LookupProjectFactOutput {
    fact: String,
}

struct LookupProjectFact;

impl Tool for LookupProjectFact {
    type Args = LookupProjectFactArgs;
    type Output = LookupProjectFactOutput;
    type Error = Infallible;

    const NAME: &'static str = "lookup_project_fact";

    fn description(&self) -> Cow<'static, str> {
        Cow::Borrowed("Look up a deterministic local fact about a project topic")
    }

    async fn call(
        &self,
        _context: ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let fact = if args.topic.eq_ignore_ascii_case("armillae") {
            "Armillae exposes provider-independent LLM and Tool protocols.".to_owned()
        } else {
            format!("No local project fact is registered for {}.", args.topic)
        };
        Ok(LookupProjectFactOutput { fact })
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("deepseek", "deepseek-v4-flash")
        .credential(CredentialRef::Environment {
            name: "DEEPSEEK_API_KEY".to_owned(),
        })
        .defaults(GenerationOptions {
            temperature: Some(0.7),
            max_output_tokens: Some(512),
            ..GenerationOptions::default()
        })
        .build()?;
    let resolved = config.resolve().await?;
    let bridge = RigBridgeFactory.create(resolved).await?;
    let tools = ToolRegistry::builder().register(LookupProjectFact)?.build();
    let definitions = tools.definitions();

    let mut history = vec![Message::new(
        Role::System,
        vec![ContentPart::text(
            "You are a concise assistant. Answer in the same language as the user. Use the local \
             lookup tool when the user asks for a project fact.",
        )],
    )];
    let mut input = String::new();

    println!("DeepSeek conversation with local Tool dispatch. Type /quit to exit.");
    println!("Try: Use the local lookup tool to tell me about Armillae.");
    loop {
        print!("you> ");
        io::stdout().flush()?;

        input.clear();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        let prompt = input.trim();
        if prompt == "/quit" {
            break;
        }
        if prompt.is_empty() {
            continue;
        }

        history.push(Message::user(prompt));
        let first = bridge
            .complete(CompletionRequest {
                messages: history.clone(),
                tools: definitions.clone(),
                tool_choice: Some(ToolChoice::Auto),
                ..CompletionRequest::default()
            })
            .await?;
        let calls = first.tool_calls().cloned().collect::<Vec<_>>();

        let response = if calls.is_empty() {
            first
        } else {
            history.push(first.as_assistant_message());
            for call in calls {
                println!("tool> executing {}", call.name);
                let result = tools.execute(ToolContext::default(), call).await?;
                history.push(Message::tool_result(result));
            }

            bridge
                .complete(CompletionRequest {
                    messages: history.clone(),
                    tools: definitions.clone(),
                    tool_choice: Some(ToolChoice::None),
                    ..CompletionRequest::default()
                })
                .await?
        };

        print!("assistant> ");
        let mut has_text = false;
        for content in &response.content {
            if let AssistantContent::Text(text) = content {
                print!("{}", text.text);
                has_text = true;
            }
        }
        if !has_text {
            print!("[no text returned]");
        }
        println!();
        history.push(response.as_assistant_message());
    }

    Ok(())
}

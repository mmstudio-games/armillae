use std::io::{self, Write};

use armillae_core::{
    AssistantContent, CompletionRequest, ContentPart, GenerationOptions, Message, Role,
};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("rig", "deepseek", "deepseek-v4-flash")
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

    let mut history = vec![Message::new(
        Role::System,
        vec![ContentPart::text(
            "You are a concise assistant. Answer in the same language as the user.",
        )],
    )];
    let mut input = String::new();

    println!("DeepSeek conversation. Type /quit to exit.");
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
        let response = bridge
            .complete(CompletionRequest {
                messages: history.clone(),
                ..CompletionRequest::default()
            })
            .await?;

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

use armillae_core::{AssistantContent, CompletionRequest, Message};
use armillae_llm::{BridgeConfig, BridgeFactory};
use armillae_llm_rig::RigBridgeFactory;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("rig", "ollama", "qwen3:8b").build()?;
    let resolved = config.resolve().await?;
    let bridge = RigBridgeFactory.create(resolved).await?;

    let response = bridge
        .complete(CompletionRequest {
            messages: vec![Message::user(
                "Explain what a provider-independent bridge is in one sentence.",
            )],
            ..CompletionRequest::default()
        })
        .await?;

    for content in response.content {
        if let AssistantContent::Text(text) = content {
            println!("{}", text.text);
        }
    }
    Ok(())
}

use armillae_core::{AssistantContent, CompletionRequest, Message};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("rig", "openai", "gpt-4.1-mini")
        .credential(CredentialRef::Environment {
            name: "OPENAI_API_KEY".to_owned(),
        })
        .build()?;
    let resolved = config.resolve(None, None).await?;
    let bridge = RigBridgeFactory.create(resolved).await?;

    let response = bridge
        .complete(CompletionRequest {
            messages: vec![Message::user(
                "Explain what one model call means in one sentence.",
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

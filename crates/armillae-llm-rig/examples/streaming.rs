use std::io::{self, Write};

use armillae_core::{CompletionEvent, CompletionRequest, Message};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef, LlmBridge};
use armillae_llm_rig::RigBridgeFactory;
use futures::StreamExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = BridgeConfig::builder("rig", "openai", "gpt-4.1-mini")
        .credential(CredentialRef::Environment {
            name: "OPENAI_API_KEY".to_owned(),
        })
        .build()?;
    let resolved = config.resolve(None, None).await?;
    let bridge = RigBridgeFactory.create(resolved).await?;

    stream_to_stdout(
        bridge.as_ref(),
        CompletionRequest {
            messages: vec![Message::user("Write one short sentence about streaming.")],
            ..CompletionRequest::default()
        },
    )
    .await?;
    Ok(())
}

async fn stream_to_stdout(
    bridge: &dyn LlmBridge,
    request: CompletionRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = bridge.stream(request).await?;
    while let Some(event) = stream.next().await {
        match event? {
            CompletionEvent::TextDelta { text, .. } => {
                print!("{text}");
                io::stdout().flush()?;
            }
            CompletionEvent::ResponseCompleted { response } => {
                println!();
                if let Some(usage) = response.usage {
                    eprintln!("token usage: {usage:?}");
                }
            }
            _ => {}
        }
    }
    Ok(())
}

use std::io;

use armillae_core::{AssistantContent, CompletionRequest, Message, OutputFormat};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReleaseSummary {
    title: String,
    highlights: Vec<String>,
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

    let response = bridge
        .complete(CompletionRequest {
            messages: vec![Message::user(
                "Summarize Armillae as a short release note with exactly two highlights.",
            )],
            output_format: Some(OutputFormat::JsonSchema {
                name: "release_summary".to_owned(),
                schema: serde_json::to_value(schema_for!(ReleaseSummary))?,
                strict: true,
            }),
            ..CompletionRequest::default()
        })
        .await?;

    let json = response
        .content
        .iter()
        .find_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .ok_or_else(|| io::Error::other("the model returned no JSON text"))?;
    let summary: ReleaseSummary = serde_json::from_str(json)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

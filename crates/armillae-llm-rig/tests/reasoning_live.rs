//! Opt-in real Provider reasoning gates. Never infer credentials or model settings.
//! ARMILLAE_LIVE_<PROFILE>_MODEL, optional _ENDPOINT and required _GENERATION (JSON)
//! select the exact model and controls. <PROFILE>_API_KEY supplies credentials.
use armillae_core::{CompletionRequest, GenerationOptions, Message};
use armillae_llm::{
    BridgeConfig, BridgeFactory, CredentialRef, mock::contract::validate_stream_events,
};
use armillae_llm_rig::RigBridgeFactory;
use futures::StreamExt;
use std::{env, error::Error, io};

async fn run(provider: &str, profile: &str) -> Result<(), Box<dyn Error>> {
    let model = env::var(format!("ARMILLAE_LIVE_{profile}_MODEL"))?;
    let generation: GenerationOptions =
        serde_json::from_str(&env::var(format!("ARMILLAE_LIVE_{profile}_GENERATION"))?)?;
    if generation.thinking.is_none() && generation.reasoning_effort.is_none() {
        return Err(io::Error::other("explicit thinking or reasoning_effort is required").into());
    }
    let mut builder = BridgeConfig::builder(provider, model).defaults(generation);
    if provider != "ollama" {
        builder = builder.credential(CredentialRef::Environment {
            name: format!("{profile}_API_KEY"),
        });
    }
    if let Ok(endpoint) = env::var(format!("ARMILLAE_LIVE_{profile}_ENDPOINT")) {
        builder = builder.endpoint(endpoint.parse()?);
    }
    let bridge = RigBridgeFactory
        .create(builder.build()?.resolve().await?)
        .await?;
    let initial = Message::user("What is 17 times 23? Give a short answer.");
    let request = CompletionRequest {
        messages: vec![initial.clone()],
        ..Default::default()
    };
    bridge.project(&request)?;
    let response = bridge.complete(request.clone()).await?;
    let events = bridge
        .stream(request)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    validate_stream_events(&events)?;
    let followup = CompletionRequest {
        messages: vec![
            initial,
            response.as_assistant_message(),
            Message::user("Add one to that result."),
        ],
        ..Default::default()
    };
    bridge.project(&followup)?;
    bridge.complete(followup).await?;
    Ok(())
}

macro_rules! gate {
    ($name:ident, $provider:literal, $profile:literal) => {
        #[tokio::test]
        #[ignore = "requires explicit model, controls, credentials and host authorization"]
        async fn $name() -> Result<(), Box<dyn Error>> {
            run($provider, $profile).await
        }
    };
}
gate!(openai, "openai", "OPENAI");
gate!(openai_compatible, "openai-compatible", "OPENAI_COMPATIBLE");
gate!(deepseek, "deepseek", "DEEPSEEK");
gate!(anthropic, "anthropic", "ANTHROPIC");
gate!(moonshot, "moonshot", "MOONSHOT");
gate!(minimax, "minimax", "MINIMAX");
gate!(ollama, "ollama", "OLLAMA");

//! Live end-to-end verification of the section-paradigm compression chain
//! against a real provider (default ignored; requires credentials).
//!
//! Env contract (same convention as `openai_live.rs`):
//! - `DEEPSEEK_API_KEY` — provider credential (relay keys work the same way)
//! - `ARMILLAE_LIVE_PROVIDER` — optional provider override (default `deepseek`)
//! - `ARMILLAE_LIVE_MODEL` — optional model override (default `deepseek-v4-flash`)
//! - `ARMILLAE_LIVE_ENDPOINT` — optional base URL override (required for relays)

use std::{env, error::Error, io, sync::Arc, time::Duration};

use armillae_context::{
    ActiveWindow, AutoCompression, CompressionTarget, Context, InMemorySectionStore, SectionConfig,
    SectionContext,
};
use armillae_core::{
    AssistantContent, CompletionRequest, CompletionResponse, ContentPart, Message, Role,
    TokenUsage, ToolCall, ToolCallId,
};
use armillae_llm::{BridgeConfig, BridgeError, BridgeFactory, CredentialRef, LlmBridge};
use armillae_llm_rig::RigBridgeFactory;
use serde_json::json;

struct LiveTarget {
    provider: String,
    model: String,
    credential_env: String,
    endpoint: Option<String>,
}

impl LiveTarget {
    fn from_environment() -> Result<Self, Box<dyn Error>> {
        let provider = env::var("ARMILLAE_LIVE_PROVIDER").unwrap_or_else(|_| "deepseek".to_owned());
        let (default_model, credential_env) = match provider.as_str() {
            "deepseek" => ("deepseek-v4-flash", "DEEPSEEK_API_KEY"),
            "openai" => ("gpt-4.1-mini", "OPENAI_API_KEY"),
            _ => {
                return Err(
                    io::Error::other("ARMILLAE_LIVE_PROVIDER must be deepseek or openai").into(),
                );
            }
        };
        Ok(Self {
            provider,
            model: env::var("ARMILLAE_LIVE_MODEL").unwrap_or_else(|_| default_model.to_owned()),
            credential_env: credential_env.to_owned(),
            endpoint: env::var("ARMILLAE_LIVE_ENDPOINT").ok(),
        })
    }

    async fn bridge(&self) -> Result<Arc<dyn LlmBridge>, Box<dyn Error>> {
        let mut builder = BridgeConfig::builder(&self.provider, &self.model).credential(
            CredentialRef::Environment {
                name: self.credential_env.clone(),
            },
        );
        if let Some(endpoint) = &self.endpoint {
            builder = builder.endpoint(endpoint.parse()?);
        }
        let resolved = builder.build()?.resolve().await?;
        Ok(RigBridgeFactory.create(resolved).await?)
    }
}

fn usage(input: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        total_tokens: Some(input + 1),
        cached_input_tokens: Some(0),
    }
}

/// Simulate the model calling `record_section` at the end of each answer.
fn record_section(rounds: u64, label: &str, id: &str) -> Message {
    Message::assistant(vec![
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new(id).expect("live call ids are non-empty"),
            name: "context.record_section".to_owned(),
            arguments: json!({ "section_start_rounds": rounds, "label": label }),
        }),
        ContentPart::text(format!("（第 {rounds} 小节边界已记录：{label}）")),
    ])
}

fn response_text(response: &CompletionResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Retry transient provider rejections/transport failures (relays are flaky);
/// retry strategy belongs to the downstream per the bridge spec.
async fn complete_with_retry(
    bridge: &Arc<dyn LlmBridge>,
    request: CompletionRequest,
) -> Result<CompletionResponse, BridgeError> {
    let mut attempt = 0u32;
    loop {
        match bridge.complete(request.clone()).await {
            Ok(response) => return Ok(response),
            Err(BridgeError::ProviderRejected { .. } | BridgeError::Transport { .. })
                if attempt < 3 =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(800 * u64::from(attempt))).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[tokio::test]
#[ignore = "live provider: requires DEEPSEEK_API_KEY and network access"]
async fn section_compression_chain_runs_against_live_provider() -> Result<(), Box<dyn Error>> {
    let live = LiveTarget::from_environment()?;
    let bridge = live.bridge().await?;

    let mut context = SectionContext::builder(
        SectionConfig {
            active_window: ActiveWindow::Sections { count: 2 },
            auto_compression: Some(AutoCompression::TokenThreshold { threshold: 60 }),
            ..SectionConfig::default()
        },
        Arc::new(InMemorySectionStore::new()),
    )
    .build()?;
    context.restore_session("live-session")?;

    // 对话：三轮，每轮结束后模拟调用 record_section 划界
    for (index, (topic, tokens)) in [("基础规则", 30u64), ("角色设定", 50), ("当前行动", 70)]
        .iter()
        .enumerate()
    {
        context.push_user_input(Message::user(format!("第 {} 轮：{topic}", index + 1)))?;
        context.apply_model_output(
            record_section(1, "dialog", &format!("live-rs-{index}")),
            usage(*tokens),
        )?;
    }

    // 真实推理 1：export 出的上下文必须能被真实 Provider 直接消费（convert.rs 契约 + 零组装）
    let exported = context.export()?;
    let answer = complete_with_retry(
        &bridge,
        CompletionRequest {
            messages: exported,
            ..CompletionRequest::default()
        },
    )
    .await?;
    assert!(
        !response_text(&answer).trim().is_empty(),
        "live provider must answer the exported context"
    );

    // 自动压缩触发 → 真实推理 2：prepare 产物（指令 + 目标内容）直接推理出摘要
    let target = context
        .evaluate_compression()?
        .expect("TokenThreshold must trigger a sealed candidate");
    let section_id = match target {
        CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    let messages = context.prepare_compression(target)?;
    let summary_response = complete_with_retry(
        &bridge,
        CompletionRequest {
            messages,
            ..CompletionRequest::default()
        },
    )
    .await?;
    let summary_text = response_text(&summary_response);
    assert!(
        !summary_text.trim().is_empty(),
        "live provider must produce a compression summary"
    );
    let summary: Vec<Message> = summary_response
        .content
        .into_iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => {
                Some(Message::assistant(vec![ContentPart::text(text.text)]))
            }
            _ => None,
        })
        .collect();

    // 提交压缩并导出：摘要视图 + 剩余小节原文，record_section 痕迹剥离
    context.apply_compression_result(summary)?;
    let final_export = context.export()?;
    let mappings = context.section_mappings();
    let compressed = mappings
        .iter()
        .find(|mapping| mapping.id == section_id)
        .expect("compressed section exists");
    assert_eq!(compressed.view, armillae_context::View::Compressed);
    assert!(final_export.iter().any(|message| {
        message.role == Role::User
            && message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text(text) if !text.text.trim().is_empty()))
    }));
    assert!(
        !final_export
            .iter()
            .any(|message| message.content.iter().any(|part| {
                matches!(part, ContentPart::ToolCall(call) if call.name == "context.record_section")
            })),
        "record_section traces must be stripped from the exported context"
    );

    println!(
        "live chain ok: provider={} model={} summary={:?}",
        live.provider, live.model, summary_text
    );
    Ok(())
}

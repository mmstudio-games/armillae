//! 完整上下文链路示例：对话 → `record_section` 划分 → 自动压缩 → `export`
//! → Bridge 推理（离线 Mock，无凭证可跑）。
//!
//! 演示"下游零组装"压缩管道：`prepare_compression` 的产物（压缩指令消息 +
//! 目标内容）直接放入 `CompletionRequest.messages`，压缩指令全部由范式内部
//! 组装；下游只负责把 Bridge 返回的摘要交回 `apply_compression_result`。

use std::sync::Arc;

use armillae_context::{
    ActiveWindow, AutoCompression, CompressionTarget, Context, InMemorySectionStore, SectionConfig,
    SectionContext,
};
use armillae_core::{
    AssistantContent, CompletionRequest, ContentPart, Message, Role, TokenUsage, ToolCall,
    ToolCallId,
};
use armillae_llm::{
    LlmBridge,
    mock::{MockBridge, MockResponse},
};
use serde_json::json;

fn usage(input: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        total_tokens: Some(input + 1),
        cached_input_tokens: Some(0),
    }
}

/// 模拟模型每轮回答结束后调用 `record_section` 划界。
fn record_section(rounds: u64, label: &str, id: &str) -> Message {
    Message::assistant(vec![
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new(id).expect("示例 call id 非空"),
            name: "context.record_section".to_owned(),
            arguments: json!({
                "section_start_rounds": rounds,
                "label": label,
            }),
        }),
        ContentPart::text("（回答结束，已记录小节边界）"),
    ])
}

fn render(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_role(message: &Message) -> &'static str {
    match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::Developer => "developer",
        _ => "unknown",
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 构造小节范式：活跃窗口 2 + TokenThreshold 自动压缩
    let store = Arc::new(InMemorySectionStore::new());
    let mut context = SectionContext::builder(
        SectionConfig {
            active_window: ActiveWindow::Sections { count: 2 },
            auto_compression: Some(AutoCompression::TokenThreshold { threshold: 60 }),
            ..SectionConfig::default()
        },
        store,
    )
    .build()?;
    context.restore_session("example-session")?;

    // 2. 对话：三轮，模型每轮调用 record_section；token 事实逐步增长
    let rounds = [("基础规则", 30u64), ("角色设定", 50), ("当前行动", 70)];
    for (index, (topic, tokens)) in rounds.iter().enumerate() {
        context.push_user_input(Message::user(format!("第 {} 轮：{topic}", index + 1)))?;
        context.apply_model_output(
            record_section(1, "dialog", &format!("rs-{index}")),
            usage(*tokens),
        )?;
    }
    println!("== 划分后的小节 ==");
    for mapping in context.section_mappings() {
        println!(
            "  section {}: {:?} ({} 轮)",
            mapping.id, mapping.label, mapping.turn_count
        );
    }

    // 3. 自动压缩触发：token 事实 70 >= 阈值 60 → 命中固化区候选（最旧小节）
    let target = context
        .evaluate_compression()?
        .expect("TokenThreshold 必须触发自动压缩");
    let section_id = match target {
        CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    println!("\n== 自动压缩触发：目标小节 {section_id} ==");

    // 4. 准备压缩上下文（指令消息 + 目标内容，下游零组装）
    let messages = context.prepare_compression(target)?;
    println!("\n== prepare 产物（直接可推理，零组装）==");
    for message in &messages {
        println!("  [{}] {}", render_role(message), render(message));
    }

    // 5. Bridge 推理：MockBridge 固定返回压缩摘要（离线可跑）
    let bridge = MockBridge::fixed(MockResponse::text("（压缩摘要）基础规则已归档为一段概要。"));
    let response = bridge
        .complete(CompletionRequest {
            messages,
            ..CompletionRequest::default()
        })
        .await?;
    let summary: Vec<Message> = response
        .content
        .into_iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => {
                Some(Message::assistant(vec![ContentPart::text(text.text)]))
            }
            _ => None,
        })
        .collect();

    // 6. 提交压缩结果并导出可推理上下文（剥离 record_section 痕迹）
    context.apply_compression_result(summary)?;
    let exported = context.export()?;
    println!("\n== 压缩提交后的 export（可推理上下文）==");
    for message in &exported {
        println!("  [{}] {}", render_role(message), render(message));
    }
    Ok(())
}

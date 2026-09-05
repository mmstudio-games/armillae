//! Section-paradigm contract tests (spec §7.1, §8, §9, §12).

#![cfg(feature = "testing")]

use std::sync::Arc;

use armillae_context::{
    ActiveWindow, AutoCompression, CompressionMethod, CompressionState, CompressionTarget, Context,
    ContextError, InMemorySectionStore, LabelPolicy, Section, SectionConfig, SectionContext,
    SectionLabel, SectionState, SectionStore, StandardLabel, TokenFacts, ToolTurnPolicy, View,
    WindowState, testing::MockContext,
};
use armillae_core::{
    ContentPart, Message, Role, TokenUsage, ToolCall, ToolCallId, ToolResult, ToolResultContent,
};
use serde_json::{Value, json};

fn user(text: &str) -> Message {
    Message::user(text)
}

fn assistant(text: &str) -> Message {
    Message::assistant(vec![ContentPart::text(text)])
}

fn usage(input: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        total_tokens: Some(input + 1),
        cached_input_tokens: Some(0),
    }
}

fn call_id(value: &str) -> ToolCallId {
    ToolCallId::new(value).expect("fixture call ids are non-empty")
}

fn record_section_call(rounds: u64, label: Option<&str>, id: &str) -> Message {
    let mut arguments = json!({ "section_start_rounds": rounds });
    if let Some(label) = label {
        arguments["label"] = json!(label);
    }
    Message::assistant(vec![
        ContentPart::ToolCall(ToolCall {
            id: call_id(id),
            name: "context.record_section".to_owned(),
            arguments,
        }),
        ContentPart::text("done"),
    ])
}

fn tool_result(id: &str, text: &str) -> Message {
    Message::tool_result(ToolResult {
        call_id: call_id(id),
        content: vec![ToolResultContent::Text {
            text: text.to_owned(),
        }],
        is_error: false,
    })
}

fn default_config() -> SectionConfig {
    SectionConfig {
        active_window: ActiveWindow::Sections { count: 4 },
        ..SectionConfig::default()
    }
}

fn build(config: SectionConfig, store: Arc<InMemorySectionStore>) -> SectionContext {
    SectionContext::builder(config, store)
        .build()
        .expect("valid config must build")
}

fn fresh(config: SectionConfig) -> (SectionContext, Arc<InMemorySectionStore>) {
    let store = Arc::new(InMemorySectionStore::new());
    let mut context = build(config, store.clone());
    context
        .restore_session("test-session")
        .expect("restore creates a fresh session");
    (context, store)
}

fn four_rounds(context: &mut SectionContext) {
    for index in 0..4 {
        context
            .push_user_input(user(&format!("q{index}")))
            .expect("push");
        context
            .apply_model_output(assistant(&format!("a{index}")), usage(10))
            .expect("apply");
    }
}

// —— 构建与配置 ——

#[test]
fn builder_validates_custom_labels_and_freezes_tool_schema() {
    let store = Arc::new(InMemorySectionStore::new());
    let context = build(SectionConfig::default(), store.clone());
    let definition = context.record_section_definition();
    assert_eq!(definition.name, "context.record_section");
    let label_enum = definition.input_schema["properties"]["label"]["enum"]
        .as_array()
        .expect("label enum");
    assert!(label_enum.contains(&json!("decision")));
    assert!(label_enum.contains(&json!("uncategorized")));

    let with_custom = SectionContext::builder(SectionConfig::default(), store.clone())
        .with_custom_label(
            "custom.docs".to_owned(),
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: CompressionMethod::Deep,
            },
        )
        .build()
        .expect("namespaced custom label must build");
    let label_enum =
        with_custom.record_section_definition().input_schema["properties"]["label"]["enum"]
            .as_array()
            .expect("label enum");
    assert!(label_enum.contains(&json!("custom.docs")));

    let invalid = SectionContext::builder(SectionConfig::default(), store)
        .with_custom_label(
            "not-namespaced".to_owned(),
            LabelPolicy {
                compressible: true,
                priority: 0,
                method: CompressionMethod::Deep,
            },
        )
        .build();
    assert!(matches!(
        invalid,
        Err(ContextError::InvalidConfiguration { .. })
    ));
}

#[test]
fn default_config_matches_spec_standard_policies() {
    let config = SectionConfig::default();
    let policies = &config.label_policies;
    assert!(!policies[&StandardLabel::Plan].compressible);
    assert!(!policies[&StandardLabel::Constraint].compressible);
    assert!(!policies[&StandardLabel::Preference].compressible);
    let decision = policies[&StandardLabel::Decision];
    assert!(decision.compressible);
    assert_eq!(decision.priority, 1);
    assert_eq!(decision.method, CompressionMethod::Shallow);
    assert!(policies[&StandardLabel::Fact].compressible);
    assert_eq!(
        policies[&StandardLabel::Fact].method,
        CompressionMethod::Deep
    );
    assert_eq!(config.active_window, ActiveWindow::Sections { count: 4 });
    assert_eq!(config.tool_turn_policy, ToolTurnPolicy::Downgrade);
    assert_eq!(config.compressed_message_role, Role::User);
}

// —— 三层级模型与对话写入 ——

#[test]
fn dialogue_accumulates_into_one_section_and_exports_raw() {
    let (mut context, _store) = fresh(default_config());
    four_rounds(&mut context);
    let mappings = context.section_mappings();
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].turn_count, 4);
    assert_eq!(mappings[0].view, View::Raw);
    let exported = context.export().expect("export");
    assert_eq!(exported.len(), 8);
}

#[test]
fn dialogue_respects_cache_zone_write_target() {
    let config = SectionConfig {
        cache_prefix_sections: 1,
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    let mappings = context.section_mappings();
    // 第一个小节属于缓存区且冻结；后续写入进入动态小节
    assert!(mappings.len() >= 2);
    assert!(
        context
            .relabel(mappings[0].id, SectionLabel::Standard(StandardLabel::Fact))
            .is_err()
    );
}

// —— record_section 划分 ——

#[test]
fn record_section_carves_boundary_and_applies_label() {
    let (mut context, _store) = fresh(default_config());
    four_rounds(&mut context);
    context
        .apply_model_output(
            record_section_call(1, Some("decision"), "call-1"),
            usage(10),
        )
        .expect("record_section");
    let mappings = context.section_mappings();
    assert_eq!(mappings.len(), 2);
    let newest = mappings.last().expect("newest section");
    assert_eq!(newest.turn_count, 1);
    assert_eq!(
        newest.label,
        SectionLabel::Standard(StandardLabel::Decision)
    );
}

#[test]
fn record_section_is_idempotent_on_exact_boundary() {
    let (mut context, _store) = fresh(default_config());
    four_rounds(&mut context);
    context
        .apply_model_output(
            record_section_call(1, Some("decision"), "call-1"),
            usage(10),
        )
        .expect("record_section");
    context
        .apply_model_output(record_section_call(1, None, "call-2"), usage(10))
        .expect("repeat record_section");
    let mappings = context.section_mappings();
    assert_eq!(mappings.len(), 2, "exact boundary is idempotent");
}

#[test]
fn record_section_clamps_and_merges_across_open_sections() {
    let (mut context, _store) = fresh(default_config());
    four_rounds(&mut context);
    // 裁出最后 2 轮 → s1（新）；s0 剩 2 轮
    context
        .apply_model_output(record_section_call(2, Some("task"), "call-1"), usage(10))
        .expect("record_section");
    assert_eq!(context.section_mappings().len(), 2);
    // rounds=0（非正整数 → 1）+ rounds=99（> T → 全部）：先 1 轮
    context
        .apply_model_output(record_section_call(0, None, "call-2"), usage(10))
        .expect("rounds clamp to 1");
    let mappings = context.section_mappings();
    assert_eq!(mappings.last().expect("newest").turn_count, 1);
    // 再 99 轮 → 覆盖全部 4 轮 → 跨两个 Open 小节 → 合并为新小节
    context
        .apply_model_output(record_section_call(99, None, "call-3"), usage(10))
        .expect("rounds clamp to total");
    let mappings = context.section_mappings();
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].turn_count, 4);
}

#[test]
fn record_section_only_merges_open_portion() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    // s0（2 轮）+ 裁出 s1（2 轮，最新活跃）；s0 在 Sections(1) 下已固化
    context
        .apply_model_output(record_section_call(2, None, "call-1"), usage(10))
        .expect("record_section");
    assert_eq!(context.section_mappings().len(), 2);
    // rounds=3 → 需 3 轮，但固化区不参与 → 只收集到 s1 的 2 轮（铺满 → 幂等）
    context
        .apply_model_output(record_section_call(3, None, "call-2"), usage(10))
        .expect("record_section");
    assert_eq!(
        context.section_mappings().len(),
        2,
        "sealed turns never merge"
    );
}

// —— 窗口滑动与自动压缩 ——

#[test]
fn all_window_disables_auto_compression() {
    let config = SectionConfig {
        active_window: ActiveWindow::All,
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    assert!(context.evaluate_compression().expect("evaluate").is_none());
}

#[test]
fn token_threshold_targets_best_sealed_candidate() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(
            record_section_call(1, Some("decision"), "call-1"),
            usage(100),
        )
        .expect("record_section");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("token threshold must trigger");
    assert!(matches!(
        target,
        armillae_context::CompressionTarget::Section { .. }
    ));
}

#[test]
fn hyper_compresses_just_carved_section_unconditionally() {
    let config = SectionConfig {
        active_window: ActiveWindow::Hyper,
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    // 未划分 → 不触发
    assert!(context.evaluate_compression().expect("evaluate").is_none());
    // 划分出刚结束小节 → 无条件触发压缩该小节
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(10))
        .expect("record_section");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("hyper must trigger");
    let section_id = match target {
        armillae_context::CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    assert_eq!(
        section_id,
        context.section_mappings().last().expect("newest").id
    );
}

// —— 压缩管道与持久化编排 ——

#[test]
fn compression_pipeline_persists_original_then_swaps_view() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");

    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    let messages = context
        .prepare_compression(target)
        .expect("prepare must persist original first");
    assert_eq!(
        store.original_count("test-session"),
        1,
        "prepare must save the original first"
    );
    assert!(messages.first().expect("instruction").role == Role::System);
    assert!(messages.len() >= 2, "instruction message + target content");

    context
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");
    let mappings = context.section_mappings();
    assert!(
        mappings
            .iter()
            .any(|mapping| mapping.view == View::Compressed)
    );
    assert_eq!(store.compressed_count("test-session"), 1);
    assert!(store.has_state("test-session"));

    let exported = context.export().expect("export");
    assert!(exported.iter().any(|message| {
        message.role == Role::User
            && message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text(text) if text.text == "summary"))
    }));
}

#[test]
fn abandon_cleans_up_pending_original() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    context.prepare_compression(target).expect("prepare");
    assert_eq!(store.original_count("test-session"), 1);
    context.abandon_compression().expect("abandon");
    assert_eq!(
        store.original_count("test-session"),
        0,
        "abandon must delete the archived original"
    );
    assert!(context.export().expect("export").len() >= 4);
}

#[test]
fn section_pipeline_rejects_prepare_without_evaluate() {
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    assert!(matches!(
        context.prepare_compression(armillae_context::CompressionTarget::Section { id: 0 }),
        Err(ContextError::InvalidState { .. })
    ));
}

#[test]
fn prepare_output_strips_record_section_traces() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    // s1 划出（含 record_section 调用）；再一轮划出 s2 使 s1 固化
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section 1");
    context.push_user_input(user("q5")).expect("push");
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-2"), usage(100))
        .expect("record_section 2");
    // s0 改为不可压 → 目标应为含 record_section 调用的 s1
    let s0_id = context.section_mappings()[0].id;
    context
        .relabel(s0_id, SectionLabel::Standard(StandardLabel::Plan))
        .expect("relabel s0 to non-compressible");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    let section_id = match target {
        armillae_context::CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    assert_ne!(
        section_id, s0_id,
        "the compressible sealed section must be the target"
    );
    let messages = context.prepare_compression(target).expect("prepare");
    assert!(
        messages
            .iter()
            .all(|message| !message.content.iter().any(|part| {
                matches!(part, ContentPart::ToolCall(call) if call.name == "context.record_section")
            })),
        "prepare output must not carry unpaired record_section tool calls"
    );
}

// —— 恢复与降级 ——

#[test]
fn restore_session_round_trips_state_and_compressed_view() {
    let store = Arc::new(InMemorySectionStore::new());
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let mut first = build(config.clone(), store.clone());
    first.restore_session("session-a").expect("restore");
    four_rounds(&mut first);
    first
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    let target = first
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    first.prepare_compression(target).expect("prepare");
    first
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");

    let mut second = build(config, store);
    second.restore_session("session-a").expect("restore");
    let exported = second.export().expect("export after restore");
    assert!(exported.iter().any(|message| {
        message.role == Role::User
            && message
                .content
                .iter()
                .any(|part| matches!(part, ContentPart::Text(text) if text.text == "summary"))
    }));
    assert_eq!(
        first.section_mappings(),
        second.section_mappings(),
        "mappings survive the restore"
    );
}

#[test]
fn restore_degrades_to_original_when_snapshot_missing() {
    let store = Arc::new(InMemorySectionStore::new());
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let mut first = build(config.clone(), store.clone());
    first.restore_session("session-a").expect("restore");
    four_rounds(&mut first);
    first
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    let target = first
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    first.prepare_compression(target).expect("prepare");
    first
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");
    store.clear_compressed("session-a");

    let mut second = build(config, store);
    second.restore_session("session-a").expect("restore");
    assert!(
        second
            .section_mappings()
            .iter()
            .any(|mapping| mapping.view == View::Raw),
        "missing snapshot degrades to the original view"
    );
}

#[test]
fn recompress_and_decompress_round_trip() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    context.prepare_compression(target).expect("prepare");
    context
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");
    let compressed_id = context
        .section_mappings()
        .iter()
        .find(|mapping| mapping.view == View::Compressed)
        .expect("compressed section")
        .id;

    context
        .decompress(compressed_id)
        .expect("decompress restores the original");
    let mappings = context.section_mappings();
    let decompressed = mappings
        .iter()
        .find(|mapping| mapping.id == compressed_id)
        .expect("section");
    assert_eq!(decompressed.view, View::Raw);
    assert!(context.export().expect("export").len() >= 4);

    context
        .recompress(compressed_id)
        .expect("recompress restores the snapshot view");
    let mappings = context.section_mappings();
    let recompressed = mappings
        .iter()
        .find(|mapping| mapping.id == compressed_id)
        .expect("section");
    assert_eq!(recompressed.view, View::Compressed);
}

// —— 导出与剥离 ——

#[test]
fn export_strips_record_section_traces_but_keeps_real_tools() {
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: call_id("call-real"),
                name: "lookup".to_owned(),
                arguments: json!({}),
            })]),
            usage(3),
        )
        .expect("real tool call");
    context
        .apply_model_output(tool_result("call-real", "42"), usage(4))
        .expect("real tool result");
    context
        .apply_model_output(record_section_call(1, None, "call-rs"), usage(5))
        .expect("record_section");
    context
        .apply_model_output(tool_result("call-rs", "ok"), usage(6))
        .expect("record_section result");

    let exported = context.export().expect("export");
    assert!(exported.iter().all(|message| message.role != Role::Tool
        || message
            .content
            .iter()
            .any(|part| matches!(part, ContentPart::ToolResult(result) if result.call_id.as_str() == "call-real"))));
    assert!(
        !exported
            .iter()
            .any(|message| message.content.iter().any(|part| {
                matches!(part, ContentPart::ToolCall(call) if call.name == "context.record_section")
            }))
    );
    assert!(!exported.iter().any(|message| message.role == Role::Tool
        && message.content.iter().any(|part| matches!(
            part,
            ContentPart::ToolResult(result) if result.call_id.as_str() == "call-rs"
        ))));
}

#[test]
fn export_rejects_convert_contract_violations() {
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::new(
                Role::User,
                vec![ContentPart::ToolCall(ToolCall {
                    id: call_id("call-user"),
                    name: "bad".to_owned(),
                    arguments: json!({}),
                })],
            ),
            usage(3),
        )
        .expect("user message with a tool call");
    assert!(matches!(
        context.export(),
        Err(ContextError::InvalidRequest { .. })
    ));
}

// —— 范式切换无感（薄契约） ——

fn drive_dialogue(context: &mut dyn Context) -> Result<Vec<Message>, ContextError> {
    context.push_user_input(user("hello"))?;
    context.apply_model_output(assistant("hi"), usage(3))?;
    context.push_user_input(user("again"))?;
    context.apply_model_output(assistant("bye"), usage(4))?;
    context.export()
}

fn drive_compression(context: &mut dyn Context) -> Result<(), ContextError> {
    if let Some(target) = context.evaluate_compression()? {
        let messages = context.prepare_compression(target)?;
        assert!(
            !messages.is_empty(),
            "prepared messages must be directly inferable"
        );
        context.apply_compression_result(vec![assistant("summary")])?;
    }
    context.abandon_compression()?;
    Ok(())
}

#[test]
fn paradigm_switch_dialogue_driver_is_contract_stable() {
    let mut mock = MockContext::new(false);
    let mock_export = drive_dialogue(&mut mock).expect("mock dialogue driver");
    assert_eq!(mock_export.len(), 4);

    let (mut section, _store) = fresh(default_config());
    let section_export = drive_dialogue(&mut section).expect("section dialogue driver");
    assert_eq!(section_export.len(), 4);
    assert_eq!(mock_export, section_export);
}

#[test]
fn paradigm_switch_compression_driver_is_contract_stable() {
    let mut mock = MockContext::new(true);
    drive_dialogue(&mut mock).expect("seed mock dialogue");
    drive_compression(&mut mock).expect("mock compression driver");

    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut section, _store) = fresh(config);
    four_rounds(&mut section);
    section
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    drive_compression(&mut section).expect("section compression driver");
}

// —— 范式类型 Serde round-trip ——

#[test]
fn paradigm_types_serde_round_trip() {
    let config = SectionConfig {
        cache_prefix_sections: 2,
        active_window: ActiveWindow::Hyper,
        auto_compression: Some(AutoCompression::SectionSwitch),
        compression_token_target: Some(128),
        ..SectionConfig::default()
    };
    let encoded = serde_json::to_value(&config).expect("config serializes");
    let decoded: SectionConfig = serde_json::from_value(encoded).expect("config deserializes");
    assert_eq!(decoded, config);

    let label = SectionLabel::Custom("custom.docs".to_owned());
    let encoded = serde_json::to_value(&label).expect("label serializes");
    let decoded: SectionLabel = serde_json::from_value(encoded).expect("label deserializes");
    assert_eq!(decoded, label);

    let section = Section {
        id: 3,
        label: SectionLabel::Standard(StandardLabel::Dialog),
        view: View::Raw,
        turns: vec![armillae_context::Turn {
            messages: vec![user("hello"), assistant("hi")],
        }],
        version: 1,
        original_ref: None,
        compressed_ref: None,
        summary: None,
    };
    let encoded = serde_json::to_value(&section).expect("section serializes");
    let decoded: Section = serde_json::from_value(encoded).expect("section deserializes");
    assert_eq!(decoded, section);

    let window: Value =
        serde_json::to_value(ActiveWindow::Sections { count: 3 }).expect("window serializes");
    assert_eq!(window, json!({ "type": "sections", "count": 3 }));
    assert_eq!(
        serde_json::to_value(ActiveWindow::All).expect("window serializes"),
        json!({ "type": "all" })
    );
    assert_eq!(
        serde_json::to_value(ActiveWindow::Hyper).expect("window serializes"),
        json!({ "type": "hyper" })
    );
}

// —— 审计驱动补测（实现 vs 文档全量审计发现） ——

#[test]
fn apply_compression_increments_version_and_links_refs() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    let section_id = match target {
        CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    let before = context
        .section_mappings()
        .iter()
        .find(|mapping| mapping.id == section_id)
        .expect("section")
        .version;
    context.prepare_compression(target).expect("prepare");
    context
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");

    let state = store
        .session_state("test-session")
        .expect("state persisted");
    let section = state
        .sections
        .iter()
        .find(|section| section.id == section_id)
        .expect("section in state");
    assert_eq!(
        section.version,
        before + 1,
        "version must increment on apply"
    );
    let original_ref = section.original_ref.as_ref().expect("original ref kept");
    let compressed_ref = section
        .compressed_ref
        .as_ref()
        .expect("compressed ref kept");
    let entry = store
        .load_compressed("test-session", compressed_ref)
        .expect("load")
        .expect("compressed entry exists");
    assert_eq!(
        &entry.original_ref, original_ref,
        "compressed entry must link the prepare-time original ref"
    );
    let original = store
        .load_original("test-session", original_ref)
        .expect("load")
        .expect("original entry exists");
    assert!(
        !original.messages.is_empty(),
        "original entry keeps the raw content"
    );
}

#[test]
fn restore_rejects_unsupported_schema_version() {
    let store = Arc::new(InMemorySectionStore::new());
    store
        .save_state(&SectionState {
            schema_version: 999,
            session_id: "bad".to_owned(),
            sections: Vec::new(),
            window: WindowState {
                mode: ActiveWindow::Sections { count: 1 },
                cache_prefix_sections: 0,
                sealed_count: 0,
                active_count: 0,
            },
            machine: CompressionState::Idle,
            token_facts: TokenFacts::default(),
        })
        .expect("save");
    let mut context = build(default_config(), store);
    assert!(matches!(
        context.restore_session("bad"),
        Err(ContextError::InvalidConfiguration { .. })
    ));
}

#[test]
fn export_removes_empty_stripped_messages() {
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    // assistant 消息只含 record_section ToolCall（无文本）→ 剥离后整条移除
    context
        .apply_model_output(
            Message::assistant(vec![ContentPart::ToolCall(ToolCall {
                id: call_id("rs-only"),
                name: "context.record_section".to_owned(),
                arguments: json!({ "section_start_rounds": 1 }),
            })]),
            usage(3),
        )
        .expect("apply");
    let exported = context.export().expect("export");
    assert_eq!(
        exported.len(),
        1,
        "stripped-empty assistant message must be removed"
    );
    assert_eq!(exported[0].role, Role::User);
}

#[test]
fn export_rejects_each_convert_contract_violation() {
    // System 含 ToolCall
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::new(
                Role::System,
                vec![ContentPart::ToolCall(ToolCall {
                    id: call_id("sys"),
                    name: "x".to_owned(),
                    arguments: json!({}),
                })],
            ),
            usage(3),
        )
        .expect("apply");
    assert!(
        matches!(context.export(), Err(ContextError::InvalidRequest { .. })),
        "system messages must be text only"
    );

    // Tool 含非 ToolResult 文本
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::new(Role::Tool, vec![ContentPart::text("not a result")]),
            usage(3),
        )
        .expect("apply");
    assert!(
        matches!(context.export(), Err(ContextError::InvalidRequest { .. })),
        "tool messages must contain tool results only"
    );

    // Assistant 含 ToolResult
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::new(
                Role::Assistant,
                vec![ContentPart::ToolResult(ToolResult {
                    call_id: call_id("ar"),
                    content: vec![ToolResultContent::Text {
                        text: "x".to_owned(),
                    }],
                    is_error: false,
                })],
            ),
            usage(3),
        )
        .expect("apply");
    assert!(
        matches!(context.export(), Err(ContextError::InvalidRequest { .. })),
        "assistant messages must not contain tool results"
    );

    // ProviderData 一律拒绝
    let (mut context, _store) = fresh(default_config());
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(
            Message::new(
                Role::User,
                vec![ContentPart::ProviderData(armillae_core::ProviderData {
                    provider: "p".to_owned(),
                    kind: "k".to_owned(),
                    value: json!({}),
                })],
            ),
            usage(3),
        )
        .expect("apply");
    assert!(
        matches!(context.export(), Err(ContextError::InvalidRequest { .. })),
        "provider data must be rejected from the export"
    );
}

#[test]
fn cache_zone_rejects_merge_split_and_never_compresses() {
    // merge/split 涉及缓存区 → InvalidOperation
    let config = SectionConfig {
        cache_prefix_sections: 1,
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    context.push_user_input(user("q1")).expect("push");
    context.push_user_input(user("q2")).expect("push");
    let mappings = context.section_mappings();
    assert!(
        mappings.len() >= 2,
        "second write must create a dynamic section"
    );
    let cache_id = mappings[0].id;
    let dynamic_id = mappings[1].id;
    assert!(matches!(
        context.merge_sections(vec![cache_id, dynamic_id], None),
        Err(ContextError::InvalidOperation { .. })
    ));
    assert!(matches!(
        context.split_section(cache_id, 1),
        Err(ContextError::InvalidOperation { .. })
    ));

    // 缓存区永不压缩：全部小节都在缓存区 → 无固化候选
    let config = SectionConfig {
        cache_prefix_sections: 10,
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section");
    assert!(
        context.evaluate_compression().expect("evaluate").is_none(),
        "cache-zone sections are never compression candidates"
    );
}

#[test]
fn token_facts_recalibrate_after_compression() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 60 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(70))
        .expect("record_section 1");
    context.push_user_input(user("q5")).expect("push");
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-2"), usage(70))
        .expect("record_section 2");
    // 压缩一个固化 Raw 小节（s0）
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    context.prepare_compression(target).expect("prepare");
    context
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");
    // 压缩后：新一轮 usage 低于阈值 → 评估关闭（校准向下）
    context.push_user_input(user("q6")).expect("push");
    context
        .apply_model_output(assistant("a6"), usage(30))
        .expect("apply");
    assert!(
        context.evaluate_compression().expect("evaluate").is_none(),
        "below-threshold usage must not trigger after compression"
    );
    // 再划出 s3 固化 s2（Raw 候选）；usage 回到高位 → 恢复触发（校准向上）
    context.push_user_input(user("q7")).expect("push");
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-3"), usage(95))
        .expect("record_section 3");
    assert!(
        context.evaluate_compression().expect("evaluate").is_some(),
        "token facts must recalibrate to the latest usage after compression"
    );
}

#[test]
fn reject_policy_excludes_tool_turn_sections_from_candidates() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 1 }),
        tool_turn_policy: ToolTurnPolicy::Reject,
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section 1");
    context.push_user_input(user("q5")).expect("push");
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-2"), usage(100))
        .expect("record_section 2");
    let s0_id = context.section_mappings()[0].id;
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("trigger");
    let section_id = match target {
        CompressionTarget::Section { id } => id,
        _ => panic!("unexpected compression target"),
    };
    assert_eq!(
        section_id, s0_id,
        "Reject must exclude tool-turn sections (s1) from candidates"
    );
}

#[test]
fn apply_with_missing_input_tokens_is_tolerated_and_keeps_previous_facts() {
    let config = SectionConfig {
        active_window: ActiveWindow::Sections { count: 1 },
        auto_compression: Some(AutoCompression::TokenThreshold { threshold: 60 }),
        ..default_config()
    };
    let (mut context, _store) = fresh(config);
    four_rounds(&mut context);
    context
        .apply_model_output(record_section_call(1, Some("dialog"), "call-1"), usage(100))
        .expect("record_section with usage");
    context
        .apply_model_output(
            assistant("no-usage"),
            TokenUsage {
                input_tokens: None,
                ..TokenUsage::default()
            },
        )
        .expect("missing input tokens must be tolerated");
    assert!(
        context.evaluate_compression().expect("evaluate").is_some(),
        "token facts must keep the previous round when input_tokens is missing"
    );
}

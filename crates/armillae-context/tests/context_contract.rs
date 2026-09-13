//! Shared `Context` contract tests driven through the scripted `MockContext`
//! (spec §6.2, §6.3, §12).

#![cfg(feature = "testing")]

use std::sync::Arc;

use armillae_context::{
    CompressionState, CompressionTarget, Context, ContextError, testing::MockContext,
};
use armillae_core::{Message, TokenUsage};

fn user(text: &str) -> Message {
    Message::user(text)
}

fn assistant(text: &str) -> Message {
    Message::assistant(vec![armillae_core::ContentPart::text(text)])
}

fn usage(input: u64) -> TokenUsage {
    TokenUsage {
        input_tokens: Some(input),
        output_tokens: Some(1),
        total_tokens: Some(input + 1),
        cached_input_tokens: Some(0),
    }
}

#[test]
fn fresh_mock_exports_empty_request_error() {
    let context = MockContext::new(false);
    assert!(matches!(
        context.export(),
        Err(ContextError::InvalidRequest { .. })
    ));
}

#[test]
fn dialogue_writes_and_export_round_trip() {
    let mut context = MockContext::new(false);
    context.push_user_input(user("hello")).expect("push");
    context
        .apply_model_output(assistant("hi"), usage(3))
        .expect("apply");
    let exported = context.export().expect("export");
    assert_eq!(exported.len(), 2);
    assert_eq!(context.last_usage().expect("usage").input_tokens, Some(3));
}

#[test]
fn evaluate_trigger_freezes_writes_until_prepare_and_apply() {
    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");

    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("auto mock always triggers");
    assert_eq!(context.compression_state(), CompressionState::Evaluated);
    assert!(matches!(
        context.apply_model_output(assistant("late"), usage(1)),
        Err(ContextError::InvalidState { .. })
    ));
    assert!(matches!(
        context.push_user_input(user("late")),
        Err(ContextError::InvalidState { .. })
    ));

    let messages = context
        .prepare_compression(target.clone())
        .expect("prepare");
    assert_eq!(context.compression_state(), CompressionState::Prepared);
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        context.prepare_compression(target),
        Err(ContextError::InvalidState { .. })
    ));

    context
        .apply_compression_result(vec![assistant("summary")])
        .expect("apply");
    assert_eq!(context.compression_state(), CompressionState::Idle);
    assert_eq!(context.export().expect("export")[0], assistant("summary"));
}

#[test]
fn evaluate_none_keeps_writes_allowed() {
    let mut context = MockContext::new(false);
    context.push_user_input(user("hello")).expect("push");
    assert!(context.evaluate_compression().expect("evaluate").is_none());
    context
        .push_user_input(user("again"))
        .expect("still writable");
    assert_eq!(context.export().expect("export").len(), 2);
}

#[test]
fn prepare_before_evaluate_is_invalid_state() {
    let mut context = MockContext::new(false);
    context.push_user_input(user("hello")).expect("push");
    assert!(matches!(
        context.prepare_compression(CompressionTarget::Section { id: 0 }),
        Err(ContextError::InvalidState {
            operation: "prepare_compression",
            expected: CompressionState::Evaluated,
            actual: CompressionState::Idle,
        })
    ));
}

#[test]
fn prepare_with_mismatched_target_is_invalid_operation() {
    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");
    context
        .evaluate_compression()
        .expect("evaluate")
        .expect("target");
    assert!(matches!(
        context.prepare_compression(CompressionTarget::Section { id: 999 }),
        Err(ContextError::InvalidOperation { .. })
    ));
}

#[test]
fn apply_without_prepare_is_invalid_state() {
    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");
    context.evaluate_compression().expect("evaluate");
    assert!(matches!(
        context.apply_compression_result(Vec::new()),
        Err(ContextError::InvalidState { .. })
    ));
}

#[test]
fn abandon_is_noop_in_idle_and_resets_frozen_states() {
    let mut context = MockContext::new(false);
    context.push_user_input(user("hello")).expect("push");
    context
        .abandon_compression()
        .expect("idle abandon is a no-op");
    context.push_user_input(user("ok")).expect("still writable");

    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");
    context.evaluate_compression().expect("evaluate");
    context
        .abandon_compression()
        .expect("abandon from Evaluated");
    context
        .push_user_input(user("ok"))
        .expect("writable after abandon");

    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");
    let target = context
        .evaluate_compression()
        .expect("evaluate")
        .expect("target");
    context.prepare_compression(target).expect("prepare");
    context
        .abandon_compression()
        .expect("abandon from Prepared");
    assert_eq!(context.export().expect("export").len(), 1);
}

#[test]
fn export_is_pure_and_allowed_in_frozen_states() {
    let mut context = MockContext::new(true);
    context.push_user_input(user("hello")).expect("push");
    let before = context.export().expect("export");
    context.evaluate_compression().expect("evaluate");
    let during = context.export().expect("export in Evaluated");
    assert_eq!(before, during);
}

#[test]
fn mock_is_send_sync_and_usable_as_arc_dyn_context() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Arc<dyn Context>>();

    let mut context: Arc<dyn Context> = Arc::new(MockContext::new(false));
    Arc::get_mut(&mut context)
        .expect("unique Arc access for serial driving")
        .push_user_input(user("hello"))
        .expect("object-safe call");
    assert_eq!(
        Arc::get_mut(&mut context)
            .expect("unique")
            .export()
            .expect("export")
            .len(),
        1
    );
}

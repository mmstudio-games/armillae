//! Public protocol contract: serde round-trips, wire shapes, and the checked
//! JSON Schema snapshot (spec §4.1).
//!
//! `serde_json::to_value` takes its argument by value; the fixtures below pass
//! explicit references so the non-`Copy` protocol values stay usable. The
//! clippy lint that suggests dropping the borrow does not account for that
//! move, so it is disabled here.

#![allow(clippy::needless_borrows_for_generic_args)]

use armillae_context::{CompressionState, CompressionTarget, PROTOCOL_VERSION};
use schemars::JsonSchema;
use serde_json::{Value, json};

#[test]
fn protocol_version_is_frozen() {
    assert_eq!(PROTOCOL_VERSION, "armillae.context/v1alpha1");
}

#[test]
fn compression_target_round_trips_and_uses_type_tag() {
    let target = CompressionTarget::Section { id: 7 };
    let encoded = serde_json::to_value(&target).expect("target must serialize");
    assert_eq!(encoded, json!({ "type": "section", "id": 7 }));
    let decoded: CompressionTarget =
        serde_json::from_value(encoded).expect("target must deserialize");
    assert_eq!(decoded, target);
    assert!(serde_json::from_value::<CompressionTarget>(json!({ "type": "future" })).is_err());
}

#[test]
fn compression_state_round_trips_as_snake_case() {
    for (state, wire) in [
        (CompressionState::Idle, json!("idle")),
        (CompressionState::Evaluated, json!("evaluated")),
        (CompressionState::Prepared, json!("prepared")),
    ] {
        assert_eq!(
            serde_json::to_value(&state).expect("state must serialize"),
            wire
        );
        let decoded: CompressionState =
            serde_json::from_value(wire).expect("state must deserialize");
        assert_eq!(decoded, state);
    }
    assert!(serde_json::from_value::<CompressionState>(json!("compressed")).is_err());
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ProtocolSchema {
    target: CompressionTarget,
    state: CompressionState,
}

fn assert_root_schema<T: JsonSchema>() {
    let schema = schemars::schema_for!(T).to_value();
    let object = schema.as_object().expect("root schema is an object");
    assert_eq!(
        object.get("$schema"),
        Some(&json!("https://json-schema.org/draft/2020-12/schema"))
    );
    assert_ne!(object.get("additionalProperties"), Some(&json!(false)));
}

#[test]
fn protocol_root_schemas_use_draft_2020_12_and_remain_open() {
    assert_root_schema::<CompressionTarget>();
    assert_root_schema::<CompressionState>();
}

#[test]
fn protocol_schema_is_valid_json_and_matches_snapshot() {
    let schema = schemars::schema_for!(ProtocolSchema);
    let actual = serde_json::to_value(schema).expect("generated schema must be valid JSON");
    let expected: Value = serde_json::from_str(include_str!("snapshots/protocol-schema.json"))
        .expect("checked-in protocol schema snapshot must be valid JSON");
    assert_eq!(actual, expected);
}

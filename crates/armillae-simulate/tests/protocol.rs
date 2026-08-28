use std::collections::BTreeSet;

use armillae_simulate::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

macro_rules! assert_identifier_contract {
    ($type:ty, $kind:expr) => {{
        let value = <$type>::new("armillae.test/value").expect("valid identifier");
        assert_eq!(value.as_str(), "armillae.test/value");
        assert_eq!(value.to_string(), "armillae.test/value");
        assert_eq!(
            serde_json::from_value::<$type>(json!("armillae.test/value"))
                .expect("valid identifier JSON"),
            value
        );

        let empty = <$type>::new("").expect_err("empty identifier must fail");
        assert_eq!(empty.kind, $kind);
        assert_eq!(empty.reason, InvalidIdentifierReason::Empty);
        assert!(serde_json::from_value::<$type>(json!("contains space")).is_err());
        assert!(serde_json::from_value::<$type>(json!("非ascii")).is_err());
        assert!(<$type>::new("x".repeat(256)).is_err());
    }};
}

#[test]
fn identifiers_validate_construction_and_deserialization() {
    assert_identifier_contract!(ModuleId, IdentifierKind::Module);
    assert_identifier_contract!(ExecuteEntryId, IdentifierKind::ExecuteEntry);
    assert_identifier_contract!(ClockTypeId, IdentifierKind::ClockType);
    assert_identifier_contract!(ClockInstanceId, IdentifierKind::ClockInstance);
    assert_identifier_contract!(ClockErrorCode, IdentifierKind::ClockErrorCode);
    assert_identifier_contract!(SystemErrorCode, IdentifierKind::SystemErrorCode);
    assert_identifier_contract!(SystemId, IdentifierKind::System);
    assert_identifier_contract!(BackendId, IdentifierKind::Backend);
    assert_identifier_contract!(CapabilityId, IdentifierKind::Capability);
}

#[test]
fn versions_are_canonical_and_requirements_match_semver() {
    let version = SemanticVersion::new("1.2.3-alpha.1+build.7").expect("valid semver");
    assert_eq!(version.as_str(), "1.2.3-alpha.1+build.7");
    let requirement = VersionRequirement::new(" >=1.0, <2.0 ").expect("valid requirement");
    assert_eq!(requirement.as_str(), ">=1.0, <2.0");
    assert!(requirement.matches(&SemanticVersion::new("1.2.3").expect("valid stable version")));
    assert_eq!(
        serde_json::from_value::<SemanticVersion>(json!(version.as_str()))
            .expect("valid version JSON"),
        version
    );
    assert!(SemanticVersion::new("1").is_err());
    assert!(VersionRequirement::new("not a requirement").is_err());
}

fn id<T>(value: &str) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    value.parse().expect("test identifier is valid")
}

fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        api_version: SIMULATE_API_VERSION.to_owned(),
        id: id("armillae.test/module"),
        version: SemanticVersion::new("1.0.0").expect("valid version"),
        dependencies: vec![ModuleDependency {
            id: id("armillae.test/dependency"),
            version: VersionRequirement::new("^1.0").expect("valid requirement"),
        }],
        execution: ExecutionPlane::Native {
            backend: id("armillae.test/backend"),
            adapter: VersionRequirement::new("^0.1").expect("valid adapter requirement"),
        },
        required_capabilities: BTreeSet::from([id("armillae.test/capability")]),
        execute_entries: vec![ExecuteEntryDefinition {
            id: id("armillae.test/execute"),
            input_schema: json!({ "type": "object" }),
            output_schema: None,
        }],
        clocks: vec![ClockDefinition {
            id: id("armillae.test/clock"),
            value_schema: json!({ "type": "integer" }),
            step_schema: json!({ "type": "integer" }),
        }],
        systems: vec![SystemDefinition {
            id: id("armillae.test/system"),
            trigger: SystemTrigger::Execute {
                entry: id("armillae.test/execute"),
            },
            before: Vec::new(),
            after: Vec::new(),
        }],
    }
}

fn assert_round_trip<T>(value: &T)
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_value(value).expect("protocol serializes");
    let decoded: T = serde_json::from_value(encoded).expect("protocol deserializes");
    assert_eq!(&decoded, value);
}

#[test]
fn protocol_roots_round_trip_and_accept_unknown_object_fields() {
    let module = descriptor();
    assert_round_trip(&module);

    let mut encoded = serde_json::to_value(&module).expect("module serializes");
    encoded
        .as_object_mut()
        .expect("module is an object")
        .insert("future_field".to_owned(), json!({ "ignored": true }));
    assert_eq!(
        serde_json::from_value::<ModuleDescriptor>(encoded).expect("unknown field is ignored"),
        module
    );

    let execute = ExecuteRequest {
        entry: id("armillae.test/execute"),
        input: json!({ "value": 1 }),
    };
    assert_round_trip(&execute);
    assert_round_trip(&ExecuteOutcome {
        entry: execute.entry.clone(),
        output: None,
    });
    let clock = ClockState {
        key: ClockKey {
            clock_type: id("armillae.test/clock"),
            instance: id("primary"),
        },
        value: json!(1),
    };
    assert_round_trip(&clock);
    let advance = AdvanceRequest {
        clock_type: clock.key.clock_type.clone(),
        targets: vec![AdvanceTarget {
            instance: clock.key.instance.clone(),
            step: json!(2),
        }],
    };
    assert_round_trip(&advance);
    assert_round_trip(&AdvanceOutcome {
        clock_type: advance.clock_type.clone(),
        transitions: vec![ClockTransition {
            instance: clock.key.instance,
            before: json!(1),
            step: json!(2),
            after: json!(3),
        }],
    });
    assert_round_trip(&SimulationCapabilities {
        backend: BackendInfo {
            id: id("armillae.test/backend"),
            adapter_version: SemanticVersion::new("0.1.0-alpha.0").expect("valid version"),
            engine: None,
        },
        supported: BTreeSet::new(),
    });

    assert!(serde_json::from_value::<ExecutionPlane>(json!({ "type": "future" })).is_err());
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
    assert_root_schema::<ModuleDescriptor>();
    assert_root_schema::<ExecuteRequest>();
    assert_root_schema::<ExecuteOutcome>();
    assert_root_schema::<ClockState>();
    assert_root_schema::<AdvanceRequest>();
    assert_root_schema::<AdvanceOutcome>();
    assert_root_schema::<SimulationCapabilities>();
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct OpaqueClock(i64);

impl Clock for OpaqueClock {
    type Step = i64;

    fn advance(&self, step: &Self::Step) -> Result<Self, ClockTransitionError> {
        Ok(Self(self.0 + step))
    }
}

#[test]
fn typed_outcomes_do_not_require_debug_or_partial_eq_from_every_clock() {
    let _outcome = TypedAdvanceOutcome::<OpaqueClock> {
        clock_type: id("armillae.test/clock"),
        transitions: Vec::new(),
    };
}

#[test]
fn simulation_trait_is_object_safe() {
    fn accepts_object(_simulation: Option<Box<dyn Simulation>>) {}
    accepts_object(None);
}

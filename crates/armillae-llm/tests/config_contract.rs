use std::{path::PathBuf, sync::Arc};

use armillae_core::{CompletionRequest, CompletionResponse, GenerationOptions};
use armillae_llm::{
    BoxFuture, BridgeCapabilities, BridgeConfig, BridgeError, BridgeFactory, BridgeResolveContext,
    CompletionStream, CredentialRef, EndpointPolicy, LlmBridge, ResolvedBridgeConfig,
    SecretResolver, SecretString, TransportConfig,
};
use futures_executor::block_on;
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use url::Url;

#[test]
fn toml_json_and_builder_produce_the_same_config_model() {
    let toml = r#"
api_version = "armillae.llm/v1alpha1"
driver = "rig"
provider = "openai"
model = "example-model"
endpoint = "https://api.example.com/v1"

[credential]
type = "environment"
name = "EXAMPLE_API_KEY"

[transport]
connect_timeout_ms = 5000
request_timeout_ms = 60000

[defaults]
temperature = 0.7
max_output_tokens = 2048

[provider_options]
reasoning_effort = "medium"
"#;
    let endpoint = Url::parse("https://api.example.com/v1")
        .expect("the test endpoint is a valid absolute URL");
    let builder_config = BridgeConfig::builder("rig", "openai", "example-model")
        .endpoint(endpoint)
        .credential(CredentialRef::Environment {
            name: "EXAMPLE_API_KEY".to_owned(),
        })
        .transport(TransportConfig {
            connect_timeout_ms: 5_000,
            request_timeout_ms: 60_000,
        })
        .defaults(GenerationOptions {
            temperature: Some(0.7),
            max_output_tokens: Some(2_048),
            stop: Vec::new(),
            seed: None,
        })
        .provider_options(json!({"reasoning_effort": "medium"}))
        .build()
        .expect("the builder configuration satisfies the common contract");

    let toml_config = BridgeConfig::from_toml(toml)
        .expect("the TOML configuration satisfies the common contract");
    let json = serde_json::to_string(&builder_config)
        .expect("BridgeConfig is serializable without a resolved Secret");
    let json_config = BridgeConfig::from_json(&json)
        .expect("the JSON configuration satisfies the common contract");

    assert_eq!(toml_config, builder_config);
    assert_eq!(json_config, builder_config);
}

#[test]
fn config_defaults_are_stable() {
    let config = BridgeConfig::from_toml(
        r#"
api_version = "armillae.llm/v1alpha1"
driver = "rig"
provider = "openai"
model = "example-model"
"#,
    )
    .expect("the required fields are enough to construct a config");

    assert_eq!(config.transport, TransportConfig::default());
    assert_eq!(config.defaults, GenerationOptions::default());
    assert_eq!(config.provider_options, json!({}));
}

#[test]
fn common_validation_rejects_invalid_config_without_provider_guessing() {
    let invalid_configs = [
        BridgeConfig::builder("rig", "openai", "model")
            .api_version("armillae.llm/v2")
            .build(),
        BridgeConfig::builder(" ", "openai", "model").build(),
        BridgeConfig::builder("rig", " ", "model").build(),
        BridgeConfig::builder("rig", "openai", " ").build(),
        BridgeConfig::builder("rig", "openai", "model")
            .transport(TransportConfig {
                connect_timeout_ms: 0,
                request_timeout_ms: 60_000,
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .transport(TransportConfig {
                connect_timeout_ms: 5_000,
                request_timeout_ms: 0,
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .provider_options(Value::Null)
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .defaults(GenerationOptions {
                temperature: Some(-0.1),
                ..GenerationOptions::default()
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .defaults(GenerationOptions {
                max_output_tokens: Some(0),
                ..GenerationOptions::default()
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .defaults(GenerationOptions {
                stop: vec![String::new()],
                ..GenerationOptions::default()
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .credential(CredentialRef::Environment {
                name: String::new(),
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .credential(CredentialRef::File {
                path: PathBuf::new(),
            })
            .build(),
        BridgeConfig::builder("rig", "openai", "model")
            .credential(CredentialRef::Resolver { key: String::new() })
            .build(),
    ];

    for result in invalid_configs {
        assert!(matches!(
            result,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }
}

#[test]
fn custom_endpoints_are_allowed_by_default_after_structural_validation() {
    let https_endpoint =
        Url::parse("https://gateway.example.com/v1").expect("the test endpoint is a valid URL");
    let local_endpoint =
        Url::parse("http://127.0.0.1:11434/v1").expect("the local test endpoint is valid");

    assert!(
        BridgeConfig::builder("rig", "openai", "model")
            .endpoint(https_endpoint)
            .build()
            .is_ok()
    );
    assert!(
        BridgeConfig::builder("rig", "openai", "model")
            .endpoint(local_endpoint)
            .build()
            .is_ok()
    );

    let invalid_endpoints = [
        "ftp://gateway.example.com/v1",
        "https://user:password@gateway.example.com/v1",
    ];
    for endpoint in invalid_endpoints {
        let endpoint = Url::parse(endpoint).expect("the URL is syntactically valid for the test");
        assert!(matches!(
            BridgeConfig::builder("rig", "openai", "model")
                .endpoint(endpoint)
                .build(),
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }
}

#[test]
fn endpoint_policy_can_optionally_tighten_the_default() {
    struct ExampleOnly;

    impl EndpointPolicy for ExampleOnly {
        fn validate(&self, endpoint: &Url) -> Result<(), BridgeError> {
            if endpoint.host_str() == Some("gateway.example.com") {
                Ok(())
            } else {
                Err(BridgeError::InvalidConfiguration {
                    message: "endpoint host is not allowed".to_owned(),
                })
            }
        }
    }

    let allowed = BridgeConfig::builder("rig", "openai", "model")
        .endpoint(
            Url::parse("https://gateway.example.com/v1")
                .expect("the allowed endpoint is a valid URL"),
        )
        .build_with_endpoint_policy(&ExampleOnly);
    assert!(allowed.is_ok());

    let rejected = BridgeConfig::builder("rig", "openai", "model")
        .endpoint(
            Url::parse("https://other.example/v1").expect("the rejected endpoint is a valid URL"),
        )
        .build_with_endpoint_policy(&ExampleOnly);
    assert!(matches!(
        rejected,
        Err(BridgeError::InvalidConfiguration { message })
            if message == "endpoint host is not allowed"
    ));

    let allowed_config = BridgeConfig::builder("rig", "openai", "model")
        .endpoint(
            Url::parse("https://gateway.example.com/v1")
                .expect("the allowed endpoint is a valid URL"),
        )
        .build()
        .expect("the endpoint passes structural validation");
    let context = BridgeResolveContext::new().endpoint_policy(&ExampleOnly);
    assert!(block_on(allowed_config.resolve_with(context)).is_ok());

    let rejected_config = BridgeConfig::builder("rig", "openai", "model")
        .endpoint(
            Url::parse("https://other.example/v1").expect("the rejected endpoint is a valid URL"),
        )
        .build()
        .expect("the endpoint passes structural validation");
    let context = BridgeResolveContext::new().endpoint_policy(&ExampleOnly);
    assert!(matches!(
        block_on(rejected_config.resolve_with(context)),
        Err(BridgeError::InvalidConfiguration { message })
            if message == "endpoint host is not allowed"
    ));
}

#[test]
fn resolver_credentials_use_an_object_safe_async_contract() {
    struct StaticResolver;

    impl SecretResolver for StaticResolver {
        fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
            Box::pin(async move {
                if key == "primary" {
                    Ok(SecretString::from("resolver-secret"))
                } else {
                    Err(BridgeError::InvalidConfiguration {
                        message: "unknown Secret key".to_owned(),
                    })
                }
            })
        }
    }

    let resolver: Arc<dyn SecretResolver> = Arc::new(StaticResolver);
    let config = BridgeConfig::builder("rig", "openai", "model")
        .credential(CredentialRef::Resolver {
            key: "primary".to_owned(),
        })
        .build()
        .expect("the resolver reference is valid");

    let context = BridgeResolveContext::new().secret_resolver(resolver.as_ref());
    let resolved = block_on(config.resolve_with(context))
        .expect("the host resolver returns the configured Secret");
    assert_eq!(
        resolved
            .credential()
            .expect("the resolved credential is present")
            .expose_secret(),
        "resolver-secret"
    );

    assert!(matches!(
        block_on(config.resolve()),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
}

#[test]
fn empty_resolved_credentials_are_rejected() {
    struct EmptyResolver;

    impl SecretResolver for EmptyResolver {
        fn resolve<'a>(
            &'a self,
            _key: &'a str,
        ) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
            Box::pin(async { Ok(SecretString::from(String::new())) })
        }
    }

    let resolver_config = BridgeConfig::builder("rig", "openai", "model")
        .credential(CredentialRef::Resolver {
            key: "empty".to_owned(),
        })
        .build()
        .expect("the resolver reference itself is valid");
    assert!(matches!(
        block_on(
            resolver_config
                .resolve_with(BridgeResolveContext::new().secret_resolver(&EmptyResolver),)
        ),
        Err(BridgeError::InvalidConfiguration { .. })
    ));

    let path = unique_secret_path();
    std::fs::write(&path, "\r\n").expect("the test can create an empty Secret file");
    let file_config = BridgeConfig::builder("rig", "openai", "model")
        .credential(CredentialRef::File { path: path.clone() })
        .build()
        .expect("the File credential reference itself is valid");
    assert!(matches!(
        block_on(file_config.resolve()),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
    std::fs::remove_file(path).expect("the temporary Secret file can be removed");
}

#[test]
fn file_credentials_remove_exactly_one_line_ending() {
    let path = unique_secret_path();
    std::fs::write(&path, "  file-secret  \r\n")
        .expect("the test can create its temporary Secret file");
    let config = BridgeConfig::builder("rig", "openai", "model")
        .credential(CredentialRef::File { path: path.clone() })
        .build()
        .expect("the File credential reference is valid");

    let resolved =
        block_on(config.resolve()).expect("the temporary File credential can be resolved");
    assert_eq!(
        resolved
            .credential()
            .expect("the resolved credential is present")
            .expose_secret(),
        "  file-secret  "
    );

    std::fs::write(&path, "file-secret\n\n")
        .expect("the test can replace its temporary Secret file");
    let resolved = block_on(config.resolve()).expect("the updated File credential can be resolved");
    assert_eq!(
        resolved
            .credential()
            .expect("the resolved credential is present")
            .expose_secret(),
        "file-secret\n"
    );

    std::fs::remove_file(path).expect("the temporary Secret file can be removed");
}

#[test]
fn environment_credentials_are_resolved_during_construction() {
    let config = BridgeConfig::builder("rig", "openai", "model")
        .credential(CredentialRef::Environment {
            name: "PATH".to_owned(),
        })
        .build()
        .expect("PATH is a valid Environment credential reference");

    let resolved = block_on(config.resolve())
        .expect("the test process has a non-empty PATH environment variable");
    assert!(
        !resolved
            .credential()
            .expect("the resolved credential is present")
            .expose_secret()
            .is_empty()
    );
}

#[test]
fn config_and_resolved_debug_output_omit_sensitive_values() {
    struct StaticResolver;

    impl SecretResolver for StaticResolver {
        fn resolve<'a>(
            &'a self,
            _key: &'a str,
        ) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
            Box::pin(async { Ok(SecretString::from("resolved-secret-marker")) })
        }
    }

    let config = BridgeConfig::builder("rig", "openai", "model")
        .endpoint(
            Url::parse("https://gateway.example/v1?token=endpoint-secret-marker")
                .expect("the test endpoint is valid"),
        )
        .credential(CredentialRef::Resolver {
            key: "primary".to_owned(),
        })
        .provider_options(json!({"token": "option-secret-marker"}))
        .build()
        .expect("the common configuration is valid");
    let resolved =
        block_on(config.resolve_with(BridgeResolveContext::new().secret_resolver(&StaticResolver)))
            .expect("the test resolver returns a Secret");

    let config_debug = format!("{config:?}");
    assert!(!config_debug.contains("endpoint-secret-marker"));
    assert!(!config_debug.contains("option-secret-marker"));

    let resolved_debug = format!("{resolved:?}");
    assert!(!resolved_debug.contains("resolved-secret-marker"));
    assert!(!resolved_debug.contains("endpoint-secret-marker"));
    assert!(!resolved_debug.contains("option-secret-marker"));
}

#[test]
fn parse_errors_do_not_echo_configuration_contents() {
    let error = BridgeConfig::from_toml("invalid = [\"parse-secret-marker\"")
        .expect_err("the malformed TOML must be rejected");

    assert!(!error.to_string().contains("parse-secret-marker"));
}

#[test]
fn bridge_factory_is_object_safe() {
    struct ContractFactory;
    struct ContractBridge;

    impl LlmBridge for ContractBridge {
        fn capabilities(&self) -> BridgeCapabilities {
            BridgeCapabilities::default()
        }

        fn complete<'a>(
            &'a self,
            _request: CompletionRequest,
        ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>> {
            Box::pin(async {
                Err(BridgeError::InvalidRequest {
                    message: "not scripted".to_owned(),
                })
            })
        }

        fn stream<'a>(
            &'a self,
            _request: CompletionRequest,
        ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>> {
            Box::pin(async {
                Err(BridgeError::InvalidRequest {
                    message: "not scripted".to_owned(),
                })
            })
        }
    }

    impl BridgeFactory for ContractFactory {
        fn driver(&self) -> &'static str {
            "contract"
        }

        fn create<'a>(
            &'a self,
            _config: ResolvedBridgeConfig,
        ) -> BoxFuture<'a, Result<Arc<dyn LlmBridge>, BridgeError>> {
            Box::pin(async { Ok(Arc::new(ContractBridge) as Arc<dyn LlmBridge>) })
        }
    }

    let factory: Arc<dyn BridgeFactory> = Arc::new(ContractFactory);
    let config = BridgeConfig::builder("contract", "mock", "model")
        .build()
        .expect("the factory test configuration is valid");
    let resolved =
        block_on(config.resolve()).expect("a configuration without credentials resolves locally");

    assert_eq!(factory.driver(), "contract");
    assert!(block_on(factory.create(resolved)).is_ok());
}

fn unique_secret_path() -> PathBuf {
    let unique = format!(
        "armillae-llm-secret-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos()
    );
    std::env::temp_dir().join(unique)
}

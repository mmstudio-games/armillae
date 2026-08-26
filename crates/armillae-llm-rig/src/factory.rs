use std::sync::Arc;

use armillae_llm::{BoxFuture, BridgeError, BridgeFactory, LlmBridge, ResolvedBridgeConfig};

use crate::providers;

/// Constructs Rig bridges for the first-phase Provider set.
#[derive(Clone, Copy, Debug, Default)]
pub struct RigBridgeFactory;

impl BridgeFactory for RigBridgeFactory {
    fn driver(&self) -> &'static str {
        "rig"
    }

    fn create<'a>(
        &'a self,
        config: ResolvedBridgeConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn LlmBridge>, BridgeError>> {
        Box::pin(async move { create_bridge(config) })
    }
}

fn create_bridge(config: ResolvedBridgeConfig) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential) = config.into_parts();
    if config.driver != "rig" {
        return invalid_configuration(format!(
            "RigBridgeFactory cannot construct driver: {}",
            config.driver
        ));
    }

    match config.provider.as_str() {
        "anthropic" => providers::anthropic::create(config, credential),
        "deepseek" => providers::deepseek::create(config, credential),
        "minimax" => providers::minimax::create(config, credential),
        "moonshot" => providers::moonshot::create(config, credential),
        "ollama" => providers::ollama::create(config, credential),
        "openai" | "openai-compatible" => providers::openai::create(config, credential),
        provider => invalid_configuration(format!(
            "RigBridgeFactory does not support provider: {provider}"
        )),
    }
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use armillae_llm::{
        BoxFuture, BridgeConfig, BridgeError, BridgeFactory, BridgeResolveContext, CredentialRef,
        SecretResolver, SecretString,
    };

    use super::RigBridgeFactory;

    struct StaticSecretResolver;

    impl SecretResolver for StaticSecretResolver {
        fn resolve<'a>(
            &'a self,
            _key: &'a str,
        ) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
            Box::pin(async { Ok(SecretString::from("factory-test-secret")) })
        }
    }

    fn resolved_config(driver: &str, provider: &str) -> armillae_llm::ResolvedBridgeConfig {
        let config = BridgeConfig::builder(driver, provider, "test-model")
            .credential(CredentialRef::Resolver {
                key: "factory-test".to_owned(),
            })
            .build()
            .expect("factory test config must pass common validation");

        let context = BridgeResolveContext::new()
            .secret_resolver(&StaticSecretResolver as &dyn SecretResolver);
        futures::executor::block_on(config.resolve_with(context))
            .expect("factory test config must resolve")
    }

    #[test]
    fn factory_advertises_rig_driver() {
        assert_eq!(RigBridgeFactory.driver(), "rig");
    }

    #[test]
    fn factory_rejects_wrong_driver_and_unknown_provider() {
        let factory = RigBridgeFactory;
        let wrong_driver =
            futures::executor::block_on(factory.create(resolved_config("other", "openai")));
        let unsupported_provider =
            futures::executor::block_on(factory.create(resolved_config("rig", "unknown")));

        assert!(matches!(
            wrong_driver,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            unsupported_provider,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn factory_routes_named_openai_compatible_providers() {
        let factory = RigBridgeFactory;

        for provider in ["deepseek", "minimax", "moonshot"] {
            futures::executor::block_on(factory.create(resolved_config("rig", provider)))
                .unwrap_or_else(|error| panic!("factory must route {provider}: {error}"));
        }
    }

    #[test]
    fn factory_routes_anthropic_provider() {
        let factory = RigBridgeFactory;

        futures::executor::block_on(factory.create(resolved_config("rig", "anthropic")))
            .expect("factory must route Anthropic");
    }

    #[test]
    fn factory_routes_ollama_without_requiring_a_credential() {
        let config = BridgeConfig::builder("rig", "ollama", "test-model")
            .build()
            .expect("Ollama factory test config must validate");
        let resolved = futures::executor::block_on(config.resolve())
            .expect("Ollama factory test config must resolve");

        futures::executor::block_on(RigBridgeFactory.create(resolved))
            .expect("factory must route Ollama");
    }
}

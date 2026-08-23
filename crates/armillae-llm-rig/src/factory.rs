use std::sync::Arc;

use armillae_llm::{BoxFuture, BridgeError, BridgeFactory, LlmBridge, ResolvedBridgeConfig};

use crate::providers;

/// Constructs non-streaming Rig bridges for the first-phase Provider set.
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
        "deepseek" => providers::deepseek::create(config, credential),
        "minimax" => providers::minimax::create(config, credential),
        "moonshot" => providers::moonshot::create(config, credential),
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
        BoxFuture, BridgeConfig, BridgeError, BridgeFactory, CredentialRef, SecretResolver,
        SecretString,
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

        futures::executor::block_on(
            config.resolve(Some(&StaticSecretResolver as &dyn SecretResolver), None),
        )
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
            futures::executor::block_on(factory.create(resolved_config("rig", "anthropic")));

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
}

use armillae_llm::{
    BoxFuture, BridgeConfig, BridgeError, CredentialRef, SecretResolver, SecretString,
};
use serde_json::Value;

struct StaticSecretResolver;

impl SecretResolver for StaticSecretResolver {
    fn resolve<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
        Box::pin(async { Ok(SecretString::from("named-provider-test-secret")) })
    }
}

pub(super) fn resolved_config(
    provider: &str,
    endpoint: Option<&str>,
    with_credential: bool,
    provider_options: Value,
) -> (BridgeConfig, Option<SecretString>) {
    let mut builder =
        BridgeConfig::builder("rig", provider, "test-model").provider_options(provider_options);
    if let Some(endpoint) = endpoint {
        builder = builder.endpoint(
            endpoint
                .parse()
                .expect("provider test endpoint must be a valid URL"),
        );
    }
    if with_credential {
        builder = builder.credential(CredentialRef::Resolver {
            key: "named-provider-test".to_owned(),
        });
    }
    let config = builder
        .build()
        .expect("provider test config must pass common validation");
    let resolved = futures::executor::block_on(config.resolve(
        with_credential.then_some(&StaticSecretResolver as &dyn SecretResolver),
        None,
    ))
    .expect("provider test config must resolve");

    resolved.into_parts()
}

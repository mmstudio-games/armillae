use std::time::Duration;

use armillae_llm::{BridgeConfig, BridgeError};
use rig_core::http_client::ReqwestClient;
use secrecy::SecretString;

use crate::request::OpenAiRequestMapper;

pub(crate) mod deepseek;
pub(crate) mod minimax;
pub(crate) mod moonshot;
pub(crate) mod openai;

#[cfg(test)]
mod test_support;

fn validate_named_config(
    config: BridgeConfig,
    credential: Option<SecretString>,
    provider: &'static str,
    provider_label: &'static str,
) -> Result<(BridgeConfig, SecretString, OpenAiRequestMapper), BridgeError> {
    if config.provider != provider {
        return invalid_configuration(format!(
            "{provider_label} provider module cannot construct provider: {}",
            config.provider
        ));
    }

    let credential = credential.ok_or_else(|| BridgeError::InvalidConfiguration {
        message: format!("{provider} requires a credential"),
    })?;
    let request_mapper = OpenAiRequestMapper::for_named_provider(
        provider,
        provider_label,
        config.provider_options.clone(),
    )?;

    Ok((config, credential, request_mapper))
}

fn build_http_client(config: &BridgeConfig) -> Result<ReqwestClient, BridgeError> {
    ReqwestClient::builder()
        .connect_timeout(Duration::from_millis(config.transport.connect_timeout_ms))
        .timeout(Duration::from_millis(config.transport.request_timeout_ms))
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig HTTP client".to_owned(),
        })
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

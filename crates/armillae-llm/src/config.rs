use std::{fmt, path::PathBuf};

use armillae_core::GenerationOptions;
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use url::Url;

use crate::{BoxFuture, BridgeError};

pub const BRIDGE_CONFIG_API_VERSION: &str = "armillae.llm/v1alpha1";

/// Serializable configuration for constructing one LLM Bridge.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeConfig {
    pub api_version: String,
    pub driver: String,
    pub provider: String,
    pub model: String,
    pub endpoint: Option<Url>,
    pub credential: Option<CredentialRef>,
    #[serde(default)]
    pub transport: TransportConfig,
    #[serde(default, deserialize_with = "deserialize_generation_options")]
    pub defaults: GenerationOptions,
    #[serde(default = "empty_json_object")]
    pub provider_options: Value,
}

impl BridgeConfig {
    pub fn builder(
        driver: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> BridgeConfigBuilder {
        BridgeConfigBuilder::new(driver, provider, model)
    }

    pub fn from_toml(input: &str) -> Result<Self, BridgeError> {
        let config: Self =
            toml::from_str(input).map_err(|_| BridgeError::InvalidConfiguration {
                message: "failed to parse BridgeConfig as TOML".to_owned(),
            })?;
        config.validate(None)?;
        Ok(config)
    }

    pub fn from_json(input: &str) -> Result<Self, BridgeError> {
        let config: Self =
            serde_json::from_str(input).map_err(|_| BridgeError::InvalidConfiguration {
                message: "failed to parse BridgeConfig as JSON".to_owned(),
            })?;
        config.validate(None)?;
        Ok(config)
    }

    /// Validates cross-Provider configuration and an optional host policy.
    pub fn validate(
        &self,
        endpoint_policy: Option<&dyn EndpointPolicy>,
    ) -> Result<(), BridgeError> {
        if self.api_version != BRIDGE_CONFIG_API_VERSION {
            return invalid_configuration(format!("unsupported api_version: {}", self.api_version));
        }
        validate_non_empty("driver", &self.driver)?;
        validate_non_empty("provider", &self.provider)?;
        validate_non_empty("model", &self.model)?;
        self.transport.validate()?;
        validate_generation_options(&self.defaults)?;

        if !self.provider_options.is_object() {
            return invalid_configuration("provider_options must be a JSON object");
        }

        if let Some(credential) = &self.credential {
            credential.validate()?;
        }

        if let Some(endpoint) = &self.endpoint {
            validate_endpoint(endpoint)?;
            if let Some(policy) = endpoint_policy {
                policy.validate(endpoint)?;
            }
        }

        Ok(())
    }

    /// Resolves built-in credential references using the default host context.
    pub fn resolve(&self) -> BoxFuture<'_, Result<ResolvedBridgeConfig, BridgeError>> {
        self.resolve_with(BridgeResolveContext::new())
    }

    /// Resolves the credential with optional host services and policies.
    pub fn resolve_with<'a>(
        &'a self,
        context: BridgeResolveContext<'a>,
    ) -> BoxFuture<'a, Result<ResolvedBridgeConfig, BridgeError>> {
        Box::pin(async move {
            self.validate(context.endpoint_policy)?;

            let credential = match &self.credential {
                None => None,
                Some(CredentialRef::Environment { name }) => {
                    let value =
                        std::env::var(name).map_err(|_| BridgeError::InvalidConfiguration {
                            message: "failed to resolve Environment credential".to_owned(),
                        })?;
                    Some(to_non_empty_secret(value, "Environment")?)
                }
                Some(CredentialRef::File { path }) => {
                    let value = std::fs::read_to_string(path).map_err(|_| {
                        BridgeError::InvalidConfiguration {
                            message: "failed to resolve File credential".to_owned(),
                        }
                    })?;
                    Some(to_non_empty_secret(remove_one_line_ending(value), "File")?)
                }
                Some(CredentialRef::Resolver { key }) => {
                    let resolver = context.secret_resolver.ok_or_else(|| {
                        BridgeError::InvalidConfiguration {
                            message: "Resolver credential requires a SecretResolver".to_owned(),
                        }
                    })?;
                    let secret = resolver.resolve(key).await?;
                    if secret_is_empty(&secret) {
                        return invalid_configuration(
                            "Resolver credential resolved to an empty Secret",
                        );
                    }
                    Some(secret)
                }
            };

            Ok(ResolvedBridgeConfig {
                config: self.clone(),
                credential,
            })
        })
    }
}

impl fmt::Debug for BridgeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BridgeConfig")
            .field("api_version", &self.api_version)
            .field("driver", &self.driver)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint.as_ref().map(|_| "[configured]"))
            .field("credential", &self.credential)
            .field("transport", &self.transport)
            .field("defaults", &self.defaults)
            .field("provider_options", &"[omitted]")
            .finish()
    }
}

/// Network transport settings independent of generation and retry policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportConfig {
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

impl TransportConfig {
    pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 5_000;
    pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 60_000;

    pub fn validate(&self) -> Result<(), BridgeError> {
        if self.connect_timeout_ms == 0 {
            return invalid_configuration("connect_timeout_ms must be greater than zero");
        }
        if self.request_timeout_ms == 0 {
            return invalid_configuration("request_timeout_ms must be greater than zero");
        }
        Ok(())
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: Self::DEFAULT_CONNECT_TIMEOUT_MS,
            request_timeout_ms: Self::DEFAULT_REQUEST_TIMEOUT_MS,
        }
    }
}

/// A serializable reference to a credential, never the Secret value itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialRef {
    Environment { name: String },
    File { path: PathBuf },
    Resolver { key: String },
}

impl CredentialRef {
    fn validate(&self) -> Result<(), BridgeError> {
        match self {
            Self::Environment { name } => validate_non_empty("credential environment name", name),
            Self::File { path } if path.as_os_str().is_empty() => {
                invalid_configuration("credential file path must not be empty")
            }
            Self::File { .. } => Ok(()),
            Self::Resolver { key } => validate_non_empty("credential resolver key", key),
        }
    }
}

/// A host-provided asynchronous resolver for named Secret references.
pub trait SecretResolver: Send + Sync {
    fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SecretString, BridgeError>>;
}

/// An optional host restriction applied after structural endpoint validation.
pub trait EndpointPolicy: Send + Sync {
    fn validate(&self, endpoint: &Url) -> Result<(), BridgeError>;
}

/// Optional host-owned services and policies used while resolving a Bridge configuration.
#[derive(Clone, Copy, Default)]
pub struct BridgeResolveContext<'a> {
    secret_resolver: Option<&'a dyn SecretResolver>,
    endpoint_policy: Option<&'a dyn EndpointPolicy>,
}

impl<'a> BridgeResolveContext<'a> {
    /// Creates a context without host-provided services or policies.
    pub const fn new() -> Self {
        Self {
            secret_resolver: None,
            endpoint_policy: None,
        }
    }

    /// Uses a host resolver for [`CredentialRef::Resolver`] credentials.
    pub fn secret_resolver(mut self, resolver: &'a dyn SecretResolver) -> Self {
        self.secret_resolver = Some(resolver);
        self
    }

    /// Applies a host restriction to an explicitly configured endpoint.
    pub fn endpoint_policy(mut self, policy: &'a dyn EndpointPolicy) -> Self {
        self.endpoint_policy = Some(policy);
        self
    }
}

/// A validated Bridge configuration paired with its non-serializable Secret.
#[derive(Clone)]
pub struct ResolvedBridgeConfig {
    config: BridgeConfig,
    credential: Option<SecretString>,
}

impl ResolvedBridgeConfig {
    pub fn config(&self) -> &BridgeConfig {
        &self.config
    }

    pub fn credential(&self) -> Option<&SecretString> {
        self.credential.as_ref()
    }

    pub fn into_parts(self) -> (BridgeConfig, Option<SecretString>) {
        (self.config, self.credential)
    }
}

impl fmt::Debug for ResolvedBridgeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedBridgeConfig")
            .field("config", &self.config)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// A Rust builder producing the same validated configuration model as TOML and JSON.
pub struct BridgeConfigBuilder {
    config: BridgeConfig,
}

impl BridgeConfigBuilder {
    pub fn new(
        driver: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            config: BridgeConfig {
                api_version: BRIDGE_CONFIG_API_VERSION.to_owned(),
                driver: driver.into(),
                provider: provider.into(),
                model: model.into(),
                endpoint: None,
                credential: None,
                transport: TransportConfig::default(),
                defaults: GenerationOptions::default(),
                provider_options: empty_json_object(),
            },
        }
    }

    pub fn api_version(mut self, api_version: impl Into<String>) -> Self {
        self.config.api_version = api_version.into();
        self
    }

    pub fn endpoint(mut self, endpoint: Url) -> Self {
        self.config.endpoint = Some(endpoint);
        self
    }

    pub fn credential(mut self, credential: CredentialRef) -> Self {
        self.config.credential = Some(credential);
        self
    }

    pub fn transport(mut self, transport: TransportConfig) -> Self {
        self.config.transport = transport;
        self
    }

    pub fn defaults(mut self, defaults: GenerationOptions) -> Self {
        self.config.defaults = defaults;
        self
    }

    pub fn provider_options(mut self, provider_options: Value) -> Self {
        self.config.provider_options = provider_options;
        self
    }

    pub fn build(self) -> Result<BridgeConfig, BridgeError> {
        self.config.validate(None)?;
        Ok(self.config)
    }

    pub fn build_with_endpoint_policy(
        self,
        endpoint_policy: &dyn EndpointPolicy,
    ) -> Result<BridgeConfig, BridgeError> {
        self.config.validate(Some(endpoint_policy))?;
        Ok(self.config)
    }
}

fn default_connect_timeout_ms() -> u64 {
    TransportConfig::DEFAULT_CONNECT_TIMEOUT_MS
}

fn default_request_timeout_ms() -> u64 {
    TransportConfig::DEFAULT_REQUEST_TIMEOUT_MS
}

fn empty_json_object() -> Value {
    Value::Object(Map::new())
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConfigGenerationOptions {
    temperature: Option<f64>,
    max_output_tokens: Option<u64>,
    stop: Vec<String>,
    seed: Option<u64>,
}

impl Default for ConfigGenerationOptions {
    fn default() -> Self {
        let defaults = GenerationOptions::default();
        Self {
            temperature: defaults.temperature,
            max_output_tokens: defaults.max_output_tokens,
            stop: defaults.stop,
            seed: defaults.seed,
        }
    }
}

fn deserialize_generation_options<'de, D>(deserializer: D) -> Result<GenerationOptions, D::Error>
where
    D: Deserializer<'de>,
{
    let options = ConfigGenerationOptions::deserialize(deserializer)?;
    Ok(GenerationOptions {
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
        stop: options.stop,
        seed: options.seed,
    })
}

fn validate_non_empty(field: &str, value: &str) -> Result<(), BridgeError> {
    if value.trim().is_empty() {
        return invalid_configuration(format!("{field} must not be empty"));
    }
    Ok(())
}

fn validate_generation_options(options: &GenerationOptions) -> Result<(), BridgeError> {
    if options
        .temperature
        .is_some_and(|temperature| !temperature.is_finite() || temperature < 0.0)
    {
        return invalid_configuration("temperature must be finite and non-negative");
    }
    if options.max_output_tokens == Some(0) {
        return invalid_configuration("max_output_tokens must be greater than zero");
    }
    if options.stop.iter().any(String::is_empty) {
        return invalid_configuration("stop strings must not be empty");
    }
    Ok(())
}

fn validate_endpoint(endpoint: &Url) -> Result<(), BridgeError> {
    if !matches!(endpoint.scheme(), "http" | "https") {
        return invalid_configuration("endpoint scheme must be HTTP or HTTPS");
    }
    if endpoint.host_str().is_none() {
        return invalid_configuration("endpoint must contain a host");
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return invalid_configuration("endpoint must not contain userinfo");
    }
    Ok(())
}

fn remove_one_line_ending(mut value: String) -> String {
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    value
}

fn to_non_empty_secret(value: String, kind: &str) -> Result<SecretString, BridgeError> {
    if value.is_empty() {
        return invalid_configuration(format!("{kind} credential resolved to an empty Secret"));
    }
    Ok(SecretString::from(value))
}

fn secret_is_empty(secret: &SecretString) -> bool {
    use secrecy::ExposeSecret;

    secret.expose_secret().is_empty()
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

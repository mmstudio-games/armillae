//! Runtime-independent LLM Bridge contracts and implementations.

mod bridge;
mod capability;
mod config;
mod error;
mod factory;
#[cfg(feature = "mock")]
pub mod mock;

pub use bridge::{BoxFuture, CompletionStream, LlmBridge};
pub use capability::{BridgeCapabilities, OutputFormatCapabilities, ToolChoiceCapabilities};
pub use config::{
    BRIDGE_CONFIG_API_VERSION, BridgeConfig, BridgeConfigBuilder, CredentialRef, EndpointPolicy,
    ResolvedBridgeConfig, SecretResolver, TransportConfig,
};
pub use error::{BridgeError, ErrorMetadata};
pub use factory::BridgeFactory;
#[cfg(feature = "mock")]
pub use mock::{MockBridge, MockResponse};
pub use secrecy::SecretString;

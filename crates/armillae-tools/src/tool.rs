use std::{borrow::Cow, future::Future};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::{IntoToolOutput, ToolContext};

/// A type-safe Tool implementation authored by an application.
pub trait Tool: Send + Sync {
    type Args: DeserializeOwned + JsonSchema + Send;
    type Output: IntoToolOutput + Send;
    type Error: std::error::Error + Send + Sync + 'static;

    const NAME: &'static str;

    fn description(&self) -> Cow<'static, str>;

    fn call(
        &self,
        context: ToolContext,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

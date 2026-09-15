//! Explicit, lossless reasoning controls for the pinned Rig request boundary.
use armillae_core::{GenerationOptions, ReasoningEffort as Effort, Thinking, ToolChoice};
use armillae_llm::{BridgeError, ReasoningCapabilities};
use rig_core::completion::CompletionRequest;
use serde_json::{Map, Value, json};

pub(crate) fn capabilities(provider: &str, model: &str) -> ReasoningCapabilities {
    let mut result = ReasoningCapabilities::NONE;
    match provider {
        "openai" | "openai-compatible" => {
            result.enabled = true;
            result.disabled = true;
            result.efforts = &[
                Effort::None,
                Effort::Minimal,
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::XHigh,
                Effort::Max,
            ];
        }
        "deepseek" => {
            result.enabled = true;
            result.disabled = true;
            result.efforts = &[Effort::Low, Effort::High, Effort::Max];
        }
        "moonshot" => {
            result.enabled = matches!(
                model,
                "kimi-k2.5" | "kimi-k2.6" | "kimi-k2.7-code" | "kimi-k2.7-code-highspeed"
            );
            result.disabled = matches!(model, "kimi-k2.5" | "kimi-k2.6");
            if model == "kimi-k3" {
                result.efforts = &[Effort::Low, Effort::High, Effort::Max];
            }
        }
        "minimax" => {
            if model == "MiniMax-M3" {
                result.enabled = true;
                result.disabled = true;
                result.adaptive = true;
            }
        }
        "anthropic" => {
            result.disabled = true;
            result.adaptive = true;
            result.budget = true;
            result.efforts = &[
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::XHigh,
                Effort::Max,
            ];
            // These documented models reject manual budgets or disabling thinking.
            if model.starts_with("claude-fable-") || model.starts_with("claude-mythos-") {
                result.disabled = false;
                result.budget = false;
            }
            if [
                "claude-opus-4-7",
                "claude-opus-4-8",
                "claude-opus-5",
                "claude-sonnet-5",
            ]
            .iter()
            .any(|prefix| model.starts_with(prefix))
            {
                result.budget = false;
            }
            if [
                "claude-opus-4-5",
                "claude-sonnet-4-5",
                "claude-haiku-4-5",
                "claude-sonnet-4-0",
                "claude-opus-4-0",
            ]
            .iter()
            .any(|prefix| model.starts_with(prefix))
            {
                result.adaptive = false;
                result.efforts = if model.starts_with("claude-opus-4-5") {
                    &[Effort::Low, Effort::Medium, Effort::High]
                } else {
                    &[]
                };
            }
            if model.starts_with("claude-opus-4-6") || model.starts_with("claude-sonnet-4-6") {
                result.efforts = &[Effort::Low, Effort::Medium, Effort::High, Effort::Max];
            }
        }
        "ollama" => {
            let gpt_oss = model
                .split('/')
                .next_back()
                .is_some_and(|name| name.split(':').next() == Some("gpt-oss"));
            result.enabled = !gpt_oss;
            result.disabled = !gpt_oss;
            result.efforts = if gpt_oss {
                &[Effort::Low, Effort::Medium, Effort::High]
            } else {
                &[Effort::Low, Effort::Medium, Effort::High, Effort::Max]
            };
        }
        _ => {}
    }
    result
}

pub(crate) fn parameters(
    provider: &str,
    model: &str,
    options: &GenerationOptions,
    tool_choice: Option<&ToolChoice>,
) -> Result<Map<String, Value>, BridgeError> {
    capabilities(provider, model).validate(options)?;
    let thinking = options
        .thinking
        .filter(|value| *value != Thinking::ProviderDefault);
    let effort = options
        .reasoning_effort
        .filter(|value| *value != Effort::ProviderDefault);
    let enabled = matches!(
        thinking,
        Some(Thinking::Enabled | Thinking::Adaptive | Thinking::Budget { .. })
    );
    if enabled && effort == Some(Effort::None) {
        return invalid("enabled thinking conflicts with reasoning_effort=none");
    }
    if provider != "anthropic"
        && thinking == Some(Thinking::Disabled)
        && effort.is_some_and(|value| value != Effort::None)
    {
        return invalid("disabled thinking conflicts with non-zero reasoning effort");
    }
    let mut params = Map::new();
    match provider {
        "openai" | "openai-compatible" => {
            if thinking == Some(Thinking::Enabled) && effort.is_none() {
                return invalid(
                    "OpenAI thinking=enabled requires an explicit non-none reasoning_effort",
                );
            }
            let effort = if thinking == Some(Thinking::Disabled) {
                Some(Effort::None)
            } else {
                effort
            };
            if let Some(effort) = effort {
                params.insert("reasoning_effort".into(), json!(effort));
            }
        }
        "deepseek" | "moonshot" | "minimax" => {
            if provider == "deepseek"
                && (enabled || effort.is_some())
                && options.temperature.is_some()
            {
                return invalid("DeepSeek thinking does not honor temperature");
            }
            if let Some(thinking) = thinking {
                let mode = match thinking {
                    Thinking::Disabled => "disabled",
                    Thinking::Enabled if provider != "minimax" => "enabled",
                    Thinking::Enabled | Thinking::Adaptive => "adaptive",
                    _ => return unsupported(),
                };
                params.insert("thinking".into(), json!({"type": mode}));
            }
            if let Some(effort) = effort {
                params.insert("reasoning_effort".into(), json!(effort));
            }
        }
        "anthropic" => {
            if enabled {
                if options.temperature.is_some_and(|value| value != 1.0) {
                    return invalid("Anthropic thinking requires temperature=1 or no temperature");
                }
                if matches!(
                    tool_choice,
                    Some(ToolChoice::Required | ToolChoice::Specific { .. })
                ) {
                    return invalid("Anthropic thinking does not support forced ToolChoice");
                }
            }
            if model.starts_with("claude-opus-5")
                && thinking == Some(Thinking::Disabled)
                && matches!(effort, Some(Effort::XHigh | Effort::Max))
            {
                return invalid("Claude Opus 5 cannot disable thinking above high effort");
            }
            if let Some(thinking) = thinking {
                let value = match thinking {
                    Thinking::Disabled => json!({"type": "disabled"}),
                    Thinking::Adaptive => json!({"type": "adaptive"}),
                    Thinking::Budget { tokens } => {
                        if tokens < 1024
                            || options.max_output_tokens.is_none_or(|max| tokens >= max)
                        {
                            return invalid(
                                "Anthropic thinking budget must be >=1024 and less than max_output_tokens",
                            );
                        }
                        json!({"type": "enabled", "budget_tokens": tokens})
                    }
                    _ => return unsupported(),
                };
                params.insert("thinking".into(), value);
            }
            if let Some(effort) = effort {
                params.insert("output_config".into(), json!({"effort": effort}));
            }
        }
        "ollama" => {
            if let Some(effort) = effort {
                params.insert("think".into(), json!(effort));
            } else if let Some(thinking) = thinking {
                params.insert("think".into(), json!(thinking == Thinking::Enabled));
            }
        }
        _ => {}
    }
    Ok(params)
}

/// Merge at the shared request boundary so complete and stream use identical wire controls.
pub(crate) fn apply(
    request: &mut CompletionRequest,
    mut controls: Map<String, Value>,
) -> Result<(), BridgeError> {
    if controls.is_empty() {
        return Ok(());
    }
    // Rig's typed output_config only holds format and is flattened alongside additional_params.
    // Move the already-validated schema into the same object as effort to avoid duplicate keys.
    if let Some(output) = controls.get_mut("output_config")
        && let Some(schema) = request.output_schema.take()
    {
        output["format"] = json!({"type": "json_schema", "schema": schema.to_value()});
    }
    let additional = request.additional_params.get_or_insert_with(|| json!({}));
    let Some(additional) = additional.as_object_mut() else {
        return invalid("Rig additional parameters must be an object");
    };
    additional.extend(controls);
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidRequest {
        message: message.to_owned(),
    })
}
fn unsupported<T>() -> Result<T, BridgeError> {
    Err(BridgeError::UnsupportedCapability {
        capability: "generation.thinking".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_generation_and_tool_constraints_are_enforced() {
        let mut options = GenerationOptions {
            thinking: Some(Thinking::Budget { tokens: 1024 }),
            max_output_tokens: Some(2048),
            ..Default::default()
        };
        assert!(parameters("anthropic", "claude-sonnet-4-5", &options, None).is_ok());
        assert!(
            parameters(
                "anthropic",
                "claude-sonnet-4-5",
                &options,
                Some(&ToolChoice::Required)
            )
            .is_err()
        );
        options.max_output_tokens = Some(1024);
        assert!(parameters("anthropic", "claude-sonnet-4-5", &options, None).is_err());
        options.max_output_tokens = Some(2048);
        options.temperature = Some(0.7);
        assert!(parameters("anthropic", "claude-sonnet-4-5", &options, None).is_err());
        options.thinking = Some(Thinking::Enabled);
        assert!(parameters("deepseek", "deepseek-v4-pro", &options, None).is_err());
        options.temperature = None;
        assert!(parameters("deepseek", "deepseek-v4-pro", &options, None).is_ok());
        options.reasoning_effort = Some(Effort::None);
        assert!(parameters("openai", "model", &options, None).is_err());
    }

    #[test]
    fn anthropic_effort_is_independent_of_disabled_thinking() {
        let options = GenerationOptions {
            thinking: Some(Thinking::Disabled),
            reasoning_effort: Some(Effort::Low),
            ..Default::default()
        };
        let params = parameters("anthropic", "claude-sonnet-4-6", &options, None).unwrap();
        assert_eq!(params["thinking"], json!({"type":"disabled"}));
        assert_eq!(params["output_config"], json!({"effort":"low"}));
    }
}

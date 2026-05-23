use futures::StreamExt;
use reqwest::Client;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::event::AppEvent;
use crate::types::ResponseStats;

/// Spawn a streaming LLM request for the main agent.
/// Returns the JoinHandle so the caller can abort it on Ctrl+B.
pub fn spawn_stream(
    config: &Config,
    messages: Vec<serde_json::Value>,
    tx: mpsc::UnboundedSender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    let provider = config.agents.main.provider.clone();
    let model = config.agents.main.model.clone();
    let thinking = config.agents.main.thinking.clone();
    let api_key = config.resolve_api_key(&provider).unwrap_or_default();
    let base_url = resolve_base_url(config, &provider);

    tokio::spawn(async move {
        if let Err(e) = do_stream(
            &base_url, &api_key, &model, &thinking, &messages, &tx, false,
        )
        .await
        {
            let _ = tx.send(AppEvent::LlmError(crate::types::LlmError::NetworkError {
                message: e.to_string(),
            }));
        }
    })
}

/// Spawn a streaming LLM request for the query overlay agent.
/// Returns the JoinHandle so the caller can abort it on Ctrl+B.
pub fn spawn_overlay_stream(
    config: &Config,
    messages: Vec<serde_json::Value>,
    tx: mpsc::UnboundedSender<AppEvent>,
) -> tokio::task::JoinHandle<()> {
    let provider = config.agents.query.provider.clone();
    let model = config.agents.query.model.clone();
    let thinking = config.agents.query.thinking.clone();
    let api_key = config.resolve_api_key(&provider).unwrap_or_default();
    let base_url = resolve_base_url(config, &provider);

    tokio::spawn(async move {
        if let Err(e) =
            do_stream(&base_url, &api_key, &model, &thinking, &messages, &tx, true).await
        {
            let _ = tx.send(AppEvent::OverlayError(e.to_string()));
        }
    })
}

fn resolve_base_url(config: &Config, provider: &str) -> String {
    match provider {
        "deepseek" => {
            let url = &config.providers.deepseek.base_url;
            if url.is_empty() {
                "https://api.deepseek.com".to_string()
            } else {
                url.clone()
            }
        }
        _ => "https://api.deepseek.com".to_string(),
    }
}

async fn do_stream(
    base_url: &str,
    api_key: &str,
    model: &str,
    thinking: &crate::config::ThinkingConfig,
    messages: &[serde_json::Value],
    tx: &mpsc::UnboundedSender<AppEvent>,
    is_overlay: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let started_at = Instant::now();
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true}
    });
    if thinking.enabled {
        body["thinking"] = serde_json::json!({"type": "enabled"});
        body["reasoning_effort"] = serde_json::json!(thinking.reasoning_effort);
    }

    let client = Client::new();
    let resp = client
        .post(format!("{}/chat/completions", base_url))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error ({}): {}", status, text).into());
    }

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut in_thinking = false;
    let mut usage_data: Option<serde_json::Value> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() {
                continue;
            }
            if line == "data: [DONE]" {
                if is_overlay {
                    let _ = tx.send(AppEvent::OverlayResponseComplete);
                } else {
                    let stats = usage_data.map(|u| ResponseStats {
                        input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as usize,
                        output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as usize,
                        cost: estimate_cost(model, &u),
                        duration_secs: started_at.elapsed().as_secs_f64(),
                    });
                    let _ = tx.send(AppEvent::ResponseComplete(stats));
                }
                return Ok(());
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if !is_overlay {
                        if let Some(usage) = data.get("usage") {
                            if !usage.is_null() {
                                usage_data = Some(usage.clone());
                            }
                        }
                    }

                    if let Some(delta) = data["choices"].get(0).and_then(|c| c.get("delta")) {
                        if let Some(reasoning) = delta.get("reasoning_content") {
                            if let Some(text) = reasoning.as_str() {
                                if !text.is_empty() {
                                    if !in_thinking {
                                        in_thinking = true;
                                        let _ = tx.send(if is_overlay {
                                            AppEvent::OverlayThinkStart
                                        } else {
                                            AppEvent::ThinkStart
                                        });
                                    }
                                    let _ = tx.send(if is_overlay {
                                        AppEvent::OverlayToken(format!("\x00THINK:{}", text))
                                    } else {
                                        AppEvent::Token(format!("\x00THINK:{}", text))
                                    });
                                }
                            }
                        }

                        if let Some(content) = delta.get("content") {
                            if let Some(text) = content.as_str() {
                                if !text.is_empty() {
                                    if in_thinking {
                                        in_thinking = false;
                                        let _ = tx.send(if is_overlay {
                                            AppEvent::OverlayThinkEnd
                                        } else {
                                            AppEvent::ThinkEnd
                                        });
                                    }
                                    let _ = tx.send(if is_overlay {
                                        AppEvent::OverlayToken(text.to_string())
                                    } else {
                                        AppEvent::Token(text.to_string())
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if in_thinking {
        let _ = tx.send(if is_overlay {
            AppEvent::OverlayThinkEnd
        } else {
            AppEvent::ThinkEnd
        });
    }
    if is_overlay {
        let _ = tx.send(AppEvent::OverlayResponseComplete);
    } else {
        let stats = usage_data.map(|u| ResponseStats {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as usize,
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as usize,
            cost: estimate_cost(model, &u),
            duration_secs: started_at.elapsed().as_secs_f64(),
        });
        let _ = tx.send(AppEvent::ResponseComplete(stats));
    }
    Ok(())
}

pub fn estimate_cost(model: &str, usage: &serde_json::Value) -> f64 {
    let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0) as f64;
    let prompt_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as f64;
    let cache_hit_tokens = usage["prompt_cache_hit_tokens"].as_u64().unwrap_or(0) as f64;
    let cache_miss_tokens = usage["prompt_cache_miss_tokens"]
        .as_u64()
        .map(|n| n as f64)
        .unwrap_or_else(|| (prompt_tokens - cache_hit_tokens).max(0.0));

    if let Some(price) = pricing_for_model(model) {
        ((cache_hit_tokens * price.input_cache_hit)
            + (cache_miss_tokens * price.input_cache_miss)
            + (output_tokens * price.output))
            / 1_000_000.0
    } else {
        0.0
    }
}

struct ModelPricing {
    input_cache_hit: f64,
    input_cache_miss: f64,
    output: f64,
}

fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    match model {
        // DeepSeek official API prices are USD per 1M tokens as of 2026-05-22:
        // https://api-docs.deepseek.com/quick_start/pricing/
        "deepseek-v4-pro" => Some(ModelPricing {
            input_cache_hit: 0.003625,
            input_cache_miss: 0.435,
            output: 0.87,
        }),
        "deepseek-v4-flash" | "deepseek-chat" | "deepseek-reasoner" => Some(ModelPricing {
            input_cache_hit: 0.0028,
            input_cache_miss: 0.14,
            output: 0.28,
        }),
        _ => None,
    }
}

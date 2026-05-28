//! End-to-end probe for the Xiaomi MiMo provider.
//!
//! Reads `XIAOMI_API_KEY` from the environment, builds a Xiaomi provider via
//! the same `LlmProvider` path the app uses at runtime, and issues a one-shot
//! request through `LlmClient::one_shot_completion`.
//!
//! Usage:
//!
//! ```bash
//! export XIAOMI_API_KEY='sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'
//! # Optional overrides:
//! #   XIAOMI_OPENAI_BASE_URL=https://api.xiaomimimo.com/v1
//! #   XIAOMI_ANTHROPIC_BASE_URL=https://api.xiaomimimo.com/anthropic
//! #   XIAOMI_MODEL=mimo-v2.5-pro            # default
//! #   XIAOMI_PROTOCOL=openai                # openai|anthropic|auto
//! cargo run --example xiaomi_live_probe -p golish-llm-providers
//! ```
//!
//! The probe prints the HTTP-level outcome. A 402 "Insufficient account
//! balance" still counts as a successful integration: the request was
//! accepted, parsed, and the upstream rejected only on billing — which means
//! header, URL, and body shape are all correct.

use std::env;

use anyhow::Context;
use golish_llm_providers::xiaomi::{XiaomiProtocol, XiaomiRegion};
use golish_llm_providers::{LlmClient, LlmProvider, XiaomiProviderImpl};

fn main() -> anyhow::Result<()> {
    // Build a single-threaded Tokio runtime so the example doesn't need to
    // depend on the `tokio_macros` proc-macro crate (avoids touching
    // workspace dev-deps for a probe utility).
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime")?;
    runtime.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let api_key =
        env::var("XIAOMI_API_KEY").context("XIAOMI_API_KEY env var is required for this probe")?;

    let model = env::var("XIAOMI_MODEL").unwrap_or_else(|_| "mimo-v2.5-pro".to_string());
    let protocol = XiaomiProtocol::from_settings(env::var("XIAOMI_PROTOCOL").ok().as_deref());
    let region = XiaomiRegion::from_settings(env::var("XIAOMI_REGION").ok().as_deref());

    let provider = XiaomiProviderImpl {
        api_key,
        region,
        default_protocol: protocol,
        openai_base_url: env::var("XIAOMI_OPENAI_BASE_URL").ok(),
        anthropic_base_url: env::var("XIAOMI_ANTHROPIC_BASE_URL").ok(),
    };

    println!(
        "[probe] provider_name={}, region={:?}, default_protocol={:?}, model={}",
        provider.provider_name(),
        region,
        protocol,
        model
    );

    provider.validate_credentials()?;
    let client = provider.create_client(&model).await?;
    let variant = match &client {
        LlmClient::RigXiaomi(_) => "RigXiaomi (OpenAI-compatible)",
        LlmClient::RigXiaomiAnthropic(_) => "RigXiaomiAnthropic (Anthropic-compatible)",
        other => {
            println!(
                "[probe] unexpected client variant: {}",
                other.provider_name()
            );
            return Ok(());
        }
    };
    println!("[probe] LlmClient variant: {variant}");

    let max_tokens: u64 = env::var("XIAOMI_MAX_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400);
    let user_prompt =
        env::var("XIAOMI_PROMPT").unwrap_or_else(|_| "用一句话介绍你自己。".to_string());

    println!("[probe] max_tokens={max_tokens}  prompt={user_prompt:?}");

    let result = client
        .one_shot_completion(
            "你是一个乐于助人的 AI 助手，回答简短。",
            &user_prompt,
            Some(0.3),
            Some(max_tokens),
        )
        .await;

    match result {
        Ok(text) => {
            println!("[probe] ✅ upstream returned text:");
            println!("{text}");
        }
        Err(err) => {
            // Errors are still informative: the probe checks plumbing, not billing.
            let msg = format!("{err:?}");
            println!("[probe] upstream returned error:\n{msg}");
            if msg.contains("402") || msg.to_lowercase().contains("balance") {
                println!(
                    "[probe] note: 402 / insufficient balance still proves auth+routing work."
                );
            }
        }
    }

    Ok(())
}

//! Manual smoke test against a configured BYOK provider endpoint.
//! NOT run in CI — it needs a real key.
//!
//! Usage:
//!   SKARN_KEY=sk-... cargo run -p skarn-ai --example smoke <base_url>
//!
//! Exercises exactly what the spike window does: `list_models`, then a
//! streaming "say hi" chat, printing deltas as they arrive and the receipt
//! at the end.

use skarn_ai::{ChatMessage, ChatRequest, SkarnClient};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let key = std::env::var("SKARN_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| std::env::var("SELORA_KEY").ok()) // skarn-v1-migration-compat
        .expect("SKARN_KEY must be set");
    let base_url = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SKARN_BASE_URL").ok())
        .or_else(|| std::env::var("SELORA_BASE_URL").ok()) // skarn-v1-migration-compat
        .filter(|url| !url.is_empty())
        .expect("pass <base_url> argument or set SKARN_BASE_URL");

    let client = SkarnClient::new(&base_url, &key);

    // --- list_models ---
    println!("base_url: {base_url}");
    let models = match client.list_models().await {
        Ok(models) => {
            println!("connected ({} models):", models.len());
            for model in &models {
                println!("  - {} ({})", model.id, model.display_name);
            }
            models
        }
        Err(error) => {
            println!("connect FAILED: {error}");
            std::process::exit(1);
        }
    };

    // --- stream_chat: "say hi" ---
    // The smoke example takes the first model the provider listed; SKARN_MODEL overrides.
    let model_id = std::env::var("SKARN_MODEL")
        .ok()
        .filter(|m| !m.is_empty())
        .or_else(|| std::env::var("SELORA_MODEL").ok().filter(|m| !m.is_empty())) // skarn-v1-migration-compat
        .unwrap_or_else(|| {
            models
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_else(|| "default".to_owned())
        });

    let request =
        ChatRequest::new(model_id.clone(), vec![ChatMessage::user("say hi")]).with_stream(true);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let stream = tokio::spawn(async move { client.stream_chat(request, tx).await });

    let mut streamed = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            skarn_ai::ChatEvent::Delta(delta) => {
                streamed.push_str(&delta);
                print!("{delta}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            skarn_ai::ChatEvent::Done(receipt) => {
                println!();
                println!(
                    "--- receipt: model={}, tokens: {} in / {} out / cost: ${:.4} (estimated: {})",
                    receipt.model,
                    receipt.prompt_tokens,
                    receipt.completion_tokens,
                    receipt.cost_usd,
                    receipt.estimated
                );
            }
            skarn_ai::ChatEvent::Error(error) => {
                println!();
                println!("--- stream error: {error}");
            }
            // The smoke example never arms tools (plain chat): a tool
            // call fragment cannot arrive here.
            skarn_ai::ChatEvent::ToolCallDelta { .. } => {}
        }
    }

    match stream.await {
        Ok(Ok(_)) => {
            println!(
                "--- first 200 chars: {}",
                streamed.chars().take(200).collect::<String>()
            );
            println!("SMOKE TEST PASSED");
        }
        Ok(Err(error)) => {
            println!("stream FAILED: {error}");
            std::process::exit(1);
        }
        Err(join) => {
            println!("worker died: {join}");
            std::process::exit(1);
        }
    }
}

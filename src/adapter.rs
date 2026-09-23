use crate::types::{
    AntigravityEnvelope, CompletionChoice, CompletionRequest, CompletionResponse, GeminiContent,
    GeminiGenerationConfig, GeminiPart, GeminiRequest, GeminiSseEvent,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

pub struct Adapter;

impl Adapter {
    pub fn build_antigravity_envelope(
        req: &CompletionRequest,
        default_model: &str,
        project_id: String,
    ) -> (String, AntigravityEnvelope, String) {
        let model = req
            .model
            .as_deref()
            .unwrap_or(default_model)
            .to_string();

        let prompt = req
            .prompt
            .as_ref()
            .map(|p| p.as_str())
            .unwrap_or("");

        let prompt_prefix = Self::extract_prompt_prefix(prompt);

        debug!(
            "Formatting request for Antigravity: model={}, prompt_len={}, project={}",
            model, prompt.len(), project_id
        );

        let stop_sequences = req.stop.as_ref().map(|s| s.to_vec());
        let max_output_tokens = req.max_tokens.or(Some(64));
        let temperature = req.temperature.or(Some(0.1));

        let envelope = AntigravityEnvelope {
            project: project_id,
            model: model.clone(),
            request: GeminiRequest {
                contents: vec![GeminiContent {
                    role: Some("user".to_string()),
                    parts: vec![GeminiPart {
                        text: Some(prompt.to_string()),
                    }],
                }],
                generation_config: Some(GeminiGenerationConfig {
                    max_output_tokens,
                    temperature,
                    stop_sequences,
                }),
            },
        };

        (model, envelope, prompt_prefix)
    }

    pub fn build_openai_response(
        model: String,
        raw_text: String,
        prompt_prefix: &str,
        finish_reason: Option<String>,
    ) -> CompletionResponse {
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let cleaned_text = Self::clean_completion(&raw_text, prompt_prefix);

        let id = format!("cmpl-{}", uuid_simple());

        CompletionResponse {
            id,
            object: "text_completion",
            created,
            model,
            choices: vec![CompletionChoice {
                text: cleaned_text,
                index: 0,
                logprobs: None,
                finish_reason,
            }],
        }
    }

    pub fn extract_text_from_sse_event(event: &GeminiSseEvent) -> (String, Option<String>) {
        if let Some(resp) = &event.response {
            if let Some(candidates) = &resp.candidates {
                if let Some(candidate) = candidates.first() {
                    let finish_reason = candidate.finish_reason.clone();
                    let mut text = String::new();
                    if let Some(content) = &candidate.content {
                        for part in &content.parts {
                            if let Some(t) = &part.text {
                                text.push_str(t);
                            }
                        }
                    }
                    return (text, finish_reason);
                }
            }
        }
        (String::new(), None)
    }

    fn extract_prompt_prefix(prompt: &str) -> String {
        // Detect StarCoder / CodeLlama / DeepSeek FIM markers
        for (prefix_marker, suffix_marker) in [
            ("<fim_prefix>", "<fim_suffix>"),
            ("<|fim_prefix|>", "<|fim_suffix|>"),
            ("<PRE>", "<SUF>"),
            ("<｜fim begin｜>", "<｜fim hole｜>"),
        ] {
            if let Some(p_start) = prompt.find(prefix_marker) {
                let after_prefix = &prompt[p_start + prefix_marker.len()..];
                if let Some(s_start) = after_prefix.find(suffix_marker) {
                    return after_prefix[..s_start].to_string();
                }
            }
        }
        prompt.to_string()
    }

    pub fn clean_completion(generated: &str, prompt_prefix: &str) -> String {
        let mut text = generated.trim().to_string();

        // 1. Strip markdown fences if wrapped (```lang ... ```)
        if text.starts_with("```") {
            let mut lines: Vec<&str> = text.lines().collect();
            if !lines.is_empty() && lines[0].starts_with("```") {
                lines.remove(0);
            }
            if !lines.is_empty() && lines.last().map(|l| l.trim() == "```").unwrap_or(false) {
                lines.pop();
            }
            text = lines.join("\n");
        }

        // 2. Strip leading prefix echo if model repeated the code before cursor
        let prefix_trimmed = prompt_prefix.trim();
        if !prefix_trimmed.is_empty() && text.trim_start().starts_with(prefix_trimmed) {
            if let Some(idx) = text.find(prefix_trimmed) {
                text = text[idx + prefix_trimmed.len()..].to_string();
                if text.starts_with('\n') {
                    text.remove(0);
                }
            }
        }

        text
    }
}

fn uuid_simple() -> String {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

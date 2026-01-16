use crate::config::SpeechOutputCfg;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn speak_async(text: String, cfg: SpeechOutputCfg) {
    if !cfg.enabled || cfg.provider.to_lowercase() != "groq" { return; }
    std::thread::spawn(move || {
        if let Err(e) = speak_blocking(&text, &cfg) {
            eprintln!("TTS error: {}", e);
        }
    });
}

fn speak_blocking(text: &str, cfg: &SpeechOutputCfg) -> Result<(), String> {
    let api_key = std::env::var("GROQ_API_KEY").map_err(|_| "missing GROQ_API_KEY".to_string())?;
    let url = "https://api.groq.com/openai/v1/audio/speech"; // Groq OpenAI-compatible endpoint
    let primary_model = std::env::var("BTWD_TTS_MODEL")
        .unwrap_or_else(|_| "canopylabs/orpheus-v1-english".to_string());
    let response_format = cfg.format.to_lowercase();
    // OpenAI-style TTS uses `response_format` (not `format`).
    // Groq returns 400 with "unknown field `format`" otherwise.
    let fallback_models = std::env::var("BTWD_TTS_FALLBACK_MODELS")
        .ok()
        .and_then(|s| {
            let items: Vec<String> = s
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
            (!items.is_empty()).then_some(items)
        })
        .unwrap_or_else(|| {
            vec![
                "canopylabs/orpheus-v1-english".to_string(),
                "tts-1".to_string(),
                "tts-1-hd".to_string(),
            ]
        });

    let mut tried: Vec<String> = Vec::new();
    let mut candidates: Vec<String> = Vec::new();
    candidates.push(primary_model.clone());
    for m in fallback_models {
        if m != primary_model {
            candidates.push(m);
        }
    }

    let client = reqwest::blocking::Client::new();
    let mut last_err: Option<String> = None;

    for model in candidates {
        tried.push(model.clone());
        let mut req_body = serde_json::json!({
            "model": model,
            "voice": cfg.voice,
            "input": text,
            "response_format": response_format,
        });
        if cfg.rate > 0.0 {
            if let Some(obj) = req_body.as_object_mut() {
                obj.insert("speed".to_string(), serde_json::Value::from(cfg.rate));
            }
        }

        eprintln!(
            "tts: request (provider=groq model={} voice={} response_format={} speed={} input_len={})",
            req_body["model"].as_str().unwrap_or("?"),
            cfg.voice,
            response_format,
            cfg.rate,
            text.len()
        );

        let resp = client
            .post(url)
            .bearer_auth(&api_key)
            .json(&req_body)
            .send()
            .map_err(|e| format!("http error: {}", e))?;

        if resp.status().is_success() {
            let bytes = resp.bytes().map_err(|e| format!("read body: {}", e))?.to_vec();
            return play_bytes(&bytes, &response_format);
        }

        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        let preview: String = body.chars().take(400).collect();

        // Model not found / no access: try next candidate.
        if status.as_u16() == 404 || preview.contains("model_not_found") {
            last_err = Some(format!("tts model unavailable: status={} body_preview={}", status, preview));
            continue;
        }

        // Any other failure: stop early (likely bad request/unauthorized).
        return Err(format!("tts http status: {} body_preview={}", status, preview));
    }

    Err(format!(
        "tts failed for all models tried={:?}; last_error={:?}; hint=List models: curl -sS -H 'Authorization: Bearer $GROQ_API_KEY' https://api.groq.com/openai/v1/models | jq -r '.data[].id' | sort; then set BTWD_TTS_MODEL or BTWD_TTS_FALLBACK_MODELS",
        tried,
        last_err
    ))
}

fn play_bytes(bytes: &[u8], _format: &str) -> Result<(), String> {
    // Try pw-play, aplay, then ffplay
    if try_player("pw-play", &["-"], bytes).is_ok() { return Ok(()); }
    if try_player("aplay", &["-"], bytes).is_ok() { return Ok(()); }
    if try_player("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet", "-"], bytes).is_ok() { return Ok(()); }
    Err("no suitable audio player found (pw-play/aplay/ffplay)".into())
}

fn try_player(cmd: &str, args: &[&str], bytes: &[u8]) -> Result<(), String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes).map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("player {} exit: {}", cmd, status)) }
}

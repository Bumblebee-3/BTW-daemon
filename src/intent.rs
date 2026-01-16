use crate::error::{BtwError, Result};
use crate::llm::{LlmClient, LlmIntent};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct IntentConfig {
    #[serde(default = "default_deterministic_threshold")] 
    pub deterministic_threshold: f32,
    #[serde(default = "default_llm_fallback_threshold")] 
    pub llm_fallback_threshold: f32,
}
fn default_deterministic_threshold() -> f32 { 0.75 }
fn default_llm_fallback_threshold() -> f32 { 0.8 }

#[derive(Debug, Deserialize)]
pub struct IntentCommand {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub dangerous: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct IntentResult {
    pub intent_type: String,
    pub command_id: Option<String>,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub deterministic_score: Option<f32>,
    #[serde(default)]
    pub dangerous: bool,
    #[serde(default)]
    pub requires_confirmation: bool,
}

pub struct IntentRouter {
    pub cfg: IntentConfig,
    pub commands: Vec<IntentCommand>,
    pub llm: std::sync::Arc<dyn LlmClient>,
}

impl IntentRouter {
    pub fn from_file(commands_path: &PathBuf, cfg: IntentConfig, llm: std::sync::Arc<dyn LlmClient>) -> Result<Self> {
        let s = fs::read_to_string(commands_path).map_err(|e| BtwError::ReadError { path: commands_path.clone(), source: e })?;
        let cmds: Vec<IntentCommand> = serde_json::from_str(&s).map_err(|e| BtwError::ParseError { path: commands_path.clone(), kind: "json", message: e.to_string() })?;
        Ok(Self { cfg, commands: cmds, llm })
    }

    pub fn route(&self, text: &str) -> IntentResult {
        let norm = normalize(text);
        // Safety guard: a zero/negative threshold effectively disables intent gating.
        // Never allow that, even if config is mis-parsed.
        let det_threshold = if self.cfg.deterministic_threshold > 0.0 {
            self.cfg.deterministic_threshold
        } else {
            0.75
        };
        if is_obvious_question(&norm) {
            // Avoid running commands for informational questions.
            // Still allow deterministic routing for explicit action phrases.
            // (LLM fallback will still be allowed to classify if configured.)
        }
        // Deterministic matching
        let mut best: Option<(f32, &IntentCommand)> = None;
        for cmd in &self.commands {
            let score = self.score_command(&norm, cmd);
                best = match best {
                    Some((b, _)) if score > b => Some((score, cmd)),
                    None => Some((score, cmd)),
                    _ => best,
                };
        }
        if let Some((score, cmd)) = best {
            if score <= 0.0 {
                eprintln!(
                    "intent: no deterministic match (best was id={} score={:.3} < min=0.001)",
                    cmd.id,
                    score
                );
            } else {
                eprintln!(
                    "intent: best deterministic match id={} score={:.3} threshold={:.3}",
                    cmd.id,
                    score,
                    det_threshold
                );
            }
            // Never accept a zero-score match.
            if score > 0.0 && score >= det_threshold {
                // Special-case: for question-like inputs, require a stronger match.
                // This prevents accidental matches like "what is 2 plus 2" -> lock_screen
                // due to incidental token overlap.
                if is_obvious_question(&norm) {
                    let strict = (det_threshold + 0.20).min(0.95);
                    if score < strict {
                        eprintln!(
                            "intent: question-like input; rejecting deterministic match (score={:.3} < strict={:.3})",
                            score,
                            strict
                        );
                    } else {
                        return self.result_for(cmd, norm.as_str(), score);
                    }
                } else {
                    return self.result_for(cmd, norm.as_str(), score);
                }
            }
        }
        // LLM fallback (classification only)
        match self.llm_classify(text) {
            Ok(r) => r,
            Err(_) => IntentResult {
                intent_type: "unknown_intent".into(),
                command_id: None,
                parameters: serde_json::json!({}),
                deterministic_score: None,
                dangerous: false,
                requires_confirmation: false,
            },
        }
    }

    fn score_command(&self, norm_text: &str, cmd: &IntentCommand) -> f32 {
        // Extra safety: for sensitive commands (e.g., lock/logout), require at least
        // one explicit action keyword to even consider overlap/substrings.
        if is_sensitive_command_id(&cmd.id) {
            let keywords = ["lock", "logout", "log out", "sign out", "suspend", "shutdown", "shut down", "reboot", "restart"];
            let has_keyword = keywords.iter().any(|k| norm_text.contains(k));
            if !has_keyword {
                return 0.0;
            }
        }

        let mut score: f32 = 0.0;
        // If the input is very short, be conservative with overlap-based scoring.
        let input_tokens: Vec<&str> = norm_text.split_whitespace().collect();
        let is_short_input = input_tokens.len() <= 3;

        // exact match against examples
        for ex in &cmd.examples {
            let e = normalize(ex);
            if e == norm_text { return 1.0; }
            if !e.is_empty() && norm_text.contains(&e) { score = score.max(0.85); }
        }
        // substring match against description
        let desc = normalize(&cmd.description);
        if !desc.is_empty() && norm_text.contains(&desc) { score = score.max(0.8); }
        // token overlap (simple Jaccard-like)
        let tset: std::collections::HashSet<_> = norm_text.split_whitespace().collect();
        let mut candidates: Vec<String> = cmd.examples.clone();
        candidates.push(cmd.description.clone());
        let mut best_overlap: f32 = 0.0;
        for c in candidates {
            let cnorm = normalize(&c);
            let cset: std::collections::HashSet<_> = cnorm.split_whitespace().collect();
            let inter = tset.intersection(&cset).count() as f32;
            let union = tset.union(&cset).count() as f32;
            if union > 0.0 {
                best_overlap = best_overlap.max(inter / union);
            }
        }
        // Overlap alone is weak evidence. Cap its influence, and require a minimum
        // number of overlapping tokens to avoid accidental matches.
        if best_overlap > 0.0 {
            let mut max_inter: usize = 0;
            for c in &cmd.examples {
                let cnorm = normalize(c);
                let cset: std::collections::HashSet<_> = cnorm.split_whitespace().collect();
                max_inter = max_inter.max(tset.intersection(&cset).count());
            }
            let descset: std::collections::HashSet<_> = desc.split_whitespace().collect();
            max_inter = max_inter.max(tset.intersection(&descset).count());

            // Need at least 2 shared tokens unless the input is short.
            let min_inter = if is_short_input { 1 } else { 2 };
            if max_inter >= min_inter {
                score = score.max(0.55 * best_overlap);
            }
        }
        score
    }

    fn result_for(&self, cmd: &IntentCommand, text: &str, score: f32) -> IntentResult {
        let params = extract_parameters(cmd, text);
        let dangerous = cmd.dangerous;
        // Always require confirmation for dangerous commands.
        // Also require confirmation for high-impact "session/security" commands like lock.*
        // even if not marked dangerous in JSON.
        let requires_confirmation = dangerous || is_sensitive_command_id(&cmd.id);
        IntentResult {
            intent_type: if dangerous { "dangerous_command".into() } else { "command".into() },
            command_id: Some(cmd.id.clone()),
            parameters: params,
            deterministic_score: Some(score),
            dangerous,
            requires_confirmation,
        }
    }

    fn llm_classify(&self, text: &str) -> Result<IntentResult> {
        let llm_result: LlmIntent = self.llm.classify_intent(text, &self.commands)
            .map_err(|e| BtwError::ParseError { path: PathBuf::new(), kind: "llm", message: e })?;
        if let Some(id) = llm_result.command_id {
            if llm_result.confidence >= self.cfg.llm_fallback_threshold {
                let dangerous = self.commands.iter().find(|c| c.id == id).map(|c| c.dangerous).unwrap_or(false);
                    let requires_confirmation = dangerous || is_sensitive_command_id(&id);
                    return Ok(IntentResult {
                        intent_type: if dangerous { "dangerous_command".into() } else { "command".into() },
                        command_id: Some(id),
                        parameters: llm_result.parameters,
                        deterministic_score: None,
                        dangerous,
                        requires_confirmation,
                    });
            }
        }
        Ok(IntentResult {
            intent_type: "unknown_intent".into(),
            command_id: None,
            parameters: serde_json::json!({}),
            deterministic_score: None,
            dangerous: false,
            requires_confirmation: false,
        })
    }
}

fn is_obvious_question(norm_text: &str) -> bool {
    let t = norm_text.trim();
    if t.is_empty() { return false; }
    // Very small heuristic to stop accidental command execution.
    // This is intentionally conservative: it only triggers on clear question forms.
    let starters = ["what is", "whats", "what's", "who is", "why", "how", "when", "where", "tell me", "explain", "calculate", "solve", "how much", "how many"];
    if starters.iter().any(|s| t.starts_with(s)) { return true; }
    t.ends_with('?')
}

fn is_sensitive_command_id(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    // Conservative list: commands that change session/security state.
    id.contains("lock") || id.contains("logout") || id.contains("suspend") || id.contains("shutdown") || id.contains("reboot")
}

fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch.is_whitespace() { out.push(ch.to_ascii_lowercase()); }
    }
    out.trim().to_string()
}

fn extract_parameters(cmd: &IntentCommand, text: &str) -> serde_json::Value {
    // Minimal heuristic: extract first integer and map by common ids
    if let Some(num) = first_int(text) {
        if cmd.id.contains("brightness") || cmd.id.contains("volume") {
            return serde_json::json!({"value": num});
        }
        if cmd.id.contains("up") || cmd.id.contains("down") {
            return serde_json::json!({"delta": num});
        }
    }
    serde_json::json!({})
}

fn first_int(s: &str) -> Option<i64> {
    let mut buf = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() { buf.push(ch); } else if !buf.is_empty() { break; }
    }
    if buf.is_empty() { None } else { buf.parse::<i64>().ok() }
}


#[cfg(test)]
mod tests {
    use super::*;

    struct DummyLlm;
    impl crate::llm::LlmClient for DummyLlm {
        fn classify_intent(&self, _text: &str, _commands: &[crate::intent::IntentCommand]) -> std::result::Result<crate::llm::LlmIntent, String> {
            Ok(crate::llm::LlmIntent { command_id: None, confidence: 0.0, parameters: serde_json::json!({}) })
        }
        fn summarize_search(&self, _query: &str, _snippets: &[String]) -> std::result::Result<String, String> {
            Err("not implemented in tests".into())
        }
        fn tts(&self, _text: &str) -> std::result::Result<Vec<u8>, String> {
            Err("not implemented in tests".into())
        }
        fn answer_short(&self, prompt: &str) -> std::result::Result<String, String> {
            Ok(format!("dummy answer: {}", prompt))
        }
    }

    fn test_router() -> IntentRouter {
        let cfg = IntentConfig {
            deterministic_threshold: 0.6,
            llm_fallback_threshold: 0.9,
        };

        let commands = vec![
            IntentCommand {
                id: "brightness_set".into(),
                description: "Set screen brightness".into(),
                examples: vec![
                    "set brightness to 40 percent".into(),
                    "set screen brightness to 70".into(),
                ],
                dangerous: false,
            },
            IntentCommand {
                id: "volume_up".into(),
                description: "Increase system volume".into(),
                examples: vec![
                    "increase volume".into(),
                    "turn volume up".into(),
                ],
                dangerous: false,
            },
            IntentCommand {
                id: "system_reboot".into(),
                description: "Reboot the system".into(),
                examples: vec![
                    "restart my system".into(),
                    "reboot".into(),
                ],
                dangerous: true,
            },
        ];

        IntentRouter { cfg, commands, llm: std::sync::Arc::new(DummyLlm) }
    }

    #[test]
    fn test_intents() {
        let router = test_router();

        let cases = [
            ("set brightness to 40 percent", Some("brightness_set")),
            ("increase volume", Some("volume_up")),
            ("restart my system", Some("system_reboot")),
            ("what is the weather tomorrow", None),
        ];

        for (input, expected) in cases {
            let intent = router.route(input);
            assert_eq!(
                intent.command_id.as_deref(),
                expected,
                "input: {:?}, intent: {:?}",
                input,
                intent
            );
        }
    }
}

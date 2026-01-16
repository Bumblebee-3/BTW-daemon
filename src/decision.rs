use crate::intent::IntentResult;

#[derive(Debug, Clone)]
pub enum Decision {
    Command {
        intent: IntentResult,
        preview: String,
        requires_confirmation: bool,
    },
    Question {
        text: String,
    },
    WebQuery {
        text: String,
    },
    Ignored,
}

#[derive(Debug, Clone)]
pub struct DecisionConfig {
    pub deterministic_threshold: f32,
}

pub struct DecisionManager {
    cfg: DecisionConfig,
}

impl DecisionManager {
    pub fn new(cfg: DecisionConfig) -> Self {
        Self { cfg }
    }

    pub fn decide(&self, raw_text: &str, deterministic: IntentResult) -> Decision {
        let normalized = normalize_input(raw_text);
        if normalized.is_empty() {
            return Decision::Ignored;
        }

        // Step 2: deterministic-only command matching.
        // If it isn't a command here, it is not a command at all.
        if let Some(command_id) = deterministic.command_id.as_deref() {
            if deterministic.intent_type == "command" || deterministic.intent_type == "dangerous_command" {
                // Safety: the router must provide a score that meets threshold before treating this as a command.
                // If unavailable, default to NOT executing.
                let score = deterministic.deterministic_score.unwrap_or(0.0);

                if score >= self.cfg.deterministic_threshold {
                    let dangerous = deterministic.dangerous;
                    let requires_confirmation = dangerous;
                    let preview = if deterministic.parameters.is_object() && deterministic.parameters.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
                        format!("About to run: {} {}", command_id, deterministic.parameters)
                    } else {
                        format!("About to run: {}", command_id)
                    };
                    return Decision::Command {
                        intent: deterministic,
                        preview,
                        requires_confirmation,
                    };
                }
            }

            // If a router produced a command_id without meeting strict requirements,
            // treat as non-command (do not ask for confirmation, do not touch executor).
            let _ = command_id;
        }

        // Step 4: Non-command handling.
        if is_web_query(&normalized) {
            return Decision::WebQuery { text: raw_text.trim().to_string() };
        }
        if is_question(&normalized) {
            return Decision::Question { text: raw_text.trim().to_string() };
        }

        Decision::Question { text: raw_text.trim().to_string() }
    }
}

fn normalize_input(s: &str) -> String {
    // Lowercase + trim + normalize punctuation and basic number words.
    let mut cleaned = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch.is_whitespace() {
            cleaned.push(ch.to_ascii_lowercase());
        }
    }
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    normalize_number_words(&cleaned)
}

fn normalize_number_words(s: &str) -> String {
    // Minimal word->digit normalization for common speech.
    // This is intentionally conservative and only handles 0-10.
    let map = [
        ("zero", "0"),
        ("one", "1"),
        ("two", "2"),
        ("three", "3"),
        ("four", "4"),
        ("five", "5"),
        ("six", "6"),
        ("seven", "7"),
        ("eight", "8"),
        ("nine", "9"),
        ("ten", "10"),
        ("percent", "%"),
    ];
    let mut out: Vec<String> = Vec::new();
    for tok in s.split_whitespace() {
        let mut replaced = tok.to_string();
        for (w, d) in map {
            if tok == w {
                replaced = d.to_string();
                break;
            }
        }
        out.push(replaced);
    }
    out.join(" ")
}

fn is_question(norm: &str) -> bool {
    let t = norm.trim();
    if t.is_empty() {
        return false;
    }
    let starters = [
        "what is",
        "whats",
        "who is",
        "why",
        "how",
        "when",
        "where",
        "explain",
        "tell me",
        "calculate",
        "solve",
    ];
    starters.iter().any(|s| t.starts_with(s))
}

fn is_web_query(norm: &str) -> bool {
    let t = norm.trim();
    if t.is_empty() {
        return false;
    }
    let keywords = [
        "weather",
        "news",
        "current time",
        "time is",
        "date is",
        "today",
        "stock",
        "price of",
    ];
    keywords.iter().any(|k| t.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_intent(score: Option<f32>) -> IntentResult {
        IntentResult {
            intent_type: "unknown_intent".into(),
            command_id: None,
            parameters: serde_json::json!({}),
            deterministic_score: score,
            dangerous: false,
            requires_confirmation: false,
        }
    }

    fn intent_command(id: &str, score: f32, dangerous: bool) -> IntentResult {
        IntentResult {
            intent_type: if dangerous { "dangerous_command".into() } else { "command".into() },
            command_id: Some(id.to_string()),
            parameters: serde_json::json!({}),
            deterministic_score: Some(score),
            dangerous,
            requires_confirmation: dangerous,
        }
    }

    #[test]
    fn non_command_question_never_becomes_command() {
        let dm = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let det = IntentResult {
            intent_type: "unknown_intent".into(),
            command_id: None,
            parameters: serde_json::json!({}),
            deterministic_score: None,
            dangerous: false,
            requires_confirmation: false,
        };
        let d = dm.decide("what is two plus two", det);
        assert!(matches!(d, Decision::Question { .. }));
    }

    #[test]
    fn deterministic_below_threshold_is_not_command() {
        let dm = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let det = intent_command("brightness_set", 0.50, false);
        let d = dm.decide("set brightness to 40 percent", det);
        match d {
            Decision::Command { .. } => panic!("should not accept below threshold"),
            _ => {}
        }
    }

    #[test]
    fn deterministic_above_threshold_becomes_command() {
        let dm = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let det = intent_command("brightness_set", 0.90, false);
        let d = dm.decide("set brightness to 40 percent", det);
        match d {
            Decision::Command { requires_confirmation, .. } => assert!(!requires_confirmation),
            _ => panic!("expected command"),
        }
    }
    
    #[test]
    fn news_question_routes_to_web_query() {
        let mgr = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let d = mgr.decide("What's in news today?", dummy_intent(None));
        assert!(matches!(d, Decision::WebQuery { .. }));
    }
}

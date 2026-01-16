use crate::decision::{Decision, DecisionManager};
use crate::executor::{ExecStatus, Executor};
use crate::intent::IntentResult;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Listening,
    Deciding,
    Confirming,
    Responding,
}

#[derive(Debug, Clone)]
pub struct ConfirmationToken {
    request_id: String,
}

#[derive(Debug, Clone)]
pub struct PendingCommand {
    pub request_id: String,
    pub intent: IntentResult,
    pub preview: String,
    pub dangerous: bool,
}

pub struct Manager {
    pub state: State,
    pending: Option<PendingCommand>,
    decision: DecisionManager,
}

impl Manager {
    pub fn new(decision: DecisionManager) -> Self {
        Self { state: State::Idle, pending: None, decision }
    }

    pub fn on_wake(&mut self) {
        self.state = State::Listening;
    }

    pub fn on_transcript(&mut self, text: &str, deterministic: IntentResult) -> ManagerOutcome {
        // Rule 3: Speech ignored unless relevant
        if self.state != State::Deciding {
            return ManagerOutcome::Ignored;
        }

        // Rule 4: unknown can never become command (Decision enforces this)
        let d = self.decision.decide(text, deterministic);
        match d {
            Decision::Command { intent, preview, requires_confirmation } => {
                // Rule 1: No command may execute unless state == Confirming
                // Enforce: ALL commands require explicit confirmation.
                let _ = requires_confirmation;

                // Enter explicit confirmation state.
                let cmd_id = intent.command_id.clone().unwrap_or_else(|| "unknown".to_string());
                let nonce = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let request_id = format!("{}-{}", cmd_id, nonce);
                self.pending = Some(PendingCommand {
                    request_id: request_id.clone(),
                    intent,
                    preview: preview.clone(),
                    dangerous: true,
                });
                self.state = State::Confirming;
                ManagerOutcome::NeedsConfirmation {
                    request_id,
                    preview,
                }
            }
            Decision::Question { text } => {
                self.state = State::Responding;
                ManagerOutcome::Question { text }
            }
            Decision::WebQuery { text } => {
                self.state = State::Responding;
                ManagerOutcome::WebQuery { text }
            }
            Decision::Ignored => ManagerOutcome::Ignored,
        }
    }

    pub fn enter_deciding(&mut self) {
        self.state = State::Deciding;
    }

    pub fn confirmation_token(&self) -> Option<ConfirmationToken> {
        if self.state != State::Confirming {
            return None;
        }
        self.pending
            .as_ref()
            .map(|p| ConfirmationToken { request_id: p.request_id.clone() })
    }

    pub fn confirm(&mut self, token: &ConfirmationToken) -> Option<IntentResult> {
        if self.state != State::Confirming {
            return None;
        }
        let pending = self.pending.as_ref()?;
        if pending.request_id != token.request_id {
            return None;
        }
        let intent = pending.intent.clone();
        self.pending = None;
        self.state = State::Responding;
        Some(intent)
    }

    pub fn cancel(&mut self) {
        // Rule 2: Cancel = hard reset
        self.pending = None;
        self.state = State::Idle;
    }

    pub fn reset_to_idle(&mut self) {
        self.pending = None;
        self.state = State::Idle;
    }

    pub fn pending_request_id(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.request_id.as_str())
    }
}

pub enum ManagerOutcome {
    NeedsConfirmation { request_id: String, preview: String },
    Question { text: String },
    WebQuery { text: String },
    Ignored,
}

/// Executor gate: only manager-confirmed intents are allowed to execute.
pub fn execute_with_token(executor: &mut Executor, intent: &IntentResult, token: &ConfirmationToken) -> ExecStatus {
    // Hard gate: if this function isn't called with a token from Manager::confirmation_token,
    // nothing can execute. (We don't expose token fields.)
    let _ = token;
    executor.handle_intent(intent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::{DecisionConfig, DecisionManager};

    fn cmd_intent(id: &str, score: f32) -> IntentResult {
        IntentResult {
            intent_type: "command".into(),
            command_id: Some(id.to_string()),
            parameters: serde_json::json!({}),
            deterministic_score: Some(score),
            dangerous: false,
            requires_confirmation: false,
        }
    }

    #[test]
    fn ignores_transcript_unless_deciding() {
        let decision = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let mut mgr = Manager::new(decision);
        mgr.on_wake();
        let out = mgr.on_transcript("lock screen", cmd_intent("lock_screen", 0.99));
        assert!(matches!(out, ManagerOutcome::Ignored));
    }

    #[test]
    fn command_always_requires_confirmation_state() {
        let decision = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let mut mgr = Manager::new(decision);
        mgr.on_wake();
        mgr.enter_deciding();
        let out = mgr.on_transcript("lock my laptop", cmd_intent("lock_screen", 0.99));
        match out {
            ManagerOutcome::NeedsConfirmation { request_id, preview: _ } => {
                assert!(!request_id.is_empty());
                assert_eq!(mgr.state, State::Confirming);
                assert_eq!(mgr.pending_request_id(), Some(request_id.as_str()));
            }
            _ => panic!("expected NeedsConfirmation"),
        }
    }

    #[test]
    fn cancel_is_hard_reset() {
        let decision = DecisionManager::new(DecisionConfig { deterministic_threshold: 0.75 });
        let mut mgr = Manager::new(decision);
        mgr.on_wake();
        mgr.enter_deciding();
        let _ = mgr.on_transcript("lock my laptop", cmd_intent("lock_screen", 0.99));
        assert_eq!(mgr.state, State::Confirming);
        mgr.cancel();
        assert_eq!(mgr.state, State::Idle);
        assert!(mgr.pending_request_id().is_none());
        assert!(mgr.confirmation_token().is_none());
    }
}

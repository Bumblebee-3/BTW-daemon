use crate::error::{BtwError, Result};
use crate::intent::IntentResult;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize, Clone)]
pub struct ExecCommand {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub dangerous: bool,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    pub shell_command_template: String,
}

#[derive(Debug, Clone)]
pub struct ExecutionCfg {
    pub confirmation_timeout_seconds: u64,
    pub dry_run: bool,
}

#[derive(Debug)]
pub enum ExecStatus {
    Executed { id: String },
    PendingConfirmation { id: String, description: String, deadline: Instant },
    Canceled { id: String, reason: String },
    Rejected { reason: String },
    Ignored,
}

struct Pending {
    program: String,
    args: Vec<String>,
    id: String,
    description: String,
    deadline: Instant,
    request_id: String,
}

pub struct Executor {
    by_id: HashMap<String, ExecCommand>,
    cfg: ExecutionCfg,
    pending: Option<Pending>,
}

impl Executor {
    pub fn new_from_path(path: &Path, cfg: ExecutionCfg) -> Result<Self> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| BtwError::ReadError { path: path.to_path_buf(), source: e })?;
        let cmds: Vec<ExecCommand> = serde_json::from_str(&s)
            .map_err(|e| BtwError::ParseError { path: path.to_path_buf(), kind: "json", message: e.to_string() })?;
        // Validate templates and index by id
        let mut by_id = HashMap::new();
        for c in cmds {
            if let Err(msg) = validate_template(&c.shell_command_template) {
                eprintln!("Skipping command '{}' due to unsafe template: {}", c.id, msg);
                continue;
            }
            by_id.insert(c.id.clone(), c);
        }
        Ok(Self { by_id, cfg, pending: None })
    }

    pub fn has_pending(&self) -> bool { self.pending.is_some() }

    pub fn pending_request_id(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.request_id.as_str())
    }

    pub fn confirm_pending(&mut self) -> ExecStatus {
        let pending = match self.pending.take() {
            Some(p) => p,
            None => return ExecStatus::Ignored,
        };
        match self.exec_program_args(&pending.id, &pending.program, &pending.args) {
            Ok(_) => ExecStatus::Executed { id: pending.id },
            Err(e) => ExecStatus::Rejected { reason: format!("execution failed: {}", e) },
        }
    }

    pub fn cancel_pending(&mut self, reason: &str) -> ExecStatus {
        let pending = match self.pending.take() {
            Some(p) => p,
            None => return ExecStatus::Ignored,
        };
        ExecStatus::Canceled { id: pending.id, reason: reason.to_string() }
    }

    pub fn handle_tick(&mut self, now: Instant) {
        if let Some(p) = &self.pending {
            if now >= p.deadline {
                eprintln!("Confirmation timed out for '{}', canceling", p.id);
                self.pending = None;
            }
        }
    }

    pub fn handle_confirmation_text(&mut self, text: &str) -> ExecStatus {
        // Safety: voice confirmations are disabled for execution.
        // Confirmation must come from the UI action path (confirm_pending/cancel_pending).
        let _ = text;
        ExecStatus::Ignored
    }

    pub fn handle_intent(&mut self, intent: &IntentResult) -> ExecStatus {
        if self.pending.is_some() {
            return ExecStatus::Rejected { reason: "confirmation pending; ignoring new commands".into() };
        }
        let id = match &intent.command_id { Some(s) => s.clone(), None => return ExecStatus::Ignored };

        // Strict mode: only allow deterministic decisions to reach execution.
        // If deterministic_score is missing, or below threshold, reject.
        let score = intent.deterministic_score.unwrap_or(0.0);
        if score <= 0.0 {
            return ExecStatus::Rejected { reason: "non-deterministic or low-confidence command blocked".into() };
        }

        let cmd = match self.by_id.get(&id) { Some(c) => c.clone(), None => return ExecStatus::Rejected { reason: format!("unknown command id '{}': not in allow-list", id) } };
        // Validate parameters against spec
        if let Err(msg) = validate_parameters(&cmd.parameters, &intent.parameters) {
            return ExecStatus::Rejected { reason: msg };
        }
        // Render template
        let rendered = match render_template(&cmd.shell_command_template, &intent.parameters, &cmd.parameters) {
            Ok(s) => s,
            Err(msg) => return ExecStatus::Rejected { reason: msg },
        };
        let tokens: Vec<String> = split_tokens(&rendered);
        if tokens.is_empty() {
            return ExecStatus::Rejected { reason: "empty command".into() };
        }
        if let Err(msg) = validate_tokens(&tokens) {
            return ExecStatus::Rejected { reason: msg };
        }
        let program = tokens[0].clone();
        let args = tokens[1..].to_vec();
        if cmd.dangerous || intent.requires_confirmation {
            let deadline = Instant::now() + Duration::from_secs(self.cfg.confirmation_timeout_seconds);
            eprintln!("Confirmation required: {}. Say 'yes' to confirm or 'no' to cancel.", cmd.description);
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let request_id = format!("{}-{}", id, nonce);
            self.pending = Some(Pending { program, args, id: id.clone(), description: cmd.description.clone(), deadline, request_id });
            return ExecStatus::PendingConfirmation { id, description: cmd.description, deadline };
        }
        match self.exec_program_args(&id, &program, &args) {
            Ok(_) => ExecStatus::Executed { id },
            Err(e) => ExecStatus::Rejected { reason: format!("execution failed: {}", e) },
        }
    }

    fn exec_program_args(&self, id: &str, program: &str, args: &[String]) -> Result<()> {
        if self.cfg.dry_run {
            eprintln!("[dry-run] Would execute command: {}", id);
            return Ok(());
        }
        eprintln!("exec: running id='{}' program='{}' args={:?}", id, program, args);
        let mut cmd = Command::new(program);
        for a in args { cmd.arg(a); }
        // Inherit minimal env by default; do not invoke shell
        let output = cmd
            .output()
            .map_err(|e| BtwError::ParseError { path: std::path::PathBuf::new(), kind: "exec", message: e.to_string() })?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("exec: non-zero exit for id='{}': status={}", id, output.status);
            if !stdout.trim().is_empty() {
                eprintln!("exec: stdout: {}", stdout.trim());
            }
            if !stderr.trim().is_empty() {
                eprintln!("exec: stderr: {}", stderr.trim());
            }
            return Err(BtwError::ParseError {
                path: std::path::PathBuf::new(),
                kind: "exec",
                message: format!("non-zero exit: {}", output.status),
            });
        }
        Ok(())
    }
}

fn validate_template(tpl: &str) -> std::result::Result<(), String> {
    // Block known unsafe shell constructs while allowing %, @, +, -
    let forbidden_substrings = ["|", "&", ";", ">", "<", "`", "$(", "${", "\\", "\"", "'"];
    if forbidden_substrings.iter().any(|s| tpl.contains(s)) {
        return Err("template contains unsafe shell constructs".into());
    }
    // Option A: reject environment variables entirely (e.g., $HOME). Use absolute paths instead.
    if tpl.contains('$') {
        return Err("template contains environment variable; use absolute path".into());
    }
    Ok(())
}

fn render_template(tpl: &str, params: &serde_json::Value, spec: &HashMap<String, String>) -> std::result::Result<String, String> {
    let mut out = String::with_capacity(tpl.len());
    let mut i = 0;
    while i < tpl.len() {
        let b = tpl.as_bytes()[i];
        if b == b'{' {
            // find closing }
            if let Some(j) = tpl[i+1..].find('}') { 
                let key = &tpl[i+1..i+1+j];
                if !spec.contains_key(key) {
                    return Err(format!("unknown placeholder '{{{}}}'", key));
                }
                let v = params.get(key).and_then(|v| v.as_i64()).ok_or_else(|| format!("missing or non-integer parameter '{}'", key))?;
                out.push_str(&v.to_string());
                i = i + 1 + j + 1;
                continue;
            } else {
                return Err("unterminated '{' in template".into());
            }
        }
        out.push(b as char);
        i += 1;
    }
    Ok(out)
}

fn split_tokens(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

fn validate_tokens(tokens: &[String]) -> std::result::Result<(), String> {
    if tokens.is_empty() { return Err("empty command".into()); }
    let program = &tokens[0];
    if program.is_empty() { return Err("invalid program name".into()); }
    if program.starts_with('-') { return Err("invalid program name".into()); }
    if tokens.iter().any(|t| t.is_empty()) { return Err("invalid empty token".into()); }
    Ok(())
}

fn validate_parameters(spec: &HashMap<String, String>, params: &serde_json::Value) -> std::result::Result<(), String> {
    for (k, v) in spec.iter() {
        let vtrim = v.trim();
        if !vtrim.starts_with("int") {
            return Err(format!("unsupported param spec for '{}': '{}'", k, v));
        }
        // default: any int if no range
        let (min, max) = if let Some(space) = vtrim.find(' ') {
            let range = &vtrim[space+1..];
            if let Some(dash) = range.find('-') {
                let (a,b) = (&range[..dash], &range[dash+1..]);
                let min = a.trim().parse::<i64>().map_err(|_| format!("invalid min for '{}': '{}'", k, a))?;
                let max = b.trim().parse::<i64>().map_err(|_| format!("invalid max for '{}': '{}'", k, b))?;
                (Some(min), Some(max))
            } else { (None, None) }
        } else { (None, None) };
        let val = params.get(k).and_then(|x| x.as_i64()).ok_or_else(|| format!("missing integer parameter '{}'", k))?;
        if let Some(lo) = min { if val < lo { return Err(format!("parameter '{}' below min {}", k, lo)); } }
        if let Some(hi) = max { if val > hi { return Err(format!("parameter '{}' above max {}", k, hi)); } }
    }
    Ok(())
}

fn normalize(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::error::{BtwError, Result};

#[derive(Serialize)]
struct AsrRequest {
    #[serde(rename = "type")]
    typ: &'static str,
    audio_format: &'static str,
    sample_rate: u32,
    samples: Vec<i16>,
}

#[derive(Deserialize)]
pub struct AsrResponse {
    #[serde(rename = "type")]
    pub typ: String,
    pub text: String,
    pub confidence: Option<f32>,
    pub error: Option<String>,
}

pub struct MLWorker {
    script_path: PathBuf,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    resp_rx: Option<Receiver<String>>, // lines read from worker stdout
}

impl MLWorker {
    fn read_timeout_secs() -> u64 {
        std::env::var("BTWD_ASR_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v >= 1)
            .unwrap_or(25)
    }

    fn read_timeout_retry_secs() -> u64 {
        std::env::var("BTWD_ASR_TIMEOUT_RETRY_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v >= 1)
            .unwrap_or(10)
    }

    fn python_cmd() -> String {
        // Prefer a repo-local venv if present, so systemd uses the same
        // Python deps as interactive development.
        if let Ok(cwd) = std::env::current_dir() {
            let cand = cwd.join(".venv/bin/python");
            if cand.is_file() {
                return cand.to_string_lossy().to_string();
            }
            let cand = cwd.join(".venv/bin/python3");
            if cand.is_file() {
                return cand.to_string_lossy().to_string();
            }
        }
        "python3".to_string()
    }
    pub fn new() -> Result<Self> {
        let script_path = Self::default_script_path()?;
        let mut worker = MLWorker {
            script_path,
            child: None,
            stdin: None,
            resp_rx: None,
        };
        worker.spawn()?;
        Ok(worker)
    }

    fn default_script_path() -> Result<PathBuf> {
        if let Ok(p) = std::env::var("BTWD_ML_PATH") {
            return Ok(PathBuf::from(p));
        }

        // Try relative to current working directory first (systemd may set WorkingDirectory)
        // This supports running from a checked-out repo without additional env vars.
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join("ml").join("btw_ml.py");
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        let exe = std::env::current_exe().map_err(|e| BtwError::ParseError { path: PathBuf::new(), kind: "ml", message: format!("current_exe error: {}", e) })?;
        let exe_dir = exe
            .parent()
            .ok_or_else(|| BtwError::ParseError { path: PathBuf::new(), kind: "ml", message: "could not get exe parent".into() })?;
        // Try alongside the binary (packaged deploy)
        let candidate1 = exe_dir.join("ml").join("btw_ml.py");
        if candidate1.exists() {
            return Ok(candidate1);
        }
        // Try project root (../.. from target/release)
        if let Some(project_root) = exe_dir.parent().and_then(|p| p.parent()) {
            let candidate2 = project_root.join("ml").join("btw_ml.py");
            if candidate2.exists() {
                return Ok(candidate2);
            }
        }
        Err(BtwError::MissingFile {
            path: PathBuf::from("ml").join("btw_ml.py"),
            kind: "ml_worker",
        })
    }

    fn spawn(&mut self) -> Result<()> {
        let python = Self::python_cmd();

        // Log which Python interpreter we spawn. This is critical under systemd,
        // where PATH/env can differ from interactive shells.
        println!("ML worker: spawning with python={}", python);

        let mut child = Command::new(python)
            .arg(&self.script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: format!("spawn ML worker failed: {}", e) })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "worker stdin missing".into() })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "worker stdout missing".into() })?;
        // Spawn a reader thread to forward lines to a channel
        let (tx, rx) = mpsc::sync_channel::<String>(100);
        std::thread::spawn(move || {
            let mut br = BufReader::new(stdout);
            loop {
                let mut buf = String::new();
                match br.read_line(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let _ = tx.send(buf);
                    }
                    Err(_) => break,
                }
            }
        });
        self.stdin = Some(stdin);
        self.resp_rx = Some(rx);
        self.child = Some(child);
        Ok(())
    }

    fn ensure_alive(&mut self) -> Result<()> {
        let need_respawn = if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(_status)) => true,
                Ok(None) => false,
                Err(_) => true,
            }
        } else {
            true
        };
        if need_respawn {
            self.spawn()?;
        }
        Ok(())
    }

    pub fn transcribe(&mut self, samples: Vec<i16>, sample_rate: u32) -> Result<AsrResponse> {
        self.ensure_alive()?;

        let started = Instant::now();
        eprintln!(
            "asr: request start (sample_rate={}, samples={}, approx_sec={:.2})",
            sample_rate,
            samples.len(),
            samples.len() as f64 / sample_rate as f64
        );

        let req = AsrRequest {
            typ: "asr",
            audio_format: "pcm_s16le",
            sample_rate,
            samples,
        };
        let line = serde_json::to_string(&req)
            .map_err(|e| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: format!("serialize ASR req failed: {}", e) })?;

        eprintln!("asr: sending request to worker (bytes={})", line.len());

        // Write request
        if let Some(stdin) = &mut self.stdin {
            stdin
                .write_all(line.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush())
                .map_err(|e| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: format!("write to worker failed: {}", e) })?;
        } else {
            return Err(BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "worker stdin unavailable".into() });
        }

        let timeout = Duration::from_secs(Self::read_timeout_secs());
        let timeout_retry = Duration::from_secs(Self::read_timeout_retry_secs());

        // Read response line with timeout
        let buf = if let Some(rx) = &self.resp_rx {
            match rx.recv_timeout(timeout) {
                Ok(line) => line,
                Err(_) => {
                    // Timeout or disconnected; respawn worker
                    eprintln!(
                        "asr: worker read timeout/disconnect after {}s; respawning",
                        timeout.as_secs()
                    );
                    self.spawn()?;
                    // Try once more after respawn
                    if let Some(rx2) = &self.resp_rx {
                        rx2.recv_timeout(timeout_retry)
                            .map_err(|_| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "ASR read timeout".into() })?
                    } else {
                        return Err(BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "ASR reader channel missing".into() });
                    }
                }
            }
        } else {
            return Err(BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: "ASR reader not initialized".into() });
        };

        let trimmed = buf.trim();
        let preview: String = trimmed.chars().take(240).collect();
        eprintln!(
            "asr: worker response received (elapsed_ms={}, preview={})",
            started.elapsed().as_millis(),
            preview
        );

        let resp: AsrResponse = serde_json::from_str(trimmed)
            .map_err(|e| BtwError::ParseError { path: self.script_path.clone(), kind: "ml", message: format!("parse ASR resp failed: {}", e) })?;

        eprintln!(
            "asr: parsed result (elapsed_ms={}, text_len={}, has_error={})",
            started.elapsed().as_millis(),
            resp.text.len(),
            resp.error.as_deref().unwrap_or("").is_empty() == false
        );
        Ok(resp)
    }
}

use serde::Deserialize;

/// Top-level configuration loaded from `config.toml`.
///
/// Minimal, typed, and non-invasive for foundational step.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Optional human-readable name for the daemon instance.
    pub name: Option<String>,
    /// Optional description for documentation purposes.
    pub description: Option<String>,
    /// Wake word configuration (required)
    pub wake_word: WakeWord,
    /// Optional speech recording parameters
    #[serde(default)]
    pub speech: Speech,
    /// Intent routing configuration
    #[serde(default)]
    pub intent: IntentCfg,
    /// Execution configuration (confirmation, dry-run)
    #[serde(default)]
    pub execution: ExecutionCfg,
    /// UI/display configuration
    #[serde(default)]
    pub ui: UiCfg,
    /// Speech output (TTS) configuration
    #[serde(default)]
    pub speech_output: SpeechOutputCfg,
    /// Search (SERP) configuration
    #[serde(default)]
    pub search: SearchCfg,
    /// LLM provider configuration
    #[serde(default)]
    pub llm: LlmCfg,
}

impl Config {
    /// Parse a TOML string into `Config`.
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str::<Config>(s).map_err(|e| e.to_string())
    }
}

/// Wake word configuration loaded from `config.toml`.
#[derive(Debug, Deserialize)]
pub struct WakeWord {
    /// Absolute path to the .ppn keyword file.
    pub ppn_path: String,
    /// Absolute path to `porcupine_params.pv` (required for Porcupine 4.0).
    pub model_path: String,
    /// Porcupine device string: "cpu", "cpu:N", "gpu", or "best".
    #[serde(default = "default_porcupine_device")]
    pub device: String,
    /// Detection sensitivity in [0.0, 1.0].
    pub sensitivity: f32,
}

fn default_porcupine_device() -> String { "cpu".into() }

/// Speech recording parameters for end-of-speech detection.
#[derive(Debug, Deserialize, Default)]
pub struct Speech {
    /// RMS threshold (0.0..1.0) below which audio is considered silence.
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold: f32,
    /// Continuous silence duration in milliseconds to stop recording.
    #[serde(default = "default_silence_duration_ms")]
    pub silence_duration_ms: u32,
    /// Hard safety cap for utterance length in seconds.
    #[serde(default = "default_max_utterance_seconds")]
    pub max_utterance_seconds: u32,

    /// WebRTC VAD aggressiveness mode (0..=3). Higher = more strict.
    /// Defaults to 2 (VeryAggressive) to preserve prior behavior.
    #[serde(default = "default_vad_mode")]
    pub vad_mode: i32,
}

fn default_silence_threshold() -> f32 { 0.01 }
fn default_silence_duration_ms() -> u32 { 700 }
fn default_max_utterance_seconds() -> u32 { 30 }
fn default_vad_mode() -> i32 { 2 }

/// Intent routing configuration thresholds
#[derive(Debug, Deserialize, Default)]
pub struct IntentCfg {
    #[serde(default = "default_deterministic_threshold")] 
    pub deterministic_threshold: f32,
    #[serde(default = "default_llm_fallback_threshold")] 
    pub llm_fallback_threshold: f32,
}

fn default_deterministic_threshold() -> f32 { 0.75 }
fn default_llm_fallback_threshold() -> f32 { 0.8 }

/// Execution configuration
#[derive(Debug, Deserialize)]
pub struct ExecutionCfg {
    #[serde(default = "default_confirmation_timeout_seconds")]
    pub confirmation_timeout_seconds: u64,
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for ExecutionCfg {
    fn default() -> Self { Self { confirmation_timeout_seconds: 10, dry_run: false } }
}

fn default_confirmation_timeout_seconds() -> u64 { 10 }

/// UI configuration
#[derive(Debug, Deserialize, Clone)]
pub struct UiCfg {
    #[serde(default = "default_listening_notification")] 
    pub listening_notification: bool,
    #[serde(default = "default_osd")] 
    pub osd: bool,
    #[serde(default = "default_osd_timeout_ms")] 
    pub osd_timeout_ms: u64,
}

impl Default for UiCfg {
    fn default() -> Self { Self { listening_notification: true, osd: true, osd_timeout_ms: 1500 } }
}

fn default_listening_notification() -> bool { true }
fn default_osd() -> bool { true }
fn default_osd_timeout_ms() -> u64 { 1500 }

/// Speech output (TTS) configuration
#[derive(Debug, Deserialize, Clone)]
pub struct SpeechOutputCfg {
    #[serde(default = "default_tts_enabled")] 
    pub enabled: bool,
    #[serde(default = "default_tts_provider")] 
    pub provider: String,
    #[serde(default = "default_tts_voice")] 
    pub voice: String,
    #[serde(default = "default_tts_format")] 
    pub format: String, // "wav" or "mp3"
    #[serde(default = "default_tts_rate")] 
    pub rate: f32,
}

impl Default for SpeechOutputCfg {
    fn default() -> Self { Self { enabled: true, provider: "groq".into(), voice: "default".into(), format: "wav".into(), rate: 1.0 } }
}

fn default_tts_enabled() -> bool { true }
fn default_tts_provider() -> String { "groq".into() }
fn default_tts_voice() -> String { "default".into() }
fn default_tts_format() -> String { "wav".into() }
fn default_tts_rate() -> f32 { 1.0 }

/// Search configuration
#[derive(Debug, Deserialize, Clone)]
pub struct SearchCfg {
    #[serde(default = "default_search_enabled")] 
    pub enabled: bool,
    #[serde(default = "default_search_timeout_ms")] 
    pub timeout_ms: u64,

    /// Optional Tavily "country" parameter (e.g. "india", "us").
    #[serde(default)]
    pub country: Option<String>,
}

impl Default for SearchCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: 4000,
            country: None,
        }
    }
}

fn default_search_enabled() -> bool { true }
fn default_search_timeout_ms() -> u64 { 4000 }

/// LLM provider configuration
#[derive(Debug, Deserialize, Clone)]
pub struct LlmCfg {
    #[serde(default = "default_llm_provider")] 
    pub provider: String, // "groq" | "mistral"
}

impl Default for LlmCfg {
    fn default() -> Self { Self { provider: "groq".into() } }
}

fn default_llm_provider() -> String { "groq".into() }

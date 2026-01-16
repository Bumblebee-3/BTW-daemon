# Bumblebee Trusts Wikipedia (daemon) — Voice Assistant for Arch Linux

**btwd** is a lightweight voice assistant for Linux desktops.
It runs as a **systemd user service**, listens for a wake word, understands speech,
executes safe system commands, and can answer factual questions — without ever
allowing an LLM to execute commands.
Just install, configure, and enable the service.

---

## Features

- Wake word detection (Porcupine)
- Robust end-of-speech detection (WebRTC VAD)
- Speech-to-text via **Groq Whisper**
- Deterministic intent routing (LLM used only for classification)
- **Strictly allow-listed command execution**
- Explicit confirmation for dangerous actions
- Optional:
  - On-screen notifications (OSD)
  - Spoken responses via **Groq Cloud TTS**
  - Read-only web answers (Tavily → Mistral summarized)

---

## Security Model

- Only commands listed in `commands.json` can run
- No shell execution (`sh -c` is never used)
- No pipes, redirects, globbing, or env expansion
- All parameters are validated (type + range)
- Dangerous commands require explicit confirmation
- LLM output is **never executed**

---

## Installation

### 1. Install runtime dependencies

Arch Linux:

```bash
sudo pacman -S python python-pip pipewire pipewire-pulse libnotify alsa-utils ffmpeg
```
2. Install Python ASR dependencies
```
pip install --user groq numpy
```

3. Install Porcupine SDK

Copy headers and library to:

```
~/.local/include/
~/.local/lib/

Porcupine 4.0 Setup (Important)

Porcupine 4.0 requires ALL of the following at runtime:

- `libpv_porcupine.so` (shared library)
- `porcupine_params.pv` (model parameters file)
- Your wake word file: `*.ppn`

Suggested locations:

- Library: `~/.local/lib/libpv_porcupine.so`
- Params: `~/.local/share/porcupine/porcupine_params.pv`
- Wake words: `~/.config/btw/wake_words/*.ppn`

Environment:

- `PICOVOICE_ACCESS_KEY` must be set in `~/.config/btw/.env`

⚠️ Why this matters

Porcupine 4.0's C API requires `model_path` and `device` arguments in `pv_porcupine_init`. If they are omitted or mismatched, the C SDK can crash (stack corruption / segfault). btwd validates these values at startup and fails fast with a readable error.

Example `config.toml` wake-word section:

```toml
[wake_word]
ppn_path = "/home/USER/.config/btw/wake_words/btw.ppn"
model_path = "/home/USER/.local/share/porcupine/porcupine_params.pv"
device = "cpu"     # optional, default "cpu"
sensitivity = 0.6
```
```

4. Install btwd binary

Copy btwd to:

```
~/.local/bin/btwd
```

Ensure it is executable.

Configuration

All configuration lives in:
```
~/.config/btw/
```

Required files:
```
config.toml
commands.json
.env
```

LLM Provider

- Default provider is Groq. You can optionally switch to Mistral.
- Configure in `config.toml`:

```
[llm]
provider = "groq"   # or "mistral"; defaults to "groq" when omitted
```

- Environment keys:
  - When `[llm].provider = "groq"`: set `GROQ_API_KEY` in `.env`
  - When `[llm].provider = "mistral"`: set `MISTRAL_API_KEY` in `.env`
  - Do not require both keys at the same time; only the active provider key is required
  - `PICOVOICE_ACCESS_KEY` (Porcupine) and optional `TAVILY_API_KEY` are independent

Example `config.toml` (excerpt)

```
[wake_word]
ppn_path = "/absolute/path/to/wake_word.ppn"
sensitivity = 0.6

[execution]
confirmation_timeout_seconds = 10
dry_run = false

[ui]
listening_notification = true
osd = true
osd_timeout_ms = 2000

[speech_output]
enabled = true
provider = "groq"     # TTS provider (Groq only)
voice = "alloy"
format = "wav"
rate = 1.0

[search]
enabled = true
timeout_ms = 3500
country = "india"  # optional; passed to Tavily (e.g. "india", "us")

# btwd uses Tavily as the ONLY search backend when search is enabled.

[llm]
provider = "groq"     # or "mistral" (default is "groq")
```

Example `.env`

```
PICOVOICE_ACCESS_KEY=pk_...
GROQ_API_KEY=gsk_...         # required when provider = "groq"
MISTRAL_API_KEY=mis_...      # required when provider = "mistral"
TAVILY_API_KEY=...           # optional, enables read-only web answers

# UI notifications for web answers include a trailing `:source: tavily/mistral` line.
# That source marker is NOT spoken by TTS.
```

See the example files in this repository.

systemd User Service

Enable btwd at login:
```
systemctl --user enable --now btw.service
```

View logs:
```
journalctl --user -u btw.service -f
```
Usage

Say the wake word

Speak a command or question

For dangerous actions, confirm with “yes” or cancel with “no”

Dry-Run Mode

To test safely:
```
[execution]
dry_run = true
```

Commands will be logged but not executed.

Troubleshooting

No wake word: check PICOVOICE_ACCESS_KEY

No audio: verify microphone permissions

No speech output: ensure pw-play, aplay, or ffplay exists

No web answers: verify TAVILY_API_KEY

Privacy

Audio is processed locally until ASR

Only transcribed text is sent to APIs

No data is stored permanently

No browsing automation or background scraping

License

MIT


---

## ✅ What you have now

- A **production-ready install flow**
- A **non-developer README**
- Clear security guarantees
- systemd-native behavior


```
cargo build --release
cp target/release/btwd ~/.local/bin/btwd
```
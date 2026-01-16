#!/home/bumblebee/btw-voiceassistant/btw2/btwd/.venv/bin/python
import sys
import os
import json
import io
import wave
from typing import Any, Dict

import numpy as np
from groq import Groq

# Initialize Groq client once; reads key from GROQ_API_KEY or default env config
_client = None

def get_client() -> Groq:
    global _client
    if _client is None:
        # The Groq SDK reads GROQ_API_KEY automatically via env if not provided
        # but we pass explicitly to be clear.
        api_key = os.environ.get("GROQ_API_KEY")
        if not api_key:
            print("GROQ_API_KEY is not set", file=sys.stderr)
            # Still initialize without explicit key; SDK will error on first call
        _client = Groq(api_key=api_key) if api_key else Groq()
    return _client


def pcm16_to_wav_bytes(samples: np.ndarray, sample_rate: int) -> bytes:
    """Encode mono int16 PCM to WAV (in-memory)."""
    # Ensure dtype and little-endian order
    pcm = samples.astype(np.int16)
    # Write to BytesIO using wave module
    buf = io.BytesIO()
    with wave.open(buf, 'wb') as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)  # 16-bit
        wf.setframerate(int(sample_rate))
        wf.writeframes(pcm.tobytes())
    return buf.getvalue()


def handle_asr(req: Dict[str, Any]) -> Dict[str, Any]:
    # Validate request
    if req.get("audio_format") != "pcm_s16le":
        raise ValueError("Unsupported audio_format; expected pcm_s16le")
    sr = int(req.get("sample_rate", 0))
    if sr != 16000:
        # Groq down-samples, but we standardize to 16k per protocol
        raise ValueError("Unsupported sample_rate; expected 16000")
    samples = req.get("samples")
    if not isinstance(samples, list):
        raise ValueError("samples must be a list of int16")
    # Convert to numpy int16
    np_samples = np.array(samples, dtype=np.int16)
    wav_bytes = pcm16_to_wav_bytes(np_samples, sr)

    client = get_client()
    try:
        # Use whisper-large-v3-turbo for lower latency
        # The SDK supports file-like or (filename, bytes)
        result = client.audio.transcriptions.create(
            file=("audio.wav", wav_bytes),
            model="whisper-large-v3-turbo",
            response_format="json"
        )
        text = getattr(result, 'text', None)
    except Exception as e:
        # Report error to stderr and return structured error.
        # This keeps the Rust side from hanging and provides debuggable context.
        print(f"ASR error: {type(e).__name__}: {e}", file=sys.stderr)
        return {
            "type": "asr_result",
            "text": "",
            "confidence": None,
            "error": f"groq_asr_failed: {type(e).__name__}: {e}",
        }

    return {
        "type": "asr_result",
        "text": text or "",
        "confidence": None,
        "error": None,
    }


def main() -> None:
    # Read line-delimited JSON from stdin; write line-delimited JSON to stdout
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError as e:
            print(f"Invalid JSON: {e}", file=sys.stderr)
            continue
        typ = req.get("type")
        if typ == "asr":
            try:
                resp = handle_asr(req)
            except Exception as e:
                print(f"ASR handler error: {type(e).__name__}: {e}", file=sys.stderr)
                resp = {
                    "type": "asr_result",
                    "text": "",
                    "confidence": None,
                    "error": f"asr_handler_error: {type(e).__name__}: {e}",
                }
        else:
            # Unknown request type; ignore
            print(f"Unknown request type: {typ}", file=sys.stderr)
            resp = {
                "type": "asr_result",
                "text": "",
                "confidence": None,
                "error": f"unknown_request_type: {typ}",
            }
        # Write response JSON on a single line
        sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
        sys.stdout.flush()

if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
    except Exception as e:
        print(f"Worker fatal error: {e}", file=sys.stderr)
        # Exit on fatal
        sys.exit(1)


"""
cargo build --release -q && install -Dm755 target/release/btwd ~/.local/bin/btwd && systemctl --user daemon-reload && systemctl --user restart btw.service && sleep 0.3 && systemctl --user status btw.service --no-pager -l
"""
use crate::error::{BtwError, Result};
use crate::porcupine::Porcupine;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{Receiver, sync_channel};

/// Start microphone capture in a dedicated thread and feed frames into Porcupine.
/// Logs "Wake word detected" on detection.
pub fn start_listening(porcupine: &Porcupine) -> Result<(std::thread::JoinHandle<()>, Receiver<Vec<i16>>)> {
    // Select default input device
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| BtwError::ParseError {
        path: std::path::PathBuf::new(),
        kind: "audio",
        message: "no default input device".into(),
    })?;

    let required_rate = porcupine.sample_rate();
    let frame_length = porcupine.frame_length();

    // Choose a supported mono config matching Porcupine sample rate
    let supported = device.supported_input_configs().map_err(|e| BtwError::ParseError {
        path: std::path::PathBuf::new(),
        kind: "audio",
        message: format!("query input configs failed: {}", e),
    })?;

    // Prefer i16; fall back to f32
    let mut selected: Option<(cpal::StreamConfig, bool)> = None;
    for cfg in supported {
        if cfg.channels() != 1 { continue; }
        let range = cfg.min_sample_rate()..=cfg.max_sample_rate();
        if !range.contains(&cpal::SampleRate(required_rate)) { continue; }
        let with_sr = cfg.with_sample_rate(cpal::SampleRate(required_rate));
        let is_i16 = cfg.sample_format() == cpal::SampleFormat::I16;
        let sc = with_sr.config();
        // Prefer i16; choose f32 only if i16 not yet selected
        match (&selected, is_i16) {
            (None, true) => { selected = Some((sc, true)); }
            (None, false) => { selected = Some((sc, false)); }
            (Some(_), true) => { selected = Some((sc, true)); break; }
            _ => {}
        }
    }

    let (config, is_i16) = selected.ok_or_else(|| BtwError::ParseError { path: std::path::PathBuf::new(), kind: "audio", message: format!("no mono input config at {} Hz", required_rate) })?;

    let (tx, rx) = sync_channel::<Vec<i16>>(8);
    let handle = std::thread::spawn(move || {

        // Fixed-size frame buffer and index to avoid unbounded push and improve safety
        let mut frame: Vec<i16> = vec![0i16; frame_length];
        let mut idx: usize = 0;
        let err_fn = |err| eprintln!("audio stream error: {}", err);

        if is_i16 {
            let stream = match device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    for &sample in data {
                        frame[idx] = sample;
                        idx += 1;
                        if idx == frame_length {
                            let _ = tx.send(frame.clone());
                            idx = 0;
                        }
                    }
                },
                err_fn,
                None,
            ) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("audio: build input stream failed: {}", err);
                    return;
                }
            };
            if let Err(err) = stream.play() { eprintln!("audio: start stream failed: {}", err); return; }
            loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
        } else {
            let stream = match device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    for &sample in data {
                        // Convert normalized f32 samples (-1.0..1.0) to signed 16-bit PCM
                        // as required by Porcupine. Values are clipped to avoid overflow.
                        let s = (sample * i16::MAX as f32).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                        frame[idx] = s;
                        idx += 1;
                        if idx == frame_length {
                            let _ = tx.send(frame.clone());
                            idx = 0;
                        }
                    }
                },
                err_fn,
                None,
            ) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("audio: build input stream failed: {}", err);
                    return;
                }
            };
            if let Err(err) = stream.play() { eprintln!("audio: start stream failed: {}", err); return; }
            loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
        }
    });

    Ok((handle, rx))
}

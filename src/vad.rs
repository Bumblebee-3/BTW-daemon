use crate::error::Result;

/// Simple wrapper over WebRTC VAD
pub struct Vad {
    inner: webrtc_vad::Vad,
    sample_rate: i32,
    window_ms: i32,
}

impl Vad {
    pub fn new(mode: i32) -> Result<Self> {
        let mut inner = webrtc_vad::Vad::new();
        // Map numeric mode (0..=3) to VadMode variants
        let vm = match mode {
            0 => webrtc_vad::VadMode::LowBitrate,
            1 => webrtc_vad::VadMode::Aggressive,
            2 => webrtc_vad::VadMode::VeryAggressive,
            3 => webrtc_vad::VadMode::VeryAggressive,
            _ => webrtc_vad::VadMode::VeryAggressive,
        };
        inner.set_mode(vm);
        Ok(Vad { inner, sample_rate: 16000, window_ms: 30 })
    }

    /// Determine speech presence for a 30ms (480 samples) frame at 16kHz mono.
    pub fn is_speech(&mut self, frame: &[i16]) -> bool {
        if frame.len() < 480 {
            return false;
        }
        let slice = &frame[..480];
        // Use crate's voice segment API which expects 30ms @ 16kHz
        self.inner.is_voice_segment(slice).unwrap_or(false)
    }
}

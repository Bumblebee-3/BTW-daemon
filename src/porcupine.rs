use crate::error::{BtwError, Result};
use crate::porcupine_sys as sys;
use std::ffi::{CString, CStr};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

/// Safe RAII wrapper around Porcupine C SDK
pub struct Porcupine {
    handle: *mut sys::pv_porcupine_t,

    // These MUST be kept alive for the lifetime of `handle`
    _access_key: CString,
    _model_path: CString,
    _device: CString,
    _ppn_path: CString,

    ppn_path: PathBuf,
    device: String,
}

impl Porcupine {
    /// Initialize Porcupine with a single keyword `.ppn` and sensitivity.
    /// Requires `PICOVOICE_ACCESS_KEY` in environment.
    pub fn new(
        model_path: &Path,
        device: &str,
        ppn_path: &Path,
        sensitivity: f32,
    ) -> Result<Self> {
        if !model_path.is_absolute() {
            return Err(BtwError::ParseError {
                path: model_path.to_path_buf(),
                kind: "porcupine",
                message: "wake_word.model_path must be absolute".into(),
            });
        }
        if !model_path.exists() {
            return Err(BtwError::MissingFile {
                path: model_path.to_path_buf(),
                kind: "porcupine_params.pv",
            });
        }
        if !ppn_path.is_absolute() {
            return Err(BtwError::ParseError {
                path: ppn_path.to_path_buf(),
                kind: "porcupine",
                message: "ppn_path must be absolute".into(),
            });
        }
        if !ppn_path.exists() {
            return Err(BtwError::MissingFile {
                path: ppn_path.to_path_buf(),
                kind: "wake_word.ppn",
            });
        }

        let access_key = std::env::var("PICOVOICE_ACCESS_KEY").map_err(|_| {
            BtwError::ParseError {
                path: ppn_path.to_path_buf(),
                kind: "porcupine",
                message: "missing PICOVOICE_ACCESS_KEY in environment".into(),
            }
        })?;

        // --- C string preparation (explicit error mapping) ---
        let access_key_c = CString::new(access_key).map_err(|e| BtwError::ParseError {
            path: ppn_path.to_path_buf(),
            kind: "porcupine",
            message: format!("access key contains NUL byte: {}", e),
        })?;

        let model_c = CString::new(model_path.to_string_lossy().as_bytes()).map_err(|e| {
            BtwError::ParseError {
                path: model_path.to_path_buf(),
                kind: "porcupine",
                message: format!("model path contains NUL byte: {}", e),
            }
        })?;

        let device_c = CString::new(device).map_err(|e| BtwError::ParseError {
            path: model_path.to_path_buf(),
            kind: "porcupine",
            message: format!("device string contains NUL byte: {}", e),
        })?;

        let ppn_c = CString::new(ppn_path.to_string_lossy().as_bytes()).map_err(|e| {
            BtwError::ParseError {
                path: ppn_path.to_path_buf(),
                kind: "porcupine",
                message: format!("ppn path contains NUL byte: {}", e),
            }
        })?;

        let keyword_paths = [ppn_c.as_ptr()];
        let sensitivities = [sensitivity];

        let mut handle: *mut sys::pv_porcupine_t = null_mut();

        let status = unsafe {
            sys::pv_porcupine_init(
                access_key_c.as_ptr(),
                model_c.as_ptr(),
                device_c.as_ptr(),
                1,
                keyword_paths.as_ptr(),
                sensitivities.as_ptr(),
                &mut handle,
            )
        };

        if status != sys::pv_status_t_PV_STATUS_SUCCESS || handle.is_null() {
            let messages = unsafe { get_error_stack_messages() };
            return Err(BtwError::PorcupineInitFailed {
                status: status as i32,
                messages,
            });
        }

        Ok(Self {
            handle,
            _access_key: access_key_c,
            _model_path: model_c,
            _device: device_c,
            _ppn_path: ppn_c,
            ppn_path: ppn_path.to_path_buf(),
            device: device.to_string(),
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn version() -> String {
        unsafe {
            let c = sys::pv_porcupine_version();
            if c.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(c).to_string_lossy().into_owned()
            }
        }
    }

    pub fn frame_length(&self) -> usize {
        unsafe { sys::pv_porcupine_frame_length() as usize }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { sys::pv_sample_rate() as u32 }
    }

    pub fn process(&mut self, pcm: &[i16]) -> Result<bool> {
        if pcm.len() != self.frame_length() {
            return Err(BtwError::ParseError {
                path: self.ppn_path.clone(),
                kind: "porcupine",
                message: format!(
                    "invalid frame length: expected {} got {}",
                    self.frame_length(),
                    pcm.len()
                ),
            });
        }

        let mut keyword_index: i32 = -1;
        let status =
            unsafe { sys::pv_porcupine_process(self.handle, pcm.as_ptr(), &mut keyword_index) };

        if status != sys::pv_status_t_PV_STATUS_SUCCESS {
            let msg = unsafe {
                let c = sys::pv_status_to_string(status);
                if c.is_null() {
                    format!("status={:?}", status)
                } else {
                    CStr::from_ptr(c).to_string_lossy().into_owned()
                }
            };
            return Err(BtwError::ParseError {
                path: self.ppn_path.clone(),
                kind: "porcupine",
                message: format!("process failed: {}", msg),
            });
        }

        Ok(keyword_index >= 0)
    }
}

impl Drop for Porcupine {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { sys::pv_porcupine_delete(self.handle) };
            self.handle = null_mut();
        }
    }
}

unsafe fn get_error_stack_messages() -> Vec<String> {
    let mut stack: *mut *mut c_char = null_mut();
    let mut depth: i32 = 0;

    let status = sys::pv_get_error_stack(&mut stack, &mut depth);
    if status != sys::pv_status_t_PV_STATUS_SUCCESS || stack.is_null() || depth <= 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(depth as usize);
    for i in 0..depth {
        let p = *stack.add(i as usize);
        if !p.is_null() {
            out.push(CStr::from_ptr(p).to_string_lossy().into_owned());
        }
    }

    sys::pv_free_error_stack(stack);
    out
}

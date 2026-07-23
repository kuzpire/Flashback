use serde::Serialize;

// Estado de la captura para la UI. En la Fase 1 solo sirve para verificar que el
// bucle WGC corre y medir su impacto: cuántos frames llegan y a qué resolución.
#[derive(Serialize, Clone, Default)]
pub struct CaptureStatus {
    pub running: bool,
    pub frames: u64,
    pub width: u32,
    pub height: u32,
    pub seconds: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct MonitorInfo {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    pub thumb: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct AudioInput {
    pub id: String,
    pub name: String,
}

#[cfg(target_os = "windows")]
pub use win::{
    list_audio_inputs, list_monitors, replay_active, save_replay, start, start_replay, status,
    stop, stop_replay,
};

#[cfg(not(target_os = "windows"))]
pub fn list_monitors() -> Vec<MonitorInfo> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn list_audio_inputs() -> Vec<AudioInput> {
    Vec::new()
}

#[cfg(not(target_os = "windows"))]
pub fn start(
    _monitor_id: String,
    _out_dir: String,
    _fps: u32,
    _quality: String,
    _resolution: u32,
    _bitrate: u32,
    _mic: bool,
    _mic_device: String,
    _encoder_pref: String,
) -> Result<(), String> {
    Err("La captura solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn status() -> CaptureStatus {
    CaptureStatus::default()
}

#[cfg(not(target_os = "windows"))]
pub fn start_replay(
    _monitor_id: String,
    _out_dir: String,
    _seconds: u32,
    _fps: u32,
    _quality: String,
    _resolution: u32,
    _bitrate: u32,
    _mic: bool,
    _mic_device: String,
    _encoder_pref: String,
) -> Result<(), String> {
    Err("El replay solo está disponible en Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn stop_replay() {}

#[cfg(not(target_os = "windows"))]
pub fn save_replay(_source: &str) -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn replay_active() -> bool {
    false
}

#[cfg(target_os = "windows")]
mod win;

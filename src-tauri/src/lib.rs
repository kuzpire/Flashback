mod artwork;
mod capture;
mod detect;

#[tauri::command]
async fn game_hero(
    app: tauri::AppHandle,
    name: String,
    steam_appid: Option<u32>,
) -> Option<String> {
    artwork::game_hero(&app, &name, steam_appid).await
}

#[tauri::command]
async fn detect_game(app: tauri::AppHandle) -> Option<detect::DetectedGame> {
    detect::detect_game(&app).await
}

#[tauri::command]
fn list_monitors() -> Vec<capture::MonitorInfo> {
    capture::list_monitors()
}

#[tauri::command]
fn list_audio_inputs() -> Vec<capture::AudioInput> {
    capture::list_audio_inputs()
}

#[tauri::command]
fn start_capture(monitor_id: String) -> Result<(), String> {
    capture::start(monitor_id)
}

#[tauri::command]
fn stop_capture() {
    capture::stop()
}

#[tauri::command]
fn capture_status() -> capture::CaptureStatus {
    capture::status()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_global_shortcut::Builder::new().build())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            game_hero,
            detect_game,
            list_monitors,
            list_audio_inputs,
            start_capture,
            stop_capture,
            capture_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

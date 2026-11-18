mod xminus;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(xminus::AppState::new())
        .invoke_handler(tauri::generate_handler![
            xminus::initialize_session,
            xminus::start_separation,
            xminus::check_status,
            xminus::download_stems,
            xminus::restore_job,
            xminus::reveal_file,
        ])
        .run(tauri::generate_context!())
        .expect("No se pudo iniciar X-Minus Stems");
}

mod about;
mod autostart;
mod fonts;
mod logbuf;
mod lyrics;
mod player;
mod settings_io;
mod tray;
mod window;

use tauri::{Manager, WindowEvent};

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            window::set_locked,
            window::toggle_lock_cmd,
            window::apply_board,
            window::set_always_on_top,
            window::open_settings,
            window::close_settings,
            window::quit_app,
            window::set_dragging,
            window::window_logical_pos,
            window::move_window_logical,
            window::peek_board,
            window::reset_board_position,
            window::set_board_parked,
            about::app_version,
            about::open_url,
            about::check_update,
            logbuf::export_log,
            logbuf::log_line,
            autostart::get_autostart,
            autostart::set_autostart,
            lyrics::fetch_lyric,
            player::now_playing,
            player::now_cover,
            fonts::list_fonts,
            settings_io::settings_path_cmd,
            settings_io::settings_load,
            settings_io::settings_save,
            settings_io::settings_export,
            settings_io::settings_import_file,
        ])
        .setup(|app| {
            logbuf::info(&format!(
                "启动 LyricsFloat {}，exe={}",
                about::APP_VERSION,
                std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "未知".into())
            ));
            logbuf::info(&format!("设置文件：{}", settings_io::settings_path_cmd()));
            window::setup(app.handle())?;
            tray::setup(app.handle())?;
            player::spawn(app.handle().clone());
            // 便携版被挪过位置时，自启登记项里的路径会失效 —— 后台校正一次
            std::thread::spawn(autostart::sync_path);
            Ok(())
        })
        .on_window_event(|win, event| {
            // 设置窗口点关闭 = 藏起来，不退出进程
            if let WindowEvent::CloseRequested { api, .. } = event {
                if win.label() == "settings" {
                    api.prevent_close();
                    window::close_settings(win.app_handle().clone());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running LyricsFloat");
}

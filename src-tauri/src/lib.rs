mod about;
mod autostart;
mod fonts;
mod logbuf;
mod lyrics;
mod player;
mod settings_io;
mod startup;
mod tray;
mod window;

use tauri::{Manager, WindowEvent};

pub fn run() {
    // 两道闸都在 tauri 起来之前：没过闸就直接返回，不写日志、不碰配置文件。
    // 顺序不能反 —— 单实例要先抢，否则第二个进程会跟第一个抢 session.log 和 running.mark。
    if !startup::acquire_single_instance() {
        startup::show_already_running();
        return;
    }
    if !startup::check_version_rollback() {
        return;
    }

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
            lyrics::last_candidates,
            lyrics::clear_lyric_cache,
            player::now_playing,
            player::now_cover,
            player::list_players,
            player::set_player_pref,
            player::set_player_pin,
            fonts::list_fonts,
            settings_io::settings_path_cmd,
            settings_io::settings_load,
            settings_io::settings_save,
            settings_io::settings_export,
            settings_io::settings_import_file,
        ])
        .setup(|app| {
            // 先判断上次有没有正常退出，再开始写这一轮的日志
            logbuf::begin_session();
            install_panic_hook();
            logbuf::info(&format!(
                "启动 LyricsFloat {}，exe={}",
                about::APP_VERSION,
                std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "未知".into())
            ));
            logbuf::info(&format!("设置文件：{}", settings_io::settings_path_cmd()));
            logbuf::info(&format!(
                "系统：{} / {} 位",
                std::env::var("OS").unwrap_or_else(|_| "Windows".into()),
                std::mem::size_of::<usize>() * 8
            ));
            // 上次崩过就问一句要不要导出；线程里弹窗，别卡启动
            logbuf::prompt_crash_if_any();
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
        .build(tauri::generate_context!())
        .expect("error while building LyricsFloat")
        .run(|_app, event| {
            // 正常退出：删运行标记，下次启动才不会误判成崩溃
            if let tauri::RunEvent::Exit = event {
                logbuf::info("正常退出");
                logbuf::end_session();
            }
        });
}

/// 崩溃时至少把最后几句话留在磁盘上：panic 默认只打到控制台，release 里看不到。
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "未知位置".into());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "未知 panic".into()
        };
        logbuf::error(&format!("崩溃：{msg} @ {loc}"));
        default(info);
    }));
}

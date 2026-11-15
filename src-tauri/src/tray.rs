use std::sync::atomic::Ordering;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::window::{self, LOCKED};

/// 托盘菜单项句柄：要能随时改文案（锁定/解锁），所以存进 app state
struct ToggleItem(MenuItem<tauri::Wry>);

/// 直接用编译进二进制的正式图标（32×32 PNG，和任务栏/开始菜单同一个）
fn tray_icon() -> Image<'static> {
    Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("tray icon png is invalid")
        .to_owned()
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle_lock", "锁定词板", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "打开设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(tray_icon())
        .icon_as_template(false)
        .tooltip("歌词板")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle_lock" => window::toggle_lock(app),
            "settings" => window::open_settings(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::toggle_lock(tray.app_handle());
                sync_label(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(ToggleItem(toggle));
    sync_label(app);
    Ok(())
}

/// 菜单文案跟着锁定状态变，否则永远显示"锁定词板"很迷惑
pub fn sync_label(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let locked = LOCKED.load(Ordering::Relaxed);

    if let Some(state) = app.try_state::<ToggleItem>() {
        let _ = state.0.set_text(if locked { "解锁词板" } else { "锁定词板" });
    }

    let _ = tray.set_tooltip(Some(if locked {
        "歌词板 · 已锁定"
    } else {
        "歌词板 · 悬停显示"
    }));
}

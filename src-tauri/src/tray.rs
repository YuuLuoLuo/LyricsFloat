use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::window::{self, LOCKED};

/// 托盘菜单项句柄：要能随时改文案（锁定/解锁），所以存进 app state
struct ToggleItem(MenuItem<tauri::Wry>);

/// 单击 / 双击的区分。
///
/// Windows 的双击序列是 Down, Up, Down(=DoubleClick), Up：
/// 第一次抬起后先等一小段，期间来 DoubleClick 就作废这次单击、改开设置。
/// `CLICK_GEN` 是单击任务的世代号，双击时 +1 让在等的单击自杀；
/// `SUPPRESS_UNTIL` 拦掉双击里第二次 Up 触发的单击。
static CLICK_GEN: AtomicU64 = AtomicU64::new(0);
static SUPPRESS_UNTIL: AtomicU64 = AtomicU64::new(0);
/// 比系统双击间隔（默认 500ms）短，保证单击手感不肉
const CLICK_WAIT_MS: u64 = 220;

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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
        .tooltip("LyricsFloat")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle_lock" => window::toggle_lock(app),
            "settings" => window::open_settings(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            match event {
                // 单击：等一小段确认不是双击后，切换锁定
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => {
                    if now_ms() < SUPPRESS_UNTIL.load(Ordering::Relaxed) {
                        return;
                    }
                    let gen = CLICK_GEN.fetch_add(1, Ordering::Relaxed) + 1;
                    let app = tray.app_handle().clone();
                    thread::spawn(move || {
                        thread::sleep(Duration::from_millis(CLICK_WAIT_MS));
                        if CLICK_GEN.load(Ordering::Relaxed) == gen {
                            window::toggle_lock(&app);
                        }
                    });
                }
                // 双击：作废排队中的单击，直接开设置
                TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                } => {
                    SUPPRESS_UNTIL.store(now_ms() + CLICK_WAIT_MS * 2, Ordering::Relaxed);
                    CLICK_GEN.fetch_add(1, Ordering::Relaxed);
                    window::open_settings(tray.app_handle().clone());
                }
                _ => {}
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
        "LyricsFloat · 已锁定"
    } else {
        "LyricsFloat · 悬停显示"
    }));
}

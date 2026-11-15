pub mod effects;
mod hover;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// 锁定：暂停一切悬停响应，词板彻底消失
pub(crate) static LOCKED: AtomicBool = AtomicBool::new(false);
/// 当前是否显形（磨砂 + 可交互）
pub(crate) static REVEALED: AtomicBool = AtomicBool::new(false);
/// 玻璃底色，AABBGGRR（SCA 要的是 ABGR 顺序）
pub(crate) static GLASS_COLOR: AtomicU32 = AtomicU32::new(0x9E_1E1A1A);
/// 临时显形的截止时刻（epoch ms）。设置页拖宽高时反复刷新，
/// 悬停轮询看到还没过期就不收起 —— 调尺寸时能看清面板的边界。
pub(crate) static PEEK_UNTIL: AtomicU64 = AtomicU64::new(0);

pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn is_locked() -> bool {
    LOCKED.load(Ordering::Relaxed)
}

/// 按 hex + alpha 更新玻璃色。前端每次改设置都会调。
fn store_glass(hex: &str, alpha: f64) -> u32 {
    let h = hex.trim_start_matches('#');
    let full = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.to_string()
    };
    let n = u32::from_str_radix(&full, 16).unwrap_or(0x1a1a1e);
    let r = ((n >> 16) & 0xff) as u32;
    let g = ((n >> 8) & 0xff) as u32;
    let b = (n & 0xff) as u32;
    let a = ((alpha.clamp(0.0, 1.0) * 255.0).round() as u32) << 24;
    GLASS_COLOR.store(a | (b << 16) | (g << 8) | r, Ordering::Relaxed);
    a | (b << 16) | (g << 8) | r
}

fn board(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("board")
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let win = board(app).expect("board window not declared in tauri.conf.json");

    // 静止态：完全透明、鼠标穿透。
    // 圆角交给 DWM（固定 8px，等同上一版"圆角数值 0"时的观感）——
    // 亚克力是 DWM 画的，CSS 圆角裁不掉它，只有窗口级圆角能一起裁干净。
    effects::set_transparent_background(&win);
    effects::disable_glass(&win);
    // 圆角改由窗口区域裁（DWM 的窗口级圆角会附带一层投影，
    // 静止态全透明时那层阴影就浮在桌面上）
    effects::set_system_corner(&win, false);
    effects::strip_window_border(&win);
    effects::apply_round_region(&win);
    // 不要标题：tao 会给无边框窗口留着 CAPTION 样式位，
    // DWM 会照着窗口标题在左上角画出"歌词板"三个字
    let _ = win.set_title("");
    effects::set_click_through(&win, true);
    // tao 建窗口时把 WS_CAPTION 一起写进了 GWL_STYLE，这里先摘干净，
    // 之后由悬停轮询盯着（tao 每次改标志都会把它塞回来）
    effects::strip_caption(&win);

    if let Some(settings_win) = app.get_webview_window("settings") {
        effects::set_transparent_background(&settings_win);
        effects::set_system_corner(&settings_win, true);
        effects::strip_window_border(&settings_win);
        // 设置窗的标题栏是自绘的，同样不需要系统那份
        effects::strip_caption(&settings_win);
    }

    // 前端拿到位置后自己 show；这里兜底，避免前端异常时窗口永远不出现
    {
        let win = win.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            if !win.is_visible().unwrap_or(true) {
                let _ = win.show();
            }
        });
    }

    hover::spawn(app.clone(), win);
    Ok(())
}

fn reveal_on(win: &WebviewWindow) {
    effects::enable_glass(win, GLASS_COLOR.load(Ordering::Relaxed));
    effects::set_click_through(win, false);
    effects::strip_caption(win);
}

fn reveal_off(win: &WebviewWindow) {
    effects::disable_glass(win);
    effects::set_click_through(win, true);
    effects::strip_caption(win);
}

/// 托盘 / 快捷键共用：切换锁定
pub fn toggle_lock(app: &AppHandle) {
    set_lock_state(app, !is_locked());
}

pub fn set_lock_state(app: &AppHandle, next: bool) {
    LOCKED.store(next, Ordering::Relaxed);
    if next {
        REVEALED.store(false, Ordering::Relaxed);
        if let Some(win) = board(app) {
            reveal_off(&win);
        }
    }
    let _ = app.emit("locked-changed", next);
    crate::tray::sync_label(app);
}

#[tauri::command]
pub fn set_locked(app: AppHandle, locked: bool) {
    set_lock_state(&app, locked);
}

#[tauri::command]
pub fn toggle_lock_cmd(app: AppHandle) -> bool {
    let next = !is_locked();
    set_lock_state(&app, next);
    next
}

/// 尺寸 + 玻璃色，一次性下发。设置页拖滑块时高频调用。
#[tauri::command]
pub fn apply_board(app: AppHandle, width: f64, height: f64, tint: String, alpha: f64) {
    store_glass(&tint, alpha);
    let Some(win) = board(&app) else { return };
    resize_exact(&win, width, height);
    // 显形中用新颜色重刷一次，否则要等下次悬停
    if REVEALED.load(Ordering::Relaxed) {
        effects::enable_glass(&win, GLASS_COLOR.load(Ordering::Relaxed));
    }
}

/// 按逻辑尺寸设置窗口，并用真实缩放系数校准一次。
///
/// `set_size(LogicalSize)` 在部分 DPI 组合下换算出的物理像素会偏小，
/// 表现就是"设置里写 380 高、窗口只有 300 高" ——
/// 前排数按 380 算，实际只放得下 300，歌词自然就偏下被切。
fn resize_exact(win: &WebviewWindow, width: f64, height: f64) {
    let _ = win.set_size(tauri::LogicalSize::new(width, height));

    let Ok(scale) = win.scale_factor() else { return };
    let want_w = (width * scale).round();
    let want_h = (height * scale).round();
    if let Ok(inner) = win.inner_size() {
        let off = (inner.width as f64 - want_w).abs() > 2.0
            || (inner.height as f64 - want_h).abs() > 2.0;
        if off {
            let _ = win.set_size(tauri::PhysicalSize::new(want_w, want_h));
        }
    }
    // set_size 之后 tao 也会重写样式，顺手把标题栏摘掉
    effects::strip_caption(win);
    // 尺寸变了形状还是旧区域，必须按新尺寸重裁一次。
    // set_size 是异步生效的，立刻量还是旧尺寸，所以稍后补一次。
    effects::apply_round_region(win);
    let later = win.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(90));
        effects::apply_round_region(&later);
    });
}

/// 置顶只作用于词板；设置窗口永远是普通窗口
#[tauri::command]
pub fn set_always_on_top(app: AppHandle, on: bool) {
    if let Some(win) = board(&app) {
        let _ = win.set_always_on_top(on);
        effects::strip_caption(&win);
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        return;
    };
    // 刻意不动词板的显形状态：设置页开着不等于词板在悬停
    let _ = win.show();
    // show 会让 tao 重写一次窗口样式，标题栏样式要再摘一次
    effects::strip_caption(&win);
    let _ = win.set_focus();
}

#[tauri::command]
pub fn close_settings(app: AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// 拖动期间保持显形，避免拖动中玻璃闪退
#[tauri::command]
pub fn set_dragging(app: AppHandle, on: bool) {
    if on {
        if let Some(win) = board(&app) {
            REVEALED.store(true, Ordering::Relaxed);
            reveal_on(&win);
            let _ = app.emit("hover-changed", true);
        }
    }
}

/// 让词板临时显形 ms 毫秒（设置页调宽高时用，连续调用会把截止时间往后推）。
/// 锁定状态下不显形：锁了就该彻底消失。
#[tauri::command]
pub fn peek_board(app: AppHandle, ms: u64) {
    PEEK_UNTIL.store(now_ms() + ms.min(5000), Ordering::Relaxed);
    if is_locked() {
        return;
    }
    let Some(win) = board(&app) else { return };
    if !REVEALED.swap(true, Ordering::Relaxed) {
        reveal_on(&win);
        let _ = app.emit("hover-changed", true);
    }
}

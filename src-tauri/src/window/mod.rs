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
/// 「无播放时关闭词板」：词板整个窗口藏起来，连透明的那层都不参与。
/// 和 LOCKED 的区别是锁定时窗口还在（只是隐形），这个是彻底 hide()。
pub(crate) static PARKED: AtomicBool = AtomicBool::new(false);
/// 临时显形的截止时刻（epoch ms）。设置页拖宽高时反复刷新，
/// 悬停轮询看到还没过期就不收起 —— 调尺寸时能看清面板的边界。
pub(crate) static PEEK_UNTIL: AtomicU64 = AtomicU64::new(0);
/// 背景常显：锁定或鼠标离开时仍保留玻璃背景（只是点不到）。
/// 关着的时候静止态是"完全透明 + 穿透"，开着就变成"看得见但穿透"。
pub(crate) static GLASS_ALWAYS: AtomicBool = AtomicBool::new(false);

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
    // DWM 会照着窗口标题在左上角画出软件名的字
    let _ = win.set_title("");
    effects::set_click_through(&win, true);
    // tao 建窗口时把 WS_CAPTION 一起写进了 GWL_STYLE，这里先摘干净，
    // 之后由悬停轮询盯着（tao 每次改标志都会把它塞回来）
    effects::strip_caption(&win);
    // 终极保险：子类化 WM_NCCALCSIZE，整窗永远是客户区。
    // 光摘样式位不够 —— DWM 画出来的标题栏帧在样式干净后没人重算，
    // 会一直留到下次 resize。非客户区为 0 就永远没地方画。
    effects::subclass_frameless(&win);

    if let Some(settings_win) = app.get_webview_window("settings") {
        effects::set_transparent_background(&settings_win);
        effects::set_system_corner(&settings_win, true);
        effects::strip_window_border(&settings_win);
        // 设置窗的标题栏是自绘的，同样不需要系统那份
        effects::strip_caption(&settings_win);
        effects::subclass_frameless(&settings_win);
    }

    // 任务管理器 / 任务栏里，每个窗口的图标取的是窗口自己的 WM_SETICON；
    // 不设就显示 Windows 那个"没有图标"的通用方块，于是同组里图标五花八门。
    // 统一挂上 exe 里那份资源图标（default_window_icon 在 Windows 上就是它）。
    for label in ["board", "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            apply_window_icon(app, &w);
        }
    }

    // 前端拿到位置后自己 show；这里兜底，避免前端异常时窗口永远不出现
    {
        let win = win.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(2500));
            // 停放（无播放时关闭词板）本来就是"窗口不该在"，
            // 这里再 show 一次会把隐藏好的窗口拽回来，还是不带穿透样式的那种 ——
            // 表现就是词板突然出现、并且那块区域点不穿。
            if PARKED.load(Ordering::Relaxed) || LOCKED.load(Ordering::Relaxed) {
                return;
            }
            if !win.is_visible().unwrap_or(true) {
                let _ = win.show();
            }
        });
    }

    // 窗口几何与缩放是排查"位置不对/尺寸不对/边缘有缝"的第一手资料
    if let (Ok(pos), Ok(size), Ok(scale)) =
        (win.outer_position(), win.outer_size(), win.scale_factor())
    {
        crate::logbuf::info(&format!(
            "词板窗口：{}x{} @({}, {}) 缩放 {:.0}%",
            size.width,
            size.height,
            pos.x,
            pos.y,
            scale * 100.0
        ));
    }
    crate::logbuf::info("窗口就绪：board + settings");
    hover::spawn(app.clone(), win);
    Ok(())
}

/// 给窗口挂上 exe 资源里的图标，让任务管理器/任务栏里同一组窗口长得一样
fn apply_window_icon(app: &AppHandle, win: &WebviewWindow) {
    if let Some(icon) = app.default_window_icon().cloned() {
        let _ = win.set_icon(icon);
    }
}

fn reveal_on(win: &WebviewWindow) {
    crate::logbuf::info("悬停显形");
    effects::enable_glass(win, GLASS_COLOR.load(Ordering::Relaxed));
    effects::set_click_through(win, false);
    effects::strip_caption(win);
}

fn reveal_off(win: &WebviewWindow) {
    crate::logbuf::info("悬停收起");
    if GLASS_ALWAYS.load(Ordering::Relaxed) {
        // 背景常显：留着玻璃，只是让它点不到。
        // 关掉亚克力会让 CSS 背景失去系统模糊底衬，看起来像一块死色块。
        effects::enable_glass(win, GLASS_COLOR.load(Ordering::Relaxed));
        effects::set_passthrough(win, true, true);
    } else {
        effects::disable_glass(win);
        effects::set_click_through(win, true);
    }
    effects::strip_caption(win);
}

/// 背景常显开关。关掉时要立刻把窗口收回去，否则会留一块看不见的"幽灵"背景。
#[tauri::command]
pub fn set_glass_always(app: AppHandle, on: bool) {
    GLASS_ALWAYS.store(on, Ordering::Relaxed);
    crate::logbuf::info(if on { "背景常显：开" } else { "背景常显：关" });
    let Some(win) = board(&app) else { return };
    if PARKED.load(Ordering::Relaxed) {
        return;
    }
    if REVEALED.load(Ordering::Relaxed) {
        // 显形中：什么都不用改，悬停轮询会按新状态维持
        return;
    }
    reveal_off(&win);
}

/// 托盘 / 快捷键共用：切换锁定
pub fn toggle_lock(app: &AppHandle) {
    set_lock_state(app, !is_locked());
}

pub fn set_lock_state(app: &AppHandle, next: bool) {
    crate::logbuf::info(if next { "锁定词板" } else { "解锁词板" });
    LOCKED.store(next, Ordering::Relaxed);
    if next {
        REVEALED.store(false, Ordering::Relaxed);
        if let Some(win) = board(app) {
            reveal_off(&win);
        }
    } else if PARKED.load(Ordering::Relaxed) {
        // 开着「无播放时关闭词板」时解锁：窗口本来就该继续藏着。
        // 但 hide/show 会被 tao 顺手改样式，所以这里再按一次"藏 + 穿透"，
        // 免得解锁把词板连带渲染出来、还挡住下面那块区域的点击。
        if let Some(win) = board(app) {
            effects::disable_glass(&win);
            effects::set_click_through(&win, true);
            let _ = win.hide();
            crate::logbuf::info("解锁时词板处于停用状态，保持隐藏");
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

/// 只换玻璃颜色，不碰尺寸。
///
/// 「从封面取色」每换一首歌都要改颜色，走 `apply_board` 会连带 `resize_exact` ——
/// tao 的尺寸调用会把整份窗口样式重写一遍，换歌时闪一下就是那么来的。
#[tauri::command]
pub fn set_glass_color(app: AppHandle, tint: String, alpha: f64) {
    store_glass(&tint, alpha);
    let Some(win) = board(&app) else { return };
    if REVEALED.load(Ordering::Relaxed) || GLASS_ALWAYS.load(Ordering::Relaxed) {
        effects::enable_glass(&win, GLASS_COLOR.load(Ordering::Relaxed));
    }
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
    crate::logbuf::info(&format!("调整尺寸：{}x{}（逻辑）", width, height));
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
    crate::logbuf::info("打开设置");
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

/// 窗口当前位置（逻辑坐标），手动拖动的起点
#[tauri::command]
pub fn window_logical_pos(win: WebviewWindow) -> (f64, f64) {
    let scale = win.scale_factor().unwrap_or(1.0).max(0.01);
    match win.outer_position() {
        Ok(p) => (p.x as f64 / scale, p.y as f64 / scale),
        Err(_) => (0.0, 0.0),
    }
}

/// 手动拖动：按逻辑坐标移动窗口。
///
/// 刻意不用 tao 的 `drag_window` / `start_dragging` —— 它靠
/// `SendMessage(WM_NCLBUTTONDOWN, HTCAPTION)` 冒充"按住标题栏拖动"，
/// DWM 会因此画出一条标题栏帧。那条帧画出来之后样式位是干净的，
/// 轮询里的 strip_caption 没有变化可清、也不会触发框架重算，
/// 于是"点一下词板就冒出一条标题栏"，要等到下次 resize（调整设置）才消失。
/// 改成前端回传屏幕坐标、这里 SetWindowPos 跟着走，从源头不碰非客户区。
#[tauri::command]
pub fn move_window_logical(win: WebviewWindow, x: f64, y: f64) {
    let _ = win.set_position(tauri::LogicalPosition::new(x, y));
}

/// 让词板临时显形 ms 毫秒（设置页调宽高时用，连续调用会把截止时间往后推）。
/// 锁定状态下不显形：锁了就该彻底消失。
#[tauri::command]
pub fn peek_board(app: AppHandle, ms: u64) {
    PEEK_UNTIL.store(now_ms() + ms.min(5000), Ordering::Relaxed);
    if is_locked() || PARKED.load(Ordering::Relaxed) {
        return;
    }
    let Some(win) = board(&app) else { return };
    if !REVEALED.swap(true, Ordering::Relaxed) {
        reveal_on(&win);
        let _ = app.emit("hover-changed", true);
    }
}

/// 「无播放时关闭词板」的落地：true = 整个窗口藏起来（连隐形层都不占）。
/// 锁定时窗口本来就不可见，这里只是把 show/hide 对上去。
#[tauri::command]
pub fn set_board_parked(app: AppHandle, parked: bool) {
    PARKED.store(parked, Ordering::Relaxed);
    let Some(win) = board(&app) else { return };
    if parked {
        if REVEALED.swap(false, Ordering::Relaxed) {
            let _ = app.emit("hover-changed", false);
        }
        // 先关掉玻璃再藏，免得下次 show 时闪一帧亚克力
        effects::disable_glass(&win);
        // 藏之前也把穿透补齐：万一 show 被别的路径抢先调用，
        // 至少窗口不是"挡住点击"的状态
        effects::set_click_through(&win, true);
        let _ = win.hide();
        crate::logbuf::info("词板已隐藏（无播放/无歌词）");
    } else {
        let _ = win.show();
        // show() 会让 tao 整份重写 GWL_STYLE / GWL_EXSTYLE，
        // WS_EX_LAYERED | WS_EX_TRANSPARENT 会被抹掉 —— 这里立刻补回来，
        // 否则恢复显示的一瞬间窗口会挡住点击（锁定时还会看见它）
        effects::strip_caption(&win);
        effects::set_click_through(&win, !REVEALED.load(Ordering::Relaxed));
        if !REVEALED.load(Ordering::Relaxed) {
            effects::disable_glass(&win);
        }
        apply_window_icon(&app, &win);
        crate::logbuf::info("词板恢复显示");
    }
}

/// 把词板挪回主屏顶部居中 —— 拖到屏幕外面之后唯一的找回手段。
/// 顺手显形一下让人看见它落在哪（锁定状态下不打扰），
/// 返回新的物理坐标，前端拿去覆盖记住的位置。
#[tauri::command]
pub fn reset_board_position(app: AppHandle) -> Option<(f64, f64)> {
    let win = board(&app)?;
    let mon = win.primary_monitor().ok().flatten()?;
    let msize = mon.size();
    let mpos = mon.position();
    // 拿到的是物理像素，和 set_position 用的是同一套坐标
    let win_size = win.outer_size().ok()?;
    let gap = 16.0;
    let x = (mpos.x as f64 + (msize.width as f64 - win_size.width as f64) / 2.0)
        .max(mpos.x as f64 + gap);
    // 竖直方向放屏幕顶部 8% 处，和默认摆放一致
    let y = mpos.y as f64 + (msize.height as f64 * 0.08).round();

    crate::logbuf::info(&format!("词板位置重置 → ({:.0}, {:.0})", x, y));
    let pos = tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32);
    let _ = win.set_position(pos);
    // set_position 同样会让 tao 重写样式，标题栏再摘一次
    effects::strip_caption(&win);
    peek_board(app.clone(), 1600);
    Some((pos.x as f64, pos.y as f64))
}

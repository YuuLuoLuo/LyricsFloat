use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use super::{
    effects, now_ms, GLASS_ALWAYS, GLASS_COLOR, LOCKED, MEDIA_CLICKABLE, PARKED, PEEK_UNTIL,
    REVEALED,
};

/// 16ms ≈ 60Hz。鼠标穿透时 WebView 收不到 mousemove，只能读全局光标。
const TICK_MS: u64 = 16;
/// 离开后延迟收起，避免边缘抖动
const LEAVE_DELAY_MS: u64 = 320;
/// 进入判定外扩：更容易触发
const PAD_IN: i32 = 4;
/// 离开判定内收：防止刚滑到边上就消失
const PAD_OUT: i32 = 36;
/// 停放期间每隔这么久核对一次窗口是否还藏着（16ms * 60 ≈ 1s）
const PARKED_CHECK_TICKS: u64 = 60;

pub fn spawn(app: AppHandle, win: WebviewWindow) {
    thread::spawn(move || {
        let mut leave_ticks: u64 = 0;
        let mut parked_checks: u64 = 0;
        // 锁定态下「背景常显」已经应用过没有 —— 只在状态变化时动窗口样式
        let mut locked_always_done = false;

        loop {
            thread::sleep(Duration::from_millis(TICK_MS));

            // 「无播放时关闭词板」期间窗口是 hide 状态，什么都别碰：
            // 既不该显形，也省掉每拍的样式读写。
            // 但 hide/show 都可能被别的路径（tao 兜底 show、设置页调尺寸）搅乱，
            // 所以每秒核对一次"它真的还藏着、而且带穿透样式"，不对劲就纠正。
            if PARKED.load(Ordering::Relaxed) {
                leave_ticks = 0;
                parked_checks += 1;
                if parked_checks >= PARKED_CHECK_TICKS {
                    parked_checks = 0;
                    if win.is_visible().unwrap_or(false) {
                        effects::disable_glass(&win);
                        effects::set_click_through(&win, true);
                        let _ = win.hide();
                        let _ = app.emit("hover-changed", false);
                        crate::logbuf::warn("词板在停用状态被重新显示，已再次隐藏");
                    }
                }
                continue;
            }
            parked_checks = 0;

            // 兜底：只要有谁把 WS_CAPTION 塞回窗口样式（tao 在 set_size /
            // set_always_on_top / show 之后都会整份重写一次），就地摘掉。
            // 样式没变时这里只做两次 GetWindowLongW，开销可以忽略。
            effects::strip_caption(&win);

            let always = GLASS_ALWAYS.load(Ordering::Relaxed);
            let current = REVEALED.load(Ordering::Relaxed);
            let locked_now = LOCKED.load(Ordering::Relaxed);
            // 锁定 + 媒体控件可点：**只在光标真的落在控件上时**才取消穿透。
            //
            // 不能用 WM_NCHITTEST 返回 HTTRANSPARENT 来做局部放行 —— 那个码只对
            // 同一线程的窗口生效，跨进程（游戏、桌面）根本不往上传，结果就是
            // "控件能点，但整块词板都挡住了下面的点击"。
            // 改成"光标进控件 -> 窗口可点，离开 -> 立刻恢复穿透"，
            // 16ms 的切换延迟远小于人手移过去再按下的时间。
            let media_hot =
                locked_now && MEDIA_CLICKABLE.load(Ordering::Relaxed) && cursor_in_media(&win);
            // 同理，tao 的整份重写也会把 WS_EX_TRANSPARENT | WS_EX_LAYERED 抹掉。
            // 那两位是"静止态不显形 + 穿透点击"的全部实现 —— 启动即锁定时正好
            // 撞上前端 show()，词板就显了形还挡点击；而 REVEALED 从头到尾都是
            // false，等不到一次状态切换去补救。所以这里每拍对齐一次：
            // 与目标状态一致时是两次 GetWindowLongW，不一致才写。
            //
            // 注意：穿透只切命中测试，**不改窗口透明度**。窗口和歌词一直可见，
            // 「静止态」只是前端把玻璃背景的 opacity 归零。
            effects::set_click_through(&win, !current && !media_hot);

            let locked = locked_now;

            if locked {
                leave_ticks = 0;
                if always {
                    // 锁定 + 背景常显：窗口要看得见（背景在），但点不到、也没有悬停交互
                    if !locked_always_done {
                        locked_always_done = true;
                        effects::enable_glass(&win, GLASS_COLOR.load(Ordering::Relaxed));
                        effects::strip_caption(&win);
                    }
                    if current {
                        REVEALED.store(false, Ordering::Relaxed);
                        let _ = app.emit("hover-changed", false);
                    }
                } else {
                    locked_always_done = false;
                    if current {
                        REVEALED.store(false, Ordering::Relaxed);
                        reveal(&win, false);
                        let _ = app.emit("hover-changed", false);
                    }
                }
                continue;
            }
            locked_always_done = false;

            // 设置页刚调过宽高，或者正在调：这段时间内保持显形，
            // 让用户能看清面板的实际边界。过期后照常走悬停判断。
            if PEEK_UNTIL.load(Ordering::Relaxed) > now_ms() {
                leave_ticks = 0;
                if !current {
                    REVEALED.store(true, Ordering::Relaxed);
                    reveal(&win, true);
                    let _ = app.emit("hover-changed", true);
                }
                continue;
            }

            if cursor_over(&app, &win, current) {
                leave_ticks = 0;
                if !current {
                    REVEALED.store(true, Ordering::Relaxed);
                    reveal(&win, true);
                    let _ = app.emit("hover-changed", true);
                }
            } else if current {
                leave_ticks += 1;
                if leave_ticks * TICK_MS >= LEAVE_DELAY_MS {
                    leave_ticks = 0;
                    REVEALED.store(false, Ordering::Relaxed);
                    reveal(&win, false);
                    let _ = app.emit("hover-changed", false);
                }
            } else {
                leave_ticks = 0;
            }
        }
    });
}

fn reveal(win: &WebviewWindow, on: bool) {
    if on {
        effects::enable_glass(win, GLASS_COLOR.load(Ordering::Relaxed));
        effects::set_click_through(win, false);
    } else {
        let always = GLASS_ALWAYS.load(Ordering::Relaxed);
        if always {
            // 背景常显：留玻璃 + 可见，但点不到
            effects::enable_glass(win, GLASS_COLOR.load(Ordering::Relaxed));
        } else {
            effects::disable_glass(win);
        }
        // 收起就是穿透。媒体控件的"局部可点"由主循环按光标位置逐拍决定，
        // 这里不重复判断 —— 光标都离开了，本来就该穿透。
        effects::set_click_through(win, true);
    }
    // 刚切过状态，顺手确认一次样式（tao 的其它调用也可能把标题栏塞回来）
    effects::strip_caption(win);
}

/// 光标是否落在媒体控件的矩形里（窗口内物理像素，由前端上报）
#[cfg(windows)]
fn cursor_in_media(win: &WebviewWindow) -> bool {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let rects = match crate::window::MEDIA_RECTS.lock() {
        Ok(g) if !g.is_empty() => g.clone(),
        _ => return false,
    };
    let Ok(raw) = win.hwnd() else { return false };
    let hwnd = HWND(raw.0 as *mut core::ffi::c_void);

    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        return false;
    }
    unsafe {
        let _ = ScreenToClient(hwnd, &mut pt);
    }
    rects
        .iter()
        .any(|&(l, t, r, b)| pt.x >= l && pt.x <= r && pt.y >= t && pt.y <= b)
}

#[cfg(not(windows))]
fn cursor_in_media(_win: &WebviewWindow) -> bool {
    false
}

#[cfg(windows)]
pub fn cursor_over(app: &AppHandle, win: &WebviewWindow, current: bool) -> bool {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let Ok(pos) = win.outer_position() else {
        return false;
    };
    let Ok(size) = win.outer_size() else {
        return false;
    };

    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_err() {
        return false;
    }

    // 光标在设置窗口上时不算"悬停词板"：设置页是普通窗口、压在最上层，
    // 如果它和词板有重叠，光标必然同时落在两块矩形里，必须先判它。
    if let Some(settings) = app.get_webview_window("settings") {
        if settings.is_visible().unwrap_or(false) {
            if let (Ok(sp), Ok(ss)) = (settings.outer_position(), settings.outer_size()) {
                if pt.x >= sp.x
                    && pt.x <= sp.x + ss.width as i32
                    && pt.y >= sp.y
                    && pt.y <= sp.y + ss.height as i32
                {
                    return false;
                }
            }
        }
    }

    let pad = if current { PAD_OUT } else { PAD_IN };
    let left = pos.x - pad;
    let top = pos.y - pad;
    let right = pos.x + size.width as i32 + pad;
    let bottom = pos.y + size.height as i32 + pad;

    pt.x >= left && pt.x <= right && pt.y >= top && pt.y <= bottom
}

#[cfg(not(windows))]
pub fn cursor_over(_app: &AppHandle, _win: &WebviewWindow, _current: bool) -> bool {
    false
}

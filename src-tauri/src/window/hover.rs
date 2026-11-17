use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use super::{effects, now_ms, GLASS_COLOR, LOCKED, PARKED, PEEK_UNTIL, REVEALED};

/// 16ms ≈ 60Hz。鼠标穿透时 WebView 收不到 mousemove，只能读全局光标。
const TICK_MS: u64 = 16;
/// 离开后延迟收起，避免边缘抖动
const LEAVE_DELAY_MS: u64 = 320;
/// 进入判定外扩：更容易触发
const PAD_IN: i32 = 4;
/// 离开判定内收：防止刚滑到边上就消失
const PAD_OUT: i32 = 36;

pub fn spawn(app: AppHandle, win: WebviewWindow) {
    thread::spawn(move || {
        let mut leave_ticks: u64 = 0;

        loop {
            thread::sleep(Duration::from_millis(TICK_MS));

            // 「无播放时自动隐藏」期间窗口是 hide 状态，什么都别碰：
            // 既不该显形，也省掉每拍的样式读写
            if PARKED.load(Ordering::Relaxed) {
                leave_ticks = 0;
                continue;
            }

            // 兜底：只要有谁把 WS_CAPTION 塞回窗口样式（tao 在 set_size /
            // set_always_on_top / show 之后都会整份重写一次），就地摘掉。
            // 样式没变时这里只做两次 GetWindowLongW，开销可以忽略。
            effects::strip_caption(&win);
            // 同理，tao 的整份重写也会把 WS_EX_TRANSPARENT | WS_EX_LAYERED 抹掉。
            // 那两位是"静止态隐形 + 穿透点击"的全部实现 —— 启动即锁定时正好
            // 撞上前端 show()，词板就显了形还挡点击；而 REVEALED 从头到尾都是
            // false，等不到一次状态切换去补救。所以这里每拍对齐一次：
            // 与目标状态一致时是两次 GetWindowLongW，不一致才写。
            effects::set_click_through(&win, !REVEALED.load(Ordering::Relaxed));

            let locked = LOCKED.load(Ordering::Relaxed);
            let current = REVEALED.load(Ordering::Relaxed);

            if locked {
                leave_ticks = 0;
                if current {
                    REVEALED.store(false, Ordering::Relaxed);
                    reveal(&win, false);
                    let _ = app.emit("hover-changed", false);
                }
                continue;
            }

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
        effects::disable_glass(win);
        effects::set_click_through(win, true);
    }
    // 刚切过状态，顺手确认一次样式（tao 的其它调用也可能把标题栏塞回来）
    effects::strip_caption(win);
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

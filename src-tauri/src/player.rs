//! 播放源：Windows SMTC（系统媒体传输控件）。
//!
//! 走系统而不挂钩网易云客户端的原因：SMTC 是播放器向系统注册的统一出口，
//! 网易云、QQ音乐、Spotify、浏览器网页播放全都会出现在这里，
//! 客户端怎么更新都不影响我们读到的曲名 / 歌手 / 进度。

use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

const TICK_MS: u64 = 250;

/// FILETIME 纪元（1601-01-01）到 Unix 纪元的秒数
const EPOCH_DIFF_SECS: f64 = 11_644_473_600.0;

/// 最近一次快照。事件只在变化时推，后开的窗口（设置页）拿不到，
/// 所以这里留一份，让窗口挂载时能主动问一次。
static LATEST: Mutex<Option<NowPlaying>> = Mutex::new(None);

#[derive(Clone, Debug, Serialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub position: f64,
    pub playing: bool,
    /// 这条 timeline 是否是"刚更新过的"。false 表示播放器几秒没刷新过，
    /// 前端据此判断"读到的进度比实际更旧"，不能拿它把歌词往回拽。
    pub fresh: bool,
}

/// 暂停位置的状态机。
///
/// 播放器在暂停那一刻不一定刷新 timeline：`Position` 还是「上次更新那一刻」的值，
/// 比我们播放时外推到的时间点早好几秒。直接把它报出去，前端就会看到进度突然倒退，
/// 把时钟往回拽 —— 表现就是"暂停后歌词先滑到下一句，随后又弹回来"。
/// 所以暂停后第一次读到，先按外推值把位置钉住，之后一直报这个冻结值；
/// 只有 timeline 真的变了（用户在暂停状态下拖进度条）才跟着更新。
#[derive(Default)]
struct PauseState {
    /// 上一拍读到的原始 timeline 位置
    last_raw: f64,
    /// 暂停期间对外报的位置（冻结值）
    frozen: Option<f64>,
    /// 上一拍是否在播
    was_playing: bool,
}

pub fn spawn(app: AppHandle) {
    thread::spawn(move || {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut last_key = String::new();
        let mut was_alive = false;
        let mut pause = PauseState::default();

        loop {
            match read_snapshot(&mut pause) {
                Some(np) => {
                    was_alive = true;
                    store(Some(&np));
                    let key = format!("{}\u{1}{}", np.title, np.artist);
                    if key != last_key {
                        last_key = key.clone();
                        let _ = app.emit("track-changed", &np);
                    }
                    let _ = app.emit("player-tick", &np);
                }
                None => {
                    if was_alive {
                        was_alive = false;
                        last_key.clear();
                        store(None);
                        let _ = app.emit("player-stopped", ());
                    }
                }
            }
            thread::sleep(Duration::from_millis(TICK_MS));
        }
    });
}

fn store(np: Option<&NowPlaying>) {
    if let Ok(mut g) = LATEST.lock() {
        *g = np.cloned();
    }
}

/// 当前播放快照。新开的窗口（设置页）挂载时问一次，避免只听到 tick 却没有曲目信息。
#[tauri::command]
pub fn now_playing() -> Option<NowPlaying> {
    LATEST.lock().ok().and_then(|g| g.clone())
}

#[cfg(windows)]
fn now_unix() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(windows)]
fn is_playing(
    s: &windows::Media::Control::GlobalSystemMediaTransportControlsSession,
) -> bool {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;
    s.GetPlaybackInfo()
        .and_then(|i| i.PlaybackStatus())
        .map(|st| st == Status::Playing)
        .unwrap_or(false)
}

#[cfg(windows)]
fn read_snapshot(st: &mut PauseState) -> Option<NowPlaying> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;

    let mgr = Manager::RequestAsync().ok()?.get().ok()?;

    // 当前会话不等于"正在播的那个"：随便开个网页、视频、语音都能抢走 current session，
    // 这时读到的就是暂停的会话。它没在播时，优先挑一个真正在播放的。
    let mut session = mgr.GetCurrentSession().ok();
    let current_plays = session.as_ref().map(is_playing).unwrap_or(false);
    if !current_plays {
        if let Ok(list) = mgr.GetSessions() {
            for i in 0..list.Size().unwrap_or(0) {
                if let Ok(s) = list.GetAt(i) {
                    if is_playing(&s) {
                        session = Some(s);
                        break;
                    }
                }
            }
        }
    }
    let session = session?;

    let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();

    if title.is_empty() {
        return None;
    }

    let playing = is_playing(&session);

    let (position, duration, fresh) = match session.GetTimelineProperties() {
        Ok(t) => {
            let raw = t.Position().map(|v| v.Duration as f64 / 1e7).unwrap_or(0.0);
            let end = t.EndTime().map(|v| v.Duration as f64 / 1e7).unwrap_or(0.0);

            // SMTC 给的 Position 是「上次更新时间那一刻」的位置，不是现在。
            // 播放器往往几秒才更新一次 timeline，直接用就会整体滞后 ——
            // 歌词慢半句就是这么来的。这里按 LastUpdatedTime 外推到现在。
            let elapsed = t
                .LastUpdatedTime()
                .ok()
                .map(|dt| now_unix() - (dt.UniversalTime as f64 / 1e7 - EPOCH_DIFF_SECS))
                .unwrap_or(0.0);

            // age = 这条 timeline 距现在多久没刷新过。
            // 播放器的刷新间隔普遍在 1~5 秒，所以外推不能只认 2 秒内的：
            // 只认 2 秒会让歌词恒定落后半句（读到的一直是几秒前的进度）。
            // 但也不能无限外推 —— 有些播放器干脆不维护 LastUpdatedTime，
            // 那 elapsed 会是个天文数字，直接加进去就把歌词推到片尾。
            const MAX_AGE: f64 = 15.0;
            const FRESH_SECS: f64 = 2.5;
            let age = if elapsed.is_finite() && elapsed > 0.0 { elapsed } else { 0.0 };
            let valid = age <= MAX_AGE;
            let extrapolate = if valid { age } else { 0.0 };
            let fresh = age <= FRESH_SECS;
            let now_pos = (raw + extrapolate).max(0.0);

            // 暂停时 timeline 不再刷新，外推值会一直往上走，所以只能取一次。
            // 取的就是"这一刻"的外推值 —— 它才是按下暂停时真正的播放位置。
            let raw_changed = (raw - st.last_raw).abs() > 0.05;
            let pos = if playing {
                st.frozen = None;
                st.was_playing = true;
                st.last_raw = raw;
                now_pos
            } else {
                let take = st.was_playing || st.frozen.is_none() || raw_changed;
                if take {
                    st.frozen = Some(now_pos);
                    st.last_raw = raw;
                }
                st.was_playing = false;
                st.frozen.unwrap_or(now_pos)
            };
            let pos = if end > 0.0 { pos.min(end) } else { pos };
            (pos, end, fresh)
        }
        Err(_) => (0.0, 0.0, false),
    };

    Some(NowPlaying {
        title,
        artist,
        album,
        duration,
        position,
        fresh,
        playing,
    })
}

#[cfg(not(windows))]
fn read_snapshot(_st: &mut PauseState) -> Option<NowPlaying> {
    None
}

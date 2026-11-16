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

/// 封面缓存：(曲名 \u{1} 歌手, data URL)。
///
/// 抓封面要开一次流、读几万字节，放进每 250ms 的 tick 里太重，
/// 所以只在换歌时抓一次，前端换歌后再问 `now_cover` 取。
/// 拿不到时存空串，同一首不反复重试。
static COVER: Mutex<Option<(String, String)>> = Mutex::new(None);

fn cover_key(title: &str, artist: &str) -> String {
    format!("{}\u{1}{}", title, artist)
}

/// 标准 Base64。只为封面这一处用，不值得引入依赖。
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

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

/// 当前曲目的封面（data URL）。取不到返回 None。
#[tauri::command]
pub fn now_cover() -> Option<String> {
    let np = LATEST.lock().ok().and_then(|g| g.clone())?;
    let key = cover_key(&np.title, &np.artist);
    let g = COVER.lock().ok()?;
    match g.as_ref() {
        Some((k, v)) if *k == key && !v.is_empty() => Some(v.clone()),
        _ => None,
    }
}

/// 从 SMTC 的缩略图流里读一张封面，编成 data URL。
#[cfg(windows)]
fn grab_cover(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<String> {
    use windows::core::Interface;
    use windows::Storage::Streams::{DataReader, IInputStream, IRandomAccessStreamWithContentType};

    let thumb = props.Thumbnail().ok()?;
    let op: windows::Foundation::IAsyncOperation<IRandomAccessStreamWithContentType> =
        thumb.OpenReadAsync().ok()?;
    let stream = op.get().ok()?;
    let size = stream.Size().ok()? as usize;
    // 正常封面 20~200 KB；超过 3 MB 的一律不要，免得 base64 卡住 IPC
    if size == 0 || size > 3_000_000 {
        return None;
    }

    let input = stream.cast::<IInputStream>().ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    reader.LoadAsync(size as u32).ok()?.get().ok()?;
    let mut buf = vec![0u8; size];
    reader.ReadBytes(&mut buf).ok()?;

    let mime = if buf.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if buf.starts_with(b"GIF8") {
        "image/gif"
    } else if buf.starts_with(b"BM") {
        "image/bmp"
    } else if buf.len() > 12 && &buf[0..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    };
    Some(format!("data:{};base64,{}", mime, b64(&buf)))
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

    // 换歌就抓一次封面；取不到也缓存空串，同一首歌不反复开流
    let key = cover_key(&title, &artist);
    let need = match COVER.lock() {
        Ok(g) => !matches!(g.as_ref(), Some((k, _)) if *k == key),
        Err(_) => false,
    };
    if need {
        let data = grab_cover(&props).unwrap_or_default();
        if let Ok(mut g) = COVER.lock() {
            *g = Some((key, data));
        }
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

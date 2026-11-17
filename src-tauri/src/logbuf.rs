//! 运行日志：
//!
//! 1. **内存环形缓冲**（3000 行）—— 导出时拿它；
//! 2. **磁盘镜像**（`%APPDATA%\com.yuluo.lyricsfloat\logs\session.log`）——
//!    崩溃时来不及导出也有现场，下次启动能捡回来；
//! 3. **运行标记**（`running.mark`）—— 正常退出会删掉，残留就说明上次没退干净。
//!
//! 日志只在用户点「导出日志」或崩溃后确认时才写成独立文件，平时不占地方。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 上限按行数算：一行几百字节，3000 行大约 1 MB，够复盘又不至于失控
const MAX_LINES: usize = 3000;
/// 单行截断，防止有人把一整篇歌词/JSON 塞进来
const MAX_LINE_CHARS: usize = 600;
/// 镜像文件到这么大就不再写（防止挂一天把盘写满）
const MAX_MIRROR_BYTES: u64 = 8 * 1024 * 1024;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// 本次启动是不是"上次没退干净"
static CRASHED: AtomicBool = AtomicBool::new(false);
/// 镜像文件是否还能写（超限后关掉，避免每次 push 都去查文件大小）
static MIRROR_OK: AtomicBool = AtomicBool::new(true);

#[cfg(target_os = "windows")]
fn stamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

#[cfg(not(target_os = "windows"))]
fn stamp() -> String {
    String::from("--")
}

fn log_dir() -> Option<PathBuf> {
    let base = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(base)
        .join("com.yuluo.lyricsfloat")
        .join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn mirror_path() -> Option<PathBuf> {
    Some(log_dir()?.join("session.log"))
}

fn mark_path() -> Option<PathBuf> {
    Some(log_dir()?.join("running.mark"))
}

/// 崩溃日志（上一次会话留下的镜像，启动时被改名成这个）
fn crash_path() -> Option<PathBuf> {
    Some(log_dir()?.join("crash.log"))
}

/// 记一行。level 用 INFO / WARN / ERROR，方便导出后 grep。
pub fn push(level: &str, msg: &str) {
    let mut body: String = msg.chars().take(MAX_LINE_CHARS).collect();
    if msg.chars().count() > MAX_LINE_CHARS {
        body.push('…');
    }
    let line = format!("{} {:<5} {}", stamp(), level, body.replace('\n', " ⏎ "));
    if let Ok(mut g) = LOG.lock() {
        if g.len() >= MAX_LINES {
            g.pop_front();
        }
        g.push_back(line.clone());
    }
    mirror(&line);
}

/// 往磁盘镜像追加一行。写失败不报错 —— 日志不能反过来影响主流程。
fn mirror(line: &str) {
    if !MIRROR_OK.load(Ordering::Relaxed) {
        return;
    }
    let Some(path) = mirror_path() else { return };
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_MIRROR_BYTES {
            MIRROR_OK.store(false, Ordering::Relaxed);
            return;
        }
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn info(msg: &str) {
    push("INFO", msg);
}
pub fn warn(msg: &str) {
    push("WARN", msg);
}
pub fn error(msg: &str) {
    push("ERROR", msg);
}

/// 启动时调一次：判断上次是否异常退出，并把上一份镜像留成崩溃日志。
///
/// 顺序很重要 —— 先看标记、再改名、最后写新标记，这样中途崩了也只丢一次机会。
pub fn begin_session() {
    let crashed = mark_path().map(|p| p.exists()).unwrap_or(false);
    if crashed {
        CRASHED.store(true, Ordering::Relaxed);
        if let (Some(src), Some(dst)) = (mirror_path(), crash_path()) {
            if src.exists() {
                let _ = std::fs::remove_file(&dst);
                let _ = std::fs::rename(&src, &dst);
            }
        }
    }
    // 新会话的镜像从空开始（改名已经把旧的挪走了；首次运行本来就没有）
    if let Some(p) = mirror_path() {
        if std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false) && !crashed {
            // 正常退出时标记已删、镜像还在：说明是上一轮正常结束，清掉重开
            let _ = std::fs::remove_file(&p);
        }
    }
    if let Some(p) = mark_path() {
        let _ = std::fs::write(&p, format!("pid={} ver={}\n", std::process::id(), crate::about::APP_VERSION));
    }
}

/// 正常退出时调：删掉运行标记。镜像留着，下次启动会当"上一轮正常结束"清掉。
pub fn end_session() {
    if let Some(p) = mark_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// 上次是不是没退干净（前端据此决定要不要弹提示）
pub fn crashed_last_time() -> bool {
    CRASHED.load(Ordering::Relaxed)
}

/// 上次的日志有多大、多少行 —— 给提示语用，让人知道值不值得导出
pub fn crash_size() -> Option<u64> {
    crash_path().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len())
}

/// 把上次的日志导出到软件目录，返回路径。
pub fn export_crash() -> Result<String, String> {
    let src = crash_path().ok_or("找不到上次的日志")?;
    let body = std::fs::read_to_string(&src).map_err(|e| e.to_string())?;
    let mut text = String::new();
    text.push_str("LyricsFloat 崩溃日志（上一次运行，未能正常退出）\n");
    text.push_str(&format!("版本    : {}\n", crate::about::APP_VERSION));
    text.push_str(&format!("系统    : {}\n", os_text()));
    text.push_str("------------------------------------------------------------\n");
    text.push_str(&body);

    let name = format!("LyricsFloat-crash-{}.txt", crate::settings_io::timestamp());
    for dir in export_dirs() {
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let path = dir.join(&name);
        if std::fs::write(&path, &text).is_ok() {
            return Ok(path.to_string_lossy().to_string());
        }
    }
    Err("没有可写的目录".into())
}

/// 崩溃后主动问一次要不要导出。放后台线程里，别卡启动。
#[cfg(target_os = "windows")]
pub fn prompt_crash_if_any() {
    if !crashed_last_time() {
        return;
    }
    std::thread::spawn(move || {
        use windows::core::PCWSTR;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO,
        };
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let size = crash_size().unwrap_or(0);
        let ask: Vec<u16> = format!(
            "LyricsFloat 上次没有正常退出。\n\n是否导出上次的日志？\n（约 {} KB，含版本、系统与事件流水，不含歌词内容）",
            (size / 1024).max(1)
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
        let title: Vec<u16> = "LyricsFloat".encode_utf16().chain(std::iter::once(0)).collect();
        let ret = unsafe {
            MessageBoxW(
                None,
                PCWSTR(ask.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND,
            )
        };
        if ret != IDYES {
            info("崩溃日志：用户选择不导出");
            return;
        }
        match export_crash() {
            Ok(path) => {
                info(&format!("崩溃日志已导出：{path}"));
                let done: Vec<u16> = format!("已导出：\n{path}")
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                unsafe {
                    MessageBoxW(
                        None,
                        PCWSTR(done.as_ptr()),
                        PCWSTR(title.as_ptr()),
                        MB_SETFOREGROUND,
                    );
                }
            }
            Err(e) => error(&format!("崩溃日志导出失败：{e}")),
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn prompt_crash_if_any() {}

/// 前端也能往同一份日志里写 —— 界面侧的失败（IPC 报错、解析失败）在 Rust 里看不到。
#[tauri::command]
pub fn log_line(level: String, msg: String) {
    let lv = match level.to_uppercase().as_str() {
        "WARN" | "WARNING" => "WARN",
        "ERROR" | "ERR" => "ERROR",
        _ => "INFO",
    };
    push(lv, &msg);
}

/// 导出到软件目录（exe 同目录），返回完整路径。
/// exe 目录不可写时（比如放在 Program Files）退到应用数据目录。
#[tauri::command]
pub fn export_log() -> Result<String, String> {
    let name = format!("LyricsFloat-log-{}.txt", crate::settings_io::timestamp());
    let body = render();

    let mut last_err = String::from("没有可写的目录");
    for dir in export_dirs() {
        if std::fs::create_dir_all(&dir).is_err() {
            continue;
        }
        let path = dir.join(&name);
        match std::fs::write(&path, &body) {
            Ok(()) => {
                info(&format!("日志已导出：{}", path.display()));
                return Ok(path.to_string_lossy().to_string());
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

fn export_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.to_path_buf());
        }
    }
    if let Some(dir) = log_dir() {
        out.push(dir);
    }
    out
}

/// 导出文件的正文：先一段环境信息，再是日志本身（导出时现状，不含导出这一行）
fn render() -> String {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str("LyricsFloat 运行日志\n");
    out.push_str(&format!("版本    : {}\n", crate::about::APP_VERSION));
    out.push_str(&format!(
        "程序    : {}\n",
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "未知".into())
    ));
    out.push_str(&format!(
        "设置文件: {}\n",
        crate::settings_io::settings_path_cmd()
    ));
    out.push_str(&format!("系统    : {}\n", os_text()));
    out.push_str(&format!("行数    : {}\n", LOG.lock().map(|g| g.len()).unwrap_or(0)));
    out.push_str("------------------------------------------------------------\n");
    if let Ok(g) = LOG.lock() {
        for line in g.iter() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn os_text() -> String {
    let os = std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.into());
    format!("{} {}", os, std::env::consts::ARCH)
}

//! 运行期日志：**只存在内存里**，进程退出即消失，导出时才写成文件。
//!
//! 目的是「用户反馈问题时能把现场带走」：环形缓冲，够长又不吃内存，
//! 磁盘上平时一个文件都不留 —— 只有点了「导出日志」才在软件目录生成一份。

use std::collections::VecDeque;
use std::sync::Mutex;

/// 上限按行数算：一行几百字节，3000 行大约 1 MB，够复盘又不至于失控
const MAX_LINES: usize = 3000;
/// 单行截断，防止有人把一整篇歌词/JSON 塞进来
const MAX_LINE_CHARS: usize = 600;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

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
        g.push_back(line);
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

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(
            std::path::PathBuf::from(appdata)
                .join("com.yuluo.lyricsfloat")
                .join("logs"),
        );
    }

    let mut last_err = String::from("没有可写的目录");
    for dir in candidates {
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
    // 不为了一个版本号去调 GetVersionExW（会被兼容层骗，还得开额外 feature），
    // 系统名 + 架构就够定位问题了
    let os = std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.into());
    format!("{} {}", os, std::env::consts::ARCH)
}

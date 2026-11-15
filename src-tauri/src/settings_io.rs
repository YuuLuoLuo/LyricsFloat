//! 设置的落盘与导入导出。
//!
//! 存储位置：**应用自己的数据目录** `%APPDATA%\com.yuluo.lyricboard\settings.json`。
//!
//! 早先只存 WebView2 的 localStorage，换机器就丢；后来改成 exe 同目录的 json，
//! 结果绿色版旁边多一个文件、拷来拷去容易漏。现在收回到"程序内部"：
//! 目录由应用自己管，exe 目录保持干干净净只有一个 exe。
//! 需要备份/迁移时用导出导入（导出到桌面、把 json 拖回设置窗口）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const FILE_NAME: &str = "settings.json";
/// 上一版放在 exe 同目录的文件名，首次启动时搬进来
const LEGACY_NAME: &str = "lyric-board.settings.json";
/// 导出文件里带个版本号，将来字段改名时还能认出来
const SCHEMA: &str = "lyric-board.settings/1";

fn appdata_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("com.yuluo.lyricboard"))
}

fn exe_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(Path::to_path_buf)
}

/// 目录能不能写（万一 APPDATA 被策略锁死，得退回 exe 目录）
fn writable(dir: &Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-probe");
    if fs::write(&probe, b"1").is_err() {
        return false;
    }
    let _ = fs::remove_file(&probe);
    true
}

static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 设置的存放位置。缓存起来：每次保存都去建目录、写探测文件太浪费。
fn settings_path() -> Option<PathBuf> {
    PATH.get_or_init(|| {
        if let Some(dir) = appdata_dir() {
            if writable(&dir) {
                return Some(dir.join(FILE_NAME));
            }
        }
        if let Some(dir) = exe_dir() {
            if writable(&dir) {
                return Some(dir.join(FILE_NAME));
            }
        }
        None
    })
    .clone()
}

fn legacy_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = exe_dir() {
        out.push(d.join(LEGACY_NAME));
    }
    if let Some(d) = appdata_dir() {
        out.push(d.join(LEGACY_NAME));
    }
    out
}

/// 已存在的设置文件：新位置优先，其次上一版的位置（只读兜底）。
fn existing_path() -> Option<PathBuf> {
    if let Some(p) = settings_path() {
        if p.is_file() {
            return Some(p);
        }
    }
    legacy_candidates().into_iter().find(|p| p.is_file())
}

/// 把上一版留在 exe 旁边的设置搬进应用数据目录，搬完删掉旧文件 ——
/// 就是"exe 旁边多一个文件不方便整体挪走"这件事。
fn migrate_legacy() {
    let Some(dst) = settings_path() else { return };
    if dst.is_file() {
        return;
    }
    for old in legacy_candidates() {
        if old == dst {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&old) {
            if atomic_write(&dst, &text).is_ok() {
                let _ = fs::remove_file(&old);
            }
            return;
        }
    }
}

fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // 两个窗口可能同时保存，先写临时文件再 rename，读到的永远是完整的一份
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// 当前设置文件路径（给界面显示"设置存在哪"）
#[tauri::command]
pub fn settings_path_cmd() -> String {
    settings_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 读设置。文件不存在返回 None，前端据此决定"用默认值并立刻写一份"。
#[tauri::command]
pub fn settings_load() -> Option<String> {
    migrate_legacy();
    let path = existing_path()?;
    fs::read_to_string(path).ok()
}

/// 写设置。返回实际写入的路径，失败返回空串。
#[tauri::command]
pub fn settings_save(json: String) -> String {
    migrate_legacy();
    let Some(path) = settings_path() else {
        return String::new();
    };
    match atomic_write(&path, &json) {
        Ok(()) => path.to_string_lossy().to_string(),
        Err(_) => String::new(),
    }
}

/// 导出到桌面。返回写出的完整路径。
#[tauri::command]
pub fn settings_export(json: String) -> String {
    let Some(dir) = desktop_dir() else {
        return String::new();
    };
    let stamp = timestamp();
    let path = dir.join(format!("歌词板设置-{stamp}.json"));
    // 导出的是"给人看的备份"，包一层 schema 方便以后校验；导入时两种形态都认
    let body = match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(v) => serde_json::json!({ "schema": SCHEMA, "settings": v }),
        Err(_) => return String::new(),
    };
    let text = serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string());
    match fs::write(&path, text) {
        Ok(()) => path.to_string_lossy().to_string(),
        Err(_) => String::new(),
    }
}

/// 读任意路径的备份文件（拖进设置窗口时用）。
#[tauri::command]
pub fn settings_import_file(path: String) -> Option<String> {
    let p = PathBuf::from(path);
    // 只认 json，且限个大小，别让人拖个 2GB 的电影进来
    let meta = fs::metadata(&p).ok()?;
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    if p.extension().map(|e| e.eq_ignore_ascii_case("json")) != Some(true) {
        return None;
    }
    fs::read_to_string(p).ok()
}

#[cfg(target_os = "windows")]
fn desktop_dir() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Desktop, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s
    }
}

#[cfg(not(target_os = "windows"))]
fn desktop_dir() -> Option<PathBuf> {
    None
}

/// 本地时间戳 YYYYMMDD-HHMM，纯手算避免为一个文件名引入 chrono。
fn timestamp() -> String {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let t = unsafe { GetLocalTime() };
        format!(
            "{:04}{:02}{:02}-{:02}{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        String::from("backup")
    }
}

//! 启动前的两道闸：单实例、版本回退。
//!
//! 两件事共同的目的：**别让两个进程同时写同一份 `settings.json`**。
//! 设置文件是整份覆盖写入的，两个实例各写各的必然互相踩；新旧两个版本的字段集
//! 又不一样，旧版读不懂新版写的键、保存时会把它们整片抹掉 —— 用户看到的就是
//! "设置莫名其妙变回默认"。所以这两道检查都放在 tauri 起来之前：
//! 没过闸就直接返回，一个字都不会写盘。

#[cfg(windows)]
use std::path::PathBuf;

/// 互斥体名。**故意不带版本号** —— 新旧两个版本的 exe 也要互相排斥。
#[cfg(windows)]
const MUTEX_NAME: &str = "LyricsFloat.SingleInstance";

/// 记录"这台机器上运行过的最高版本"，用来发现版本回退（先开新版又开旧版）。
const VERSION_FILE: &str = "last-version.txt";

/// 抢单实例锁。返回 false = 已经有一个在跑。
///
/// 句柄故意不关：进程活着锁就在，进程一退（含崩溃）系统自动释放，
/// 所以不会像"标记文件"那样留下脏状态、下次启动误判。
#[cfg(windows)]
pub fn acquire_single_instance() -> bool {
    acquire_named(MUTEX_NAME)
}

/// 按名字抢互斥体。抽出来是为了让单测能用独立的名字，不跟真正在跑的实例打架。
#[cfg(windows)]
fn acquire_named(name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let mut buf: Vec<u16> = name.encode_utf16().collect();
    buf.push(0);

    unsafe {
        match CreateMutexW(None, false, PCWSTR(buf.as_ptr())) {
            Ok(h) => {
                // 句柄不 CloseHandle：进程活着锁就在，进程一退系统自动释放。
                // HANDLE 没有 Drop 实现，这里放着不管就是"持有到进程结束"。
                let _ = h;
                // 已存在时 CreateMutexW 仍返回有效句柄，只是 last error 是 183
                GetLastError() != ERROR_ALREADY_EXISTS
            }
            // 真创建不出来（权限之类）就别拦着用户启动，宁可放行
            Err(_) => true,
        }
    }
}

#[cfg(not(windows))]
pub fn acquire_single_instance() -> bool {
    true
}

/// 第二个实例被拦下时的提示。纯告知，不需要用户做选择。
#[cfg(windows)]
pub fn show_already_running() {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
    };
    msgbox(
        "LyricsFloat 已经在运行了。\n\n\
         为避免两个实例同时写配置文件把设置弄坏，这次不再启动。\n\
         需要打开设置页的话，请用任务栏托盘里的 LyricsFloat 图标。",
        "LyricsFloat",
        MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
    );
}

#[cfg(not(windows))]
pub fn show_already_running() {}

/// 检查版本回退。返回 false = 用户选择退出，不该继续启动。
///
/// 判定方式：把当前版本和 `last-version.txt` 里记的最高版本比。
/// 当前更低说明"先跑过新版，现在跑的是旧版"——旧版不认识新版写的设置项，
/// 一旦保存就会把那些键整片抹掉，所以要问一句。
///
/// 记录文件在 tauri 之前读写，跟 `settings.json` 完全独立：
/// 即使用户选择退出，配置文件也是一个字节都没动过的。
#[cfg(windows)]
pub fn check_version_rollback() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO,
    };

    let cur = crate::about::APP_VERSION;
    let Some(path) = version_file() else {
        return true;
    };

    let prev = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref prev) = prev {
        if crate::about::ver_cmp(cur, prev) == std::cmp::Ordering::Less {            let text = format!(
                "这台电脑上运行过更新的版本（{prev}），\n\
                 而你现在打开的是较旧的版本（{cur}）。\n\n\
                 旧版不认识新版写入的设置项，继续使用可能把它覆盖掉，\n\
                 导致部分设置失效或恢复默认。\n\n\
                 配置文件：\n{}\n\
                 建议先复制一份备份。\n\n\
                 仍要继续打开吗？\n  是：继续（会正常读写配置）\n  否：退出（不会写入任何配置）",
                settings_display()
            );
            let ret = msgbox(
                &text,
                "LyricsFloat — 版本回退提醒",
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND,
            );
            if ret != IDYES.0 {
                return false;
            }
            // 用户接受用旧版：把记录降到当前版本，免得下次又弹一遍
            log_ver(&path, cur);
            return true;
        }
    }

    if prev.as_deref() != Some(cur) {
        log_ver(&path, cur);
    }
    true
}

#[cfg(not(windows))]
pub fn check_version_rollback() -> bool {
    true
}

/// 版本记录文件：`%APPDATA%\com.yuluo.lyricsfloat\last-version.txt`
#[cfg(windows)]
fn version_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("com.yuluo.lyricsfloat").join(VERSION_FILE))
}

#[cfg(windows)]
fn log_ver(path: &PathBuf, ver: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 写不进去也不影响启动：下次只会重复判一次，不会出错
    let _ = std::fs::write(path, ver);
}

/// 配置文件路径，只用于在弹窗里显示给用户看（不含用户名以外的敏感信息）。
#[cfg(windows)]
fn settings_display() -> String {
    crate::settings_io::settings_path_cmd()
}

#[cfg(windows)]
fn msgbox(text: &str, title: &str, flags: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE) -> i32 {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::MessageBoxW;

    let body: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let cap: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { MessageBoxW(None, PCWSTR(body.as_ptr()), PCWSTR(cap.as_ptr()), flags).0 }
}

#[cfg(test)]
mod tests {
    /// 同一个名字第二次拿必须失败 —— 这就是"已在运行"的判据。
    /// 用独立名字，避免跟真正在跑的用户实例互相干扰。
    #[cfg(windows)]
    #[test]
    fn second_acquire_same_name_fails() {
        let name = format!("LyricsFloat.Test.{}", std::process::id());
        assert!(super::acquire_named(&name), "第一次应该拿到锁");
        assert!(!super::acquire_named(&name), "第二次应判定为已在运行");
    }

    #[cfg(windows)]
    #[test]
    fn different_names_do_not_collide() {
        let a = format!("LyricsFloat.TestA.{}", std::process::id());
        let b = format!("LyricsFloat.TestB.{}", std::process::id());
        assert!(super::acquire_named(&a));
        assert!(super::acquire_named(&b));
    }
}

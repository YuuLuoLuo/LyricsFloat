//! 关于页需要的三件小事：版本号、打开链接、查 GitHub Releases 有没有新版本。
//!
//! 不引 tauri-plugin-updater / opener 的原因：这里只需要"读一下 latest release
//! 的 tag"和"交给系统默认浏览器打开一个 https 链接"，两个都是系统调用级别的活，
//! 没必要为此多背两个插件和它们的权限声明。

use std::time::Duration;

use serde::{Deserialize, Serialize};

const UA: &str = "lyric-board-update-check";

#[derive(Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub url: String,
    pub notes: String,
    pub published: String,
    pub newer: bool,
}

#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 只放行 http(s)：别让前端把 file:// 之类的东西丢给系统去执行
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        return Err("只允许打开 http(s) 链接".into());
    }
    open_external(u)
}

#[cfg(windows)]
fn open_external(url: &str) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb_s = HSTRING::from("open");
    let url_s = HSTRING::from(url);
    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb_s.as_ptr()),
            PCWSTR(url_s.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 的返回值 <= 32 都算失败（老 API 的约定）
    if ret.0 as isize <= 32 {
        Err("系统拒绝打开这个链接".into())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn open_external(_url: &str) -> Result<(), String> {
    Err("当前平台不支持".into())
}

/// 查 `owner/repo` 的最新 Release，和当前版本比一下。
///
/// 失败时把原因原样带回前端显示 —— 网络不通、仓库还没发布、地址写错，
/// 这三种情况在"关于"页里应该看得出区别。
#[tauri::command]
pub async fn check_update(repo: String) -> Result<UpdateInfo, String> {
    let repo = repo.trim().trim_matches('/').to_string();
    if repo.split('/').count() != 2 {
        return Err("仓库地址还没配置".into());
    }
    tauri::async_runtime::spawn_blocking(move || fetch_latest(&repo))
        .await
        .map_err(|e| e.to_string())?
}

fn fetch_latest(repo: &str) -> Result<UpdateInfo, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build();

    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let text = match agent
        .get(&url)
        .set("User-Agent", UA)
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(resp) => resp.into_string().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Err("仓库还没有发布过版本".into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub 返回 {code}")),
        Err(err) => return Err(format!("连不上 GitHub：{err}")),
    };

    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| "返回内容不是预期格式".to_string())?;

    let latest = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if latest.is_empty() {
        return Err("没有读到版本号".into());
    }

    let current = app_version();
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://github.com/{repo}/releases/latest"));

    // Release 说明可能很长，截断到够看的长度就停
    let notes = json
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .chars()
        .take(900)
        .collect::<String>();

    let published = json
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .chars()
        .take(10)
        .collect::<String>();

    Ok(UpdateInfo {
        newer: is_newer(&latest, &current),
        current,
        latest,
        url,
        notes,
        published,
    })
}

/// 版本号只比数字：`v0.2.1` / `0.2.1-beta` 都按 [0,2,1] 处理。
/// 预发布标识不参与比较 —— 这个项目只往 Releases 发正式版。
fn parse_ver(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches(['v', 'V'])
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .take(4)
        .filter_map(|p| p.parse::<u64>().ok())
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    let a = parse_ver(latest);
    let b = parse_ver(current);
    for i in 0..a.len().max(b.len()) {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.3.0", "0.2.9"));
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("", "0.2.0"));
    }
}

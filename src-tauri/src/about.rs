//! 关于页需要的三件小事：版本号、打开链接、查 GitHub Releases 有没有新版本。
//!
//! 不引 tauri-plugin-updater / opener 的原因：这里只需要"读一下 latest release
//! 的 tag"和"交给系统默认浏览器打开一个 https 链接"，两个都是系统调用级别的活，
//! 没必要为此多背两个插件和它们的权限声明。

use std::time::Duration;

use serde::{Deserialize, Serialize};

const UA: &str = "lyricsfloat-update-check";

/// 版本号：年份后两位.月份_当月第几次编译（例如 `26.11_3`）。
/// 发 GitHub Release 时 tag 用同一个字符串（带不带 v 前缀都认）。
///
/// 注意：exe 的文件属性（ProductVersion / FileVersion）走的是资源格式里
/// 的数字四段，写不了下划线，所以 Cargo.toml / tauri.conf.json 里同步写成
/// `26.11.2`（点号版，语义完全一样），下面 `version_matches_cargo` 测试守着两边不走样。
pub const APP_VERSION: &str = "26.11_16";

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
    APP_VERSION.to_string()
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

    // Release 说明是 Markdown / 少量 HTML（<details> 之类），这里只留纯文本：
    // 前端用 <pre> 渲染，标签原样显示会很难看，而解析 Markdown 又太重。
    let notes = tidy_notes(
        json.get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    );

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

/// 版本号按「年份.月份_第几次编译」比：`26.11_3` -> [26, 11, 3]。
/// 也兼容点号写法和老式的 `0.2.0`，tag 前缀 v 忽略。
fn parse_ver(s: &str) -> Vec<u64> {
    let t = s.trim().trim_start_matches(['v', 'V']);
    let mut out = Vec::new();
    for part in t.split(|c: char| c == '.' || c == '_' || c == '-' || c == '+') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            if out.is_empty() {
                continue; // 开头的杂字符
            }
            break;
        }
        out.push(digits.parse::<u64>().unwrap_or(0));
    }
    out
}

/// 按「年份.月份_第几次编译」比大小。启动时的版本回退检查要用。
pub(crate) fn ver_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (parse_ver(a), parse_ver(b));
    for i in 0..x.len().max(y.len()) {
        let vx = *x.get(i).unwrap_or(&0);
        let vy = *y.get(i).unwrap_or(&0);
        match vx.cmp(&vy) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
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

/// Release 说明清洗成"可读的纯文本"：
/// - HTML 标签（<details>、<a href>…）直接扔掉，只留标签之间的内容
/// - Markdown 链接 [文字](url) 留文字
/// - 连续空行压成一行，行尾空白去掉
fn tidy_notes(body: &str) -> String {
    let mut out = String::with_capacity(body.len().min(1024));
    let mut in_tag = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '[' => {
                // 吃掉 [text](url) 里的 (url)：遇到 ']' 且下一个是 '(' 就跳过整个括号
                let mut text = String::new();
                while let Some(&n) = chars.peek() {
                    if n == ']' {
                        chars.next();
                        break;
                    }
                    text.push(n);
                    chars.next();
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    let mut paren = 1usize;
                    for n in chars.by_ref() {
                        if n == '(' {
                            paren += 1;
                        } else if n == ')' {
                            paren -= 1;
                            if paren == 0 {
                                break;
                            }
                        }
                    }
                }
                out.push_str(&text);
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let mut cleaned: Vec<&str> = out
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    if cleaned.len() > 40 {
        cleaned.truncate(40);
    }
    let mut s = cleaned.join("\n");
    if s.chars().count() > 900 {
        s = s.chars().take(900).collect::<String>() + "…";
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        // 年份.月份_第几次编译
        assert!(is_newer("26.11_2", "26.11_1"));
        assert!(!is_newer("26.11_1", "26.11_1"));
        assert!(!is_newer("26.11_1", "26.11_2"));
        assert!(is_newer("v26.12_1", "26.11_9"));
        assert!(is_newer("27.1_1", "26.12_9"));
        assert!(is_newer("2027.1_1", "2026.12_9"));
        // 点号版（exe 文件属性那种写法）和带下划线的等价
        assert!(!is_newer("26.11.2", "26.11_2"));
        // 老式语义仍然可用
        assert!(is_newer("0.3.0", "0.2.9"));
        assert!(!is_newer("", "26.11_1"));
    }

    /// 启动时的版本回退检查靠它：当前版本比记录的低就该拦。
    #[test]
    fn orders_versions() {
        use std::cmp::Ordering;
        assert_eq!(ver_cmp("26.11_7", "26.11_6"), Ordering::Greater);
        assert_eq!(ver_cmp("26.11_6", "26.11_7"), Ordering::Less);
        assert_eq!(ver_cmp("26.11_7", "26.11_7"), Ordering::Equal);
        // 点号 / 下划线 / v 前缀都等价，别因为写法不同误判成回退
        assert_eq!(ver_cmp("26.11.7", "26.11_7"), Ordering::Equal);
        assert_eq!(ver_cmp("v26.11_7", "26.11_7"), Ordering::Equal);
        // 位不够时补 0：26.11 和 26.11_0 是同一个版本
        assert_eq!(ver_cmp("26.11", "26.11_0"), Ordering::Equal);
        assert_eq!(ver_cmp("27.1_1", "26.12_9"), Ordering::Greater);
    }

    /// APP_VERSION 和 Cargo.toml 的 package.version 必须对得上，
    /// 否则"关于"页显示 26.11_2、exe 属性却是别的数，发版时容易搞错。
    #[test]
    fn version_matches_cargo() {
        let cargo = env!("CARGO_PKG_VERSION"); // 例如 "26.11.2"
        let dotted = APP_VERSION.replace('_', ".");
        assert_eq!(cargo, dotted, "请同步修改 src-tauri/Cargo.toml 的 version");
    }

    #[test]
    fn cleans_release_notes() {
        let src = "<details><summary>更新</summary>\n\n## 新内容\n\n- 支持 [翻译](https://example.com)\n</details>\n";
        assert_eq!(tidy_notes(src), "更新\n## 新内容\n- 支持 翻译");
    }
}

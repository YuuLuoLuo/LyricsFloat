//! 歌词获取：网易云优先，lrclib 兜底，结果落盘缓存。
//!
//! 为什么不用第三方插件：插件跟着客户端版本走，客户端一更新就废。
//! 这里只用公开的搜索/歌词接口，按"曲名 + 歌手 + 时长"三重校验匹配，
//! 拿不到就退到 lrclib（开源歌词库，纯 LRC），两条都不通就是没有歌词。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LyricResult {
    pub lrc: String,
    pub trans: String,
    pub yrc: String,
    pub source: String,
    pub matched_title: String,
    pub matched_artist: String,
}

#[tauri::command]
pub async fn fetch_lyric(
    title: String,
    artist: String,
    duration: f64,
) -> Result<Option<LyricResult>, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_sync(&title, &artist, duration))
        .await
        .map_err(|e| e.to_string())
}

fn fetch_sync(title: &str, artist: &str, duration: f64) -> Option<LyricResult> {
    let key = cache_key(title, artist);
    if let Some(hit) = read_cache(&key) {
        crate::logbuf::info(&format!("歌词命中缓存：{title} - {artist}（{}）", hit.source));
        return Some(hit);
    }

    let found = from_netease(title, artist, duration).or_else(|| from_lrclib(title, artist, duration));
    match &found {
        Some(r) => {
            crate::logbuf::info(&format!(
                "歌词匹配成功：{title} - {artist} → {} / {}（{}）",
                r.matched_title, r.matched_artist, r.source
            ));
            write_cache(&key, r);
        }
        None => crate::logbuf::warn(&format!("歌词匹配失败：{title} - {artist}")),
    }
    found
}

// ---------------------------------------------------------------- 网易云

#[derive(Deserialize)]
struct NeSearch {
    result: Option<NeResult>,
}

#[derive(Deserialize)]
struct NeResult {
    songs: Option<Vec<NeSong>>,
}

#[derive(Deserialize)]
struct NeSong {
    id: i64,
    name: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    artists: Vec<NeArtist>,
    #[serde(default)]
    ar: Vec<NeArtist>,
}

#[derive(Deserialize)]
struct NeArtist {
    name: String,
}

#[derive(Deserialize)]
struct NeLyric {
    lrc: Option<NeText>,
    tlyric: Option<NeText>,
    yrc: Option<NeText>,
}

#[derive(Deserialize)]
struct NeText {
    #[serde(default)]
    lyric: String,
}

fn from_netease(title: &str, artist: &str, duration: f64) -> Option<LyricResult> {
    let q = encode(&format!("{title} {artist}"));
    let search_url =
        format!("https://music.163.com/api/search/get/web?s={q}&type=1&offset=0&limit=20");
    let body = http_get(&search_url, Some("https://music.163.com/"))?;
    let parsed: NeSearch = serde_json::from_str(&body).ok()?;
    let songs = parsed.result?.songs?;

    let target = norm(title);
    let target_artist = norm(artist);

    let mut ranked: Vec<(f64, &NeSong)> = songs
        .iter()
        .map(|s| (score(s, &target, &target_artist, duration), s))
        .filter(|(sc, _)| *sc >= 30.0)
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 最高分的候选可能挂在"暂无歌词"的翻唱上，所以按分数顺序试前几个
    for (_, song) in ranked.iter().take(3) {
        let lyric_url = format!(
            "https://music.163.com/api/song/lyric?id={}&lv=-1&kv=-1&tv=-1&yv=-1&ytv=-1&yrv=-1",
            song.id
        );
        let Some(body) = http_get(&lyric_url, Some("https://music.163.com/")) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<NeLyric>(&body) else {
            continue;
        };

        let raw_lrc = parsed.lrc.map(|x| x.lyric).unwrap_or_default();
        let raw_yrc = parsed.yrc.map(|x| x.lyric).unwrap_or_default();
        let trans = parsed.tlyric.map(|x| x.lyric).unwrap_or_default();

        let lrc = sanitize(&raw_lrc);
        let yrc = sanitize(&raw_yrc);
        if lrc.is_none() && yrc.is_none() {
            continue;
        }

        let artists = if song.artists.is_empty() {
            &song.ar
        } else {
            &song.artists
        };

        return Some(LyricResult {
            lrc: lrc.unwrap_or_default(),
            trans,
            yrc: yrc.unwrap_or_default(),
            source: "网易云".into(),
            matched_title: song.name.clone(),
            matched_artist: artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join("/"),
        });
    }

    None
}

fn score(song: &NeSong, target: &str, target_artist: &str, duration: f64) -> f64 {
    let mut score = 0.0;
    let name = norm(&song.name);
    if name == *target {
        score += 60.0;
    } else if !target.is_empty() && (name.contains(target) || target.contains(&name)) {
        score += 34.0;
    }
    let artists = if song.artists.is_empty() {
        &song.ar
    } else {
        &song.artists
    };
    let joined = artists
        .iter()
        .map(|a| norm(&a.name))
        .collect::<Vec<_>>()
        .join(" ");
    if !target_artist.is_empty() && joined.contains(target_artist) {
        score += 26.0;
    }
    if duration > 1.0 && song.duration > 1.0 {
        let diff = (song.duration / 1000.0 - duration).abs();
        if diff <= 2.0 {
            score += 24.0;
        } else if diff <= 6.0 {
            score += 10.0;
        } else if diff > 25.0 {
            score -= 30.0;
        }
    }
    score
}

/// 去掉制作人名单、过滤"暂无歌词"这类占位文本。
/// 只剩不到两句就认为没拿到真歌词。
fn sanitize(raw: &str) -> Option<String> {
    const PLACEHOLDER: [&str; 5] = ["暂无歌词", "纯音乐", "没有歌词", "此歌曲为没有填词", "instrumental"];
    const CREDITS: [&str; 19] = [
        "作词", "作曲", "编曲", "制作人", "监制", "混音", "录音", "母带", "吉他", "贝斯", "鼓",
        "和声", "配唱", "统筹", "策划", "出品", "发行", "OP", "SP",
    ];

    let mut out = Vec::new();
    let mut timed = 0;

    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if PLACEHOLDER.iter().any(|p| t.contains(p)) {
            continue;
        }

        let body = t
            .split(']')
            .last()
            .unwrap_or("")
            .trim()
            .to_string();

        let is_credit = CREDITS
            .iter()
            .any(|c| body.starts_with(c) && (body.contains(':') || body.contains('：')));

        if is_credit {
            continue;
        }
        if t.starts_with('[') && !body.is_empty() {
            timed += 1;
        }
        out.push(t.to_string());
    }

    if timed < 2 {
        None
    } else {
        Some(out.join("\n"))
    }
}

// ---------------------------------------------------------------- lrclib

#[derive(Deserialize)]
struct LrItem {
    #[serde(default, rename = "trackName")]
    track_name: String,
    #[serde(default, rename = "artistName")]
    artist_name: String,
    #[serde(default, rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(default, rename = "plainLyrics")]
    plain: Option<String>,
}

fn from_lrclib(title: &str, artist: &str, duration: f64) -> Option<LyricResult> {
    let mut url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        encode(title),
        encode(artist)
    );
    if duration > 1.0 {
        url.push_str(&format!("&duration={}", duration.round() as i64));
    }

    let body = http_get(&url, None)?;
    let items: Vec<LrItem> = serde_json::from_str(&body).ok()?;

    let exact = items.iter().find(|i| i.synced.is_some());
    let item = exact.or_else(|| items.first())?;
    let lrc = item
        .synced
        .clone()
        .or_else(|| item.plain.clone())
        .unwrap_or_default();
    if lrc.trim().is_empty() {
        return None;
    }

    Some(LyricResult {
        lrc,
        trans: String::new(),
        yrc: String::new(),
        source: "lrclib".into(),
        matched_title: if item.track_name.is_empty() {
            title.to_string()
        } else {
            item.track_name.clone()
        },
        matched_artist: item.artist_name.clone(),
    })
}

// ---------------------------------------------------------------- 工具

fn http_get(url: &str, referer: Option<&str>) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(9))
        .build();

    let mut req = agent
        .get(url)
        .set("User-Agent", UA)
        .set("Accept", "application/json, text/plain, */*")
        .set("Accept-Language", "zh-CN,zh;q=0.9");
    if let Some(r) = referer {
        req = req.set("Referer", r);
    }

    match req.call() {
        Ok(resp) => {
            let code = resp.status();
            let body = resp.into_string().ok();
            if code != 200 {
                crate::logbuf::warn(&format!("歌词接口 HTTP {code}：{url}"));
            }
            body
        }
        Err(err) => {
            // 网络失败在用户看来就是"这首没歌词"，日志里得能看出是网络还是没匹配到
            crate::logbuf::warn(&format!("歌词接口请求失败：{url} → {err}"));
            None
        }
    }
}

/// 归一化：小写、去掉括号内容和所有标点空白，只留下正文用于比对
fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' | '（' | '[' | '【' => {
                depth += 1;
                continue;
            }
            ')' | '）' | ']' | '】' => {
                depth -= 1;
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            continue;
        }
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
        }
    }
    out
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn cache_key(title: &str, artist: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in format!("{}\u{1}{}", norm(title), norm(artist)).as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:016x}")
}

fn cache_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = PathBuf::from(std::env::var("APPDATA").ok()?);
        // 和 settings.json 同一个应用目录，卸载时删一个文件夹就干净
        let dir = base.join("com.yuluo.lyricsfloat").join("lyrics");
        // 改名（歌词板 -> LyricsFloat）后缓存目录也换了：老目录还在就整体搬过来，
        // 用户不用重新联网拉一遍歌词。
        if !dir.exists() {
            let old = base.join("lyric-board").join("lyrics");
            if old.is_dir() {
                let _ = fs::create_dir_all(dir.parent().unwrap_or(&dir));
                let _ = fs::rename(&old, &dir);
            }
        }
        fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .clone()
}

fn read_cache(key: &str) -> Option<LyricResult> {
    let path = cache_dir()?.join(format!("{key}.json"));
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(key: &str, value: &LyricResult) {
    if let Some(dir) = cache_dir() {
        if let Ok(text) = serde_json::to_string(value) {
            // 词板窗口和设置窗口可能同时拉同一首歌，直接覆盖写会出现半截 JSON。
            // 先写临时文件再 rename，读到的永远是完整的一份。
            let tmp = dir.join(format!("{key}.json.tmp"));
            let dst = dir.join(format!("{key}.json"));
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(&tmp, &dst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netease_live() {
        let hit = from_netease("晴天", "周杰伦", 269.0);
        let r = hit.expect("网易云没有返回歌词");
        println!("source={} title={} artist={}", r.source, r.matched_title, r.matched_artist);
        println!("lrc lines={}", r.lrc.lines().count());
        println!("first 3: {:?}", r.lrc.lines().take(3).collect::<Vec<_>>());
        assert!(r.lrc.lines().count() > 8, "歌词行数太少");
        assert!(!r.lrc.contains("暂无歌词"), "命中了占位歌词");
        assert!(!r.lrc.contains("作词 :"), "制作人名单没被过滤");
    }

    #[test]
    fn lrclib_fallback() {
        let r = from_lrclib("Bohemian Rhapsody", "Queen", 355.0).expect("lrclib 没有返回");
        assert!(r.lrc.lines().count() > 8);
        println!("lrclib lines={}", r.lrc.lines().count());
    }

    #[test]
    fn sanitize_filters_placeholders() {
        assert!(sanitize("[00:00.00]暂无歌词").is_none());
        assert!(sanitize("[00:01.00]作词 : 某人\n[00:02.00]作曲 : 某人").is_none());
        let ok = sanitize("[00:01.00]作词 : 某人\n[00:12.00]第一句\n[00:15.00]第二句").unwrap();
        assert_eq!(ok.lines().count(), 2);
    }
}

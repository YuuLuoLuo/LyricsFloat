//! 歌词获取：多源聚合，统一打分后取最优，结果落盘缓存。
//!
//! 三个源：网易云、LRCLIB、lrc.cx（酷狗镜像，免 key）。
//!
//! 早先是"顺序短路"（谁先返回用谁）。问题在于先到的差结果会赢：网易云没有版权时
//! 搜出来的全是「搁浅（Cover）」「阿杰」这类条目，而 `norm()` 会把括号内容剥掉，
//! 于是蹭名条目拿到「歌名完全相同」的满分，永远轮不到 LRCLIB 里的原版。
//! 现在改成把候选全拉回来，用同一套规则打分再挑。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 匹配算法版本，参与缓存 key 的哈希。
/// 改了打分规则就 +1：用户不用手动清缓存，旧结果自动作废。
const MATCH_VERSION: u32 = 2;

const SRC_NETEASE: &str = "netease";
const SRC_LRCLIB: &str = "lrclib";
const SRC_LRCCX: &str = "lrccx";

/// 低于这个分不算"有效歌词"。宁可报没有，也别把营销号条目放上词板。
const MIN_SCORE: f64 = 30.0;

/// 达到这个分且带时间轴，就不再麻烦 lrc.cx（它平均要 1~2 秒）。
const GOOD_ENOUGH: f64 = 70.0;

fn source_label(id: &str) -> &'static str {
    match id {
        SRC_NETEASE => "网易云",
        SRC_LRCLIB => "LRCLIB",
        SRC_LRCCX => "lrc.cx",
        _ => "未知",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LyricResult {
    pub lrc: String,
    pub trans: String,
    pub yrc: String,
    /// 词源显示名（写日志和界面用）
    pub source: String,
    /// 词源标识：netease / lrclib / lrccx
    #[serde(default)]
    pub source_id: String,
    pub matched_title: String,
    pub matched_artist: String,
}

/// 一条候选，给设置页的「歌词源」板块展示用。
#[derive(Clone, Debug, Serialize)]
pub struct LyricCandidate {
    pub source: String,
    pub source_id: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub synced: bool,
    pub score: f64,
    /// 这条是不是最后真正用上的那条
    pub chosen: bool,
}

/// 最近一次取词的候选清单。设置页轮询它来画板块，省得自己再发一遍网络请求。
#[derive(Clone, Debug, Serialize)]
pub struct CandidateBoard {
    /// 这份清单对应的曲目（title\u{1}artist），前端比对后决定要不要显示
    pub track: String,
    pub prefer: String,
    pub items: Vec<LyricCandidate>,
}

static LAST: Mutex<Option<CandidateBoard>> = Mutex::new(None);

#[tauri::command]
pub fn last_candidates() -> Option<CandidateBoard> {
    LAST.lock().ok().and_then(|g| g.as_ref().cloned())
}

fn publish_candidates(
    title: &str,
    artist: &str,
    prefer: &str,
    items: &[LyricCandidate],
    chosen: Option<usize>,
) {
    let mut list = items.to_vec();
    for (i, c) in list.iter_mut().enumerate() {
        c.chosen = chosen == Some(i);
    }
    if let Ok(mut g) = LAST.lock() {
        *g = Some(CandidateBoard {
            track: track_key(title, artist),
            prefer: prefer.to_string(),
            items: list,
        });
    }
}

/// 前端也用这个拼串来核对"这份候选是不是当前这首"
pub fn track_key(title: &str, artist: &str) -> String {
    format!("{title}\u{1}{artist}")
}

#[tauri::command]
pub async fn fetch_lyric(
    title: String,
    artist: String,
    duration: f64,
    prefer: Option<String>,
) -> Result<Option<LyricResult>, String> {
    let prefer = prefer.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || fetch_sync(&title, &artist, duration, &prefer))
        .await
        .map_err(|e| e.to_string())
}

fn fetch_sync(title: &str, artist: &str, duration: f64, prefer: &str) -> Option<LyricResult> {
    let key = cache_key(title, artist, prefer);
    if let Some(hit) = read_cache(&key) {
        crate::logbuf::info(&format!(
            "歌词命中缓存：{title} - {artist}（{} → {} / {}）",
            hit.source, hit.matched_title, hit.matched_artist
        ));
        // 缓存里没有候选清单，补一份单条的把板块撑起来
        publish_candidates(
            title,
            artist,
            prefer,
            &[LyricCandidate {
                source: hit.source.clone(),
                source_id: hit.source_id.clone(),
                title: hit.matched_title.clone(),
                artist: hit.matched_artist.clone(),
                duration,
                synced: true,
                score: 0.0,
                chosen: false,
            }],
            Some(0),
        );
        return Some(hit);
    }

    let t0 = Instant::now();
    let (cands, handles) = collect(title, artist, duration, prefer);
    // 先发一份没有选中的：用户切到歌词页就能看到搜索结果，
    // 不用等下面那串"逐个试歌词"跑完。
    publish_candidates(title, artist, prefer, &cands, None);

    // 分数从高到低逐个试：最高分那条也可能挂着「暂无歌词」
    let mut order: Vec<usize> = (0..cands.len()).collect();
    order.sort_by(|a, b| {
        cands[*b]
            .score
            .partial_cmp(&cands[*a].score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for idx in order {
        if cands[idx].score < MIN_SCORE {
            break;
        }
        if let Some(r) = take_lyric(&handles[idx]) {
            crate::logbuf::info(&format!(
                "歌词选定：{title} - {artist} → {} / {}（{}，{} 分，耗时 {}ms）",
                r.matched_title,
                r.matched_artist,
                r.source,
                cands[idx].score as i64,
                t0.elapsed().as_millis()
            ));
            write_cache(&key, &r);
            publish_candidates(title, artist, prefer, &cands, Some(idx));
            return Some(r);
        }
        crate::logbuf::warn(&format!(
            "候选无有效歌词，退下一条：{} / {}（{}，{} 分）",
            cands[idx].title, cands[idx].artist, cands[idx].source, cands[idx].score as i64
        ));
    }

    crate::logbuf::warn(&format!(
        "歌词匹配失败：{title} - {artist}（{} 条候选全部无效，耗时 {}ms）",
        cands.len(),
        t0.elapsed().as_millis()
    ));
    None
}

// ---------------------------------------------------------------- 汇总

enum Handle {
    Netease {
        id: i64,
        title: String,
        artist: String,
    },
    Lrclib(LrItem),
    Lrccx {
        lrc: String,
        title: String,
        artist: String,
    },
}

fn collect(
    title: &str,
    artist: &str,
    duration: f64,
    prefer: &str,
) -> (Vec<LyricCandidate>, Vec<Handle>) {
    let mut cands = Vec::new();
    let mut handles = Vec::new();

    let want = |id: &str| prefer.is_empty() || prefer == id;

    if want(SRC_NETEASE) {
        let t0 = Instant::now();
        let got = netease_candidates(title, artist, duration);
        log_source(SRC_NETEASE, &got, t0);
        for (c, h) in got {
            cands.push(c);
            handles.push(h);
        }
    }

    if want(SRC_LRCLIB) {
        let t0 = Instant::now();
        let got = lrclib_candidates(title, artist, duration);
        log_source(SRC_LRCLIB, &got, t0);
        for (c, h) in got {
            cands.push(c);
            handles.push(h);
        }
    }

    // 自动模式下 lrc.cx 只做兜底：前两个源已经有靠谱结果就不查，
    // 它平均 1~2 秒，每次切歌都等它不划算。
    let usable = cands
        .iter()
        .any(|c| c.score >= GOOD_ENOUGH && c.synced);
    let want_cx = prefer == SRC_LRCCX || (prefer.is_empty() && !usable);

    if want_cx {
        let t0 = Instant::now();
        if let Some((c, h)) = lrccx_lookup(title, artist, duration) {
            crate::logbuf::info(&format!(
                "歌词源 lrc.cx：命中（{}ms）",
                t0.elapsed().as_millis()
            ));
            cands.push(c);
            handles.push(h);
        } else {
            crate::logbuf::info(&format!(
                "歌词源 lrc.cx：未命中（{}ms）",
                t0.elapsed().as_millis()
            ));
        }
    } else if prefer.is_empty() {
        crate::logbuf::info("歌词源 lrc.cx：前序已命中较好结果，跳过");
    }

    (cands, handles)
}

fn log_source(id: &str, got: &[(LyricCandidate, Handle)], t0: Instant) {
    let best = got
        .iter()
        .map(|(c, _)| c.score)
        .fold(f64::NEG_INFINITY, f64::max);
    if got.is_empty() {
        crate::logbuf::info(&format!(
            "歌词源 {}：0 条候选（{}ms）",
            source_label(id),
            t0.elapsed().as_millis()
        ));
        return;
    }
    let top = got
        .iter()
        .max_by(|a, b| {
            a.0.score
                .partial_cmp(&b.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(c, _)| c)
        .unwrap();
    crate::logbuf::info(&format!(
        "歌词源 {}：{} 条候选，最佳 {} 分（{} / {} / {}s / {}）（{}ms）",
        source_label(id),
        got.len(),
        best as i64,
        top.title,
        top.artist,
        top.duration.round() as i64,
        if top.synced { "有时间轴" } else { "无时间轴" },
        t0.elapsed().as_millis()
    ));
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

fn netease_candidates(
    title: &str,
    artist: &str,
    duration: f64,
) -> Vec<(LyricCandidate, Handle)> {
    let q = encode(&format!("{title} {artist}"));
    let search_url =
        format!("https://music.163.com/api/search/get/web?s={q}&type=1&offset=0&limit=20");
    let Some(body) = http_get(&search_url, Some("https://music.163.com/")) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<NeSearch>(&body) else {
        return Vec::new();
    };
    let Some(songs) = parsed.result.and_then(|r| r.songs) else {
        return Vec::new();
    };

    songs
        .iter()
        .map(|s| {
            let artists = if s.artists.is_empty() { &s.ar } else { &s.artists };
            let joined = artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join("/");
            let dur = s.duration / 1000.0;
            // 网易云的搜索接口不告诉我们"这首歌有没有歌词"，先按有处理：
            // 真取不到时 sanitize 会拒，take_lyric 再退下一条。
            let score = unified_score(title, artist, duration, &s.name, &joined, dur, true);
            (
                LyricCandidate {
                    source: source_label(SRC_NETEASE).into(),
                    source_id: SRC_NETEASE.into(),
                    title: s.name.clone(),
                    artist: joined.clone(),
                    duration: dur,
                    synced: true,
                    score,
                    chosen: false,
                },
                Handle::Netease {
                    id: s.id,
                    title: s.name.clone(),
                    artist: joined.clone(),
                },
            )
        })
        .collect()
}

fn netease_lyric(id: i64, title: &str, artist: &str) -> Option<LyricResult> {
    let lyric_url = format!(
        "https://music.163.com/api/song/lyric?id={id}&lv=-1&kv=-1&tv=-1&yv=-1&ytv=-1&yrv=-1"
    );
    let body = http_get(&lyric_url, Some("https://music.163.com/"))?;
    let parsed: NeLyric = serde_json::from_str(&body).ok()?;

    let raw_lrc = parsed.lrc.map(|x| x.lyric).unwrap_or_default();
    let raw_yrc = parsed.yrc.map(|x| x.lyric).unwrap_or_default();
    let trans = parsed.tlyric.map(|x| x.lyric).unwrap_or_default();

    let lrc = sanitize(&raw_lrc);
    let yrc = sanitize(&raw_yrc);
    if lrc.is_none() && yrc.is_none() {
        return None;
    }

    Some(LyricResult {
        lrc: lrc.unwrap_or_default(),
        trans,
        yrc: yrc.unwrap_or_default(),
        source: source_label(SRC_NETEASE).into(),
        source_id: SRC_NETEASE.into(),
        matched_title: title.to_string(),
        matched_artist: artist.to_string(),
    })
}

// ---------------------------------------------------------------- lrclib

#[derive(Clone, Debug, Deserialize)]
struct LrItem {
    #[serde(default, rename = "trackName")]
    track_name: String,
    #[serde(default, rename = "artistName")]
    artist_name: String,
    #[serde(default)]
    duration: f64,
    #[serde(default, rename = "syncedLyrics")]
    synced: Option<String>,
    #[serde(default, rename = "plainLyrics")]
    plain: Option<String>,
}

fn lrclib_candidates(
    title: &str,
    artist: &str,
    duration: f64,
) -> Vec<(LyricCandidate, Handle)> {
    // 注意：这里**不带** duration 参数。
    // lrclib 的 duration 是硬过滤，SMTC 报试听片段（60s）时会把整首正版全滤掉，
    // 一条都搜不出来 —— 时长该用来排序，不该用来筛。
    let mut url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        encode(title),
        encode(artist)
    );
    let mut items = lrclib_fetch(&url);

    // 歌手名写法差异很大（周杰伦 / Jay Chou / 周杰倫），精确查不到就退全文搜索
    if items.is_empty() {
        url = format!(
            "https://lrclib.net/api/search?q={}",
            encode(&format!("{title} {artist}"))
        );
        items = lrclib_fetch(&url);
    }

    items
        .into_iter()
        .filter(|i| i.synced.is_some() || i.plain.is_some())
        .map(|i| {
            let synced = i.synced.is_some();
            let score = unified_score(
                title,
                artist,
                duration,
                &i.track_name,
                &i.artist_name,
                i.duration,
                synced,
            );
            (
                LyricCandidate {
                    source: source_label(SRC_LRCLIB).into(),
                    source_id: SRC_LRCLIB.into(),
                    title: i.track_name.clone(),
                    artist: i.artist_name.clone(),
                    duration: i.duration,
                    synced,
                    score,
                    chosen: false,
                },
                Handle::Lrclib(i),
            )
        })
        .collect()
}

fn lrclib_fetch(url: &str) -> Vec<LrItem> {
    let Some(body) = http_get(url, None) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<LrItem>>(&body).unwrap_or_default()
}

fn lrclib_lyric(item: &LrItem) -> Option<LyricResult> {
    let raw = item.synced.clone().or_else(|| item.plain.clone())?;
    let lrc = sanitize(&raw)?;
    Some(LyricResult {
        lrc,
        trans: String::new(),
        yrc: String::new(),
        source: source_label(SRC_LRCLIB).into(),
        source_id: SRC_LRCLIB.into(),
        matched_title: item.track_name.clone(),
        matched_artist: item.artist_name.clone(),
    })
}

// ---------------------------------------------------------------- lrc.cx

/// lrc.cx 是 LrcApi 的公开实例，背后走酷狗曲库。
///
/// 好处是只要「歌名 + 歌手」，不吃时长，能捞到网易云/LRCLIB 都没有的中文歌；
/// 代价是个人维护的公开实例，说不准哪天限流或关停，所以只当兜底用。
/// 它返回的是纯文本 LRC，还带 `[Verse]` / `[Chorus]` 段落标签（sanitize 会丢掉）。
fn lrccx_lookup(
    title: &str,
    artist: &str,
    duration: f64,
) -> Option<(LyricCandidate, Handle)> {
    let url = format!(
        "https://api.lrc.cx/lyrics?title={}&artist={}",
        encode(title),
        encode(artist)
    );
    let body = http_get(&url, None)?;
    let text = body.trim();
    // 出错时会返回 JSON 或 HTML，直接当没找到
    if text.is_empty() || text.starts_with('{') || text.starts_with('<') {
        return None;
    }
    let lrc = sanitize(text)?;
    let synced = lrc.lines().any(is_time_tag);

    Some((
        LyricCandidate {
            source: source_label(SRC_LRCCX).into(),
            source_id: SRC_LRCCX.into(),
            title: title.to_string(),
            artist: artist.to_string(),
            duration,
            synced,
            // 它内部已经按歌名+歌手匹配过，但外面没法交叉验证，
            // 固定给 60：高于 MIN_SCORE 能用，低于 GOOD_ENOUGH 不抢自动模式的位。
            score: 60.0,
            chosen: false,
        },
        Handle::Lrccx {
            lrc,
            title: title.to_string(),
            artist: artist.to_string(),
        },
    ))
}

// ---------------------------------------------------------------- 取歌词

fn take_lyric(handle: &Handle) -> Option<LyricResult> {
    match handle {
        Handle::Netease { id, title, artist } => netease_lyric(*id, title, artist),
        Handle::Lrclib(item) => lrclib_lyric(item),
        Handle::Lrccx { lrc, title, artist } => Some(LyricResult {
            lrc: lrc.clone(),
            trans: String::new(),
            yrc: String::new(),
            source: source_label(SRC_LRCCX).into(),
            source_id: SRC_LRCCX.into(),
            matched_title: title.clone(),
            matched_artist: artist.clone(),
        }),
    }
}

// ---------------------------------------------------------------- 打分

/// 三个源共用的评分规则。
///
/// - 歌名完全相同 +60，互相包含 +34
/// - 歌手匹配 +26。**歌手完全不匹配时把总分压到 45** —— 网易云没版权时搜出来的
///   全是「XX（Cover）」「阿杰」这种蹭名条目，`norm()` 剥掉括号后它们能拿歌名满分，
///   只有歌手这一项能压住
/// - 时长只在双方都 ≥ 90 秒时参与：±2s +24、±6s +10、差超过 25s 扣 15。
///   SMTC 报试听片段（60s）时整个时长维度失效，不能拿它去扣正版的分
/// - 没有时间轴的候选 -40：桌面歌词要的是能对齐的 LRC，纯文本等于没有
fn unified_score(
    target_title: &str,
    target_artist: &str,
    target_dur: f64,
    cand_title: &str,
    cand_artist: &str,
    cand_dur: f64,
    synced: bool,
) -> f64 {
    let mut score = 0.0;

    let name = norm(cand_title);
    let target = norm(target_title);
    if !target.is_empty() && !name.is_empty() {
        if name == target {
            score += 60.0;
        } else if name.contains(&target) || target.contains(&name) {
            score += 34.0;
        }
    }

    let artist_ok = artist_hit(target_artist, cand_artist);
    if artist_ok {
        score += 26.0;
    }

    if target_dur >= 90.0 && cand_dur >= 90.0 {
        let diff = (cand_dur - target_dur).abs();
        if diff <= 2.0 {
            score += 24.0;
        } else if diff <= 6.0 {
            score += 10.0;
        } else if diff > 25.0 {
            score -= 15.0;
        }
    }

    if !synced {
        score -= 40.0;
    }

    if !artist_ok && score > 45.0 {
        score = 45.0;
    }
    score
}

fn artist_hit(target: &str, cand: &str) -> bool {
    let t = norm(target);
    let c = norm(cand);
    // 太短的串（"杰"、"Jay"）做包含判断会误伤，长度不够就当不匹配
    if t.chars().count() < 2 || c.chars().count() < 2 {
        return false;
    }
    c.contains(&t) || t.contains(&c)
}

/// 去掉制作人名单、过滤"暂无歌词"这类占位文本。
/// 只剩不到两句就认为没拿到真歌词。
fn sanitize(raw: &str) -> Option<String> {
    const PLACEHOLDER: [&str; 5] = [
        "暂无歌词",
        "纯音乐",
        "没有歌词",
        "此歌曲为没有填词",
        "instrumental",
    ];
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

        // `[Verse]` / `[Chorus]` 这种段落标签既不是歌词也不是时间行，整行丢掉。
        // lrc.cx（酷狗镜像）返回的 LRC 会带这些。
        if t.starts_with('[') && !is_time_tag(t) {
            continue;
        }

        let body = t.split(']').last().unwrap_or("").trim().to_string();

        let is_credit = CREDITS
            .iter()
            .any(|c| body.starts_with(c) && (body.contains(':') || body.contains('：')));

        if is_credit {
            continue;
        }
        if is_time_tag(t) && !body.is_empty() {
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

/// 是不是 `[mm:ss.xx]` 形式的时间标签
fn is_time_tag(t: &str) -> bool {
    let Some(rest) = t.strip_prefix('[') else {
        return false;
    };
    let Some(end) = rest.find(']') else {
        return false;
    };
    let inner = &rest[..end];
    let mut parts = inner.split(':');
    let (Some(m), Some(s)) = (parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let m_ok = !m.is_empty() && m.chars().all(|c| c.is_ascii_digit());
    let s_ok = !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
    m_ok && s_ok
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

/// 缓存 key 里带上词源偏好和匹配算法版本：
/// - 偏好：手动切到 LRCLIB 时不该读到自动模式缓存的网易云结果
/// - 版本：改了打分规则，旧结果自动作废，用户不用手动清缓存
fn cache_key(title: &str, artist: &str, prefer: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let seed = format!(
        "{}\u{1}{}\u{1}{}\u{1}v{}",
        norm(title),
        norm(artist),
        prefer,
        MATCH_VERSION
    );
    for b in seed.as_bytes() {
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

// ---------------------------------------------------------------- 清缓存

#[derive(Serialize)]
pub struct CacheClearResult {
    pub files: usize,
    pub bytes: u64,
    pub dir: String,
}

/// 清掉所有本地歌词缓存。下次播放会重新联网匹配。
#[tauri::command]
pub fn clear_lyric_cache() -> CacheClearResult {
    let mut out = CacheClearResult {
        files: 0,
        bytes: 0,
        dir: cache_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    let Some(dir) = cache_dir() else {
        return out;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        let is_cache = p
            .extension()
            .map(|x| x == "json" || x == "tmp")
            .unwrap_or(false);
        if !is_cache {
            continue;
        }
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        if fs::remove_file(&p).is_ok() {
            out.files += 1;
            out.bytes += size;
        }
    }
    crate::logbuf::info(&format!(
        "已清除歌词缓存：{} 个文件，{:.1} MB",
        out.files,
        out.bytes as f64 / 1048576.0
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单源便捷入口，只给下面几个 live 测试用
    fn best_of(mut cands: Vec<(LyricCandidate, Handle)>) -> Option<LyricResult> {
        cands.sort_by(|a, b| {
            b.0.score
                .partial_cmp(&a.0.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (_, h) in cands {
            if let Some(r) = take_lyric(&h) {
                return Some(r);
            }
        }
        None
    }

    #[test]
    fn netease_live() {
        let h = best_of(netease_candidates("晴天", "周杰伦", 269.0)).expect("网易云没有返回歌词");
        println!(
            "source={} title={} artist={}",
            h.source, h.matched_title, h.matched_artist
        );
        println!("lrc lines={}", h.lrc.lines().count());
        println!("first 3: {:?}", h.lrc.lines().take(3).collect::<Vec<_>>());
        assert!(h.lrc.lines().count() > 8, "歌词行数太少");
        assert!(!h.lrc.contains("暂无歌词"), "命中了占位歌词");
        assert!(!h.lrc.contains("作词 :"), "制作人名单没被过滤");
    }

    #[test]
    fn lrclib_live() {
        let h = best_of(lrclib_candidates(
            "Bohemian Rhapsody",
            "Queen",
            355.0,
        ))
        .expect("lrclib 没有返回");
        assert!(h.lrc.lines().count() > 8);
        println!("lrclib lines={}", h.lrc.lines().count());
    }

    /// 60 秒的试听片段不该把正版歌词筛掉
    #[test]
    fn lrclib_survives_short_duration() {
        let cands = lrclib_candidates("搁浅", "周杰伦", 60.0);
        assert!(!cands.is_empty(), "带 60s 时长把候选全滤掉了");
        let best = cands
            .iter()
            .map(|(c, _)| c.score)
            .fold(f64::NEG_INFINITY, f64::max);
        println!("60s 下的最佳分={best}，候选数={}", cands.len());
        assert!(best >= MIN_SCORE, "60s 下最高分不到阈值");
    }

    #[test]
    fn sanitize_filters_placeholders() {
        assert!(sanitize("[00:00.00]暂无歌词").is_none());
        assert!(sanitize("[00:01.00]作词 : 某人\n[00:02.00]作曲 : 某人").is_none());
        let ok = sanitize("[00:01.00]作词 : 某人\n[00:12.00]第一句\n[00:15.00]第二句").unwrap();
        assert_eq!(ok.lines().count(), 2);
    }

    /// lrc.cx 的 LRC 带 [Verse] / [Chorus]，这些行不能算歌词
    #[test]
    fn sanitize_drops_section_tags() {
        let raw = "[Verse]\n[00:16.16]久未放晴的天空\n[Chorus]\n[00:20.09]依舊留著妳的笑容";
        let ok = sanitize(raw).expect("整段被判成无效");
        assert_eq!(ok.lines().count(), 2);
        assert!(!ok.contains("[Verse]"));
        assert!(!ok.contains("[Chorus]"));
    }

    #[test]
    fn time_tag_detection() {
        assert!(is_time_tag("[00:16.162]"));
        assert!(is_time_tag("[1:02.30]"));
        assert!(is_time_tag("[00:12]"));
        assert!(!is_time_tag("[Verse]"));
        assert!(!is_time_tag("[ti:歌名]"));
        assert!(!is_time_tag("普通一行"));
    }

    /// 蹭名条目（歌名一样、歌手不对）必须输给歌手对得上的正规条目
    #[test]
    fn cover_entries_lose_to_original() {
        // 营销号：名字对，歌手不对，时长 72s（双方都不到 90，时长不参与）
        let junk = unified_score(
            "搁浅",
            "周杰伦",
            60.0,
            "搁浅(我只能永远读着对白)",
            "沈幼楚",
            72.0,
            true,
        );
        // 原版：名字、歌手、时长全对上
        let original = unified_score("搁浅", "周杰伦", 240.0, "搁浅", "周杰伦", 240.0, true);
        // 翻唱：名字对、时长接近，但歌手不对 —— 只能拿压过分，仍然可用
        let cover = unified_score("搁浅", "周杰伦", 240.0, "搁浅", "文太Vent.T", 244.0, true);
        println!("junk={junk} cover={cover} original={original}");
        assert!(junk <= 45.0, "蹭名条目分数过高：{junk}");
        assert!(cover <= 45.0, "歌手不对的条目没被压分：{cover}");
        assert!(original > junk, "原版没能赢过蹭名条目");
        assert!(original >= MIN_SCORE, "原版分数不到阈值");
    }

    /// 歌手对上了就不该再被压分
    #[test]
    fn matching_artist_not_capped() {
        let s = unified_score("搁浅", "周杰伦", 240.0, "搁浅", "周杰伦", 240.0, true);
        assert!(s > 100.0, "正常匹配分数异常：{s}");
    }

    #[test]
    fn cache_key_differs_by_prefer_and_version() {
        let a = cache_key("晴天", "周杰伦", "");
        let b = cache_key("晴天", "周杰伦", "lrclib");
        assert_ne!(a, b, "不同词源共用了同一个缓存键");
        assert_eq!(a.len(), 16);
    }

    /// 用户报的现场：QQ 音乐报 60 秒试听片段，结果选中了「沈幼楚」那版营销号歌词。
    /// 修完之后同一个输入不能再落到那条上。
    #[test]
    fn short_duration_avoids_keyword_bait() {
        let (cands, _handles) = collect("搁浅", "周杰伦", 60.0, "");
        assert!(!cands.is_empty(), "一条候选都没有");

        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by(|a, b| {
            cands[*b]
                .score
                .partial_cmp(&cands[*a].score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for &i in order.iter().take(5) {
            println!(
                "{:>4} 分 | {} | {} / {} / {}s",
                cands[i].score as i64,
                cands[i].source,
                cands[i].title,
                cands[i].artist,
                cands[i].duration.round() as i64
            );
        }
        let chosen = order
            .iter()
            .find(|i| cands[**i].score >= MIN_SCORE)
            .copied()
            .expect("没有任何候选过阈值");
        assert_ne!(
            cands[chosen].artist, "沈幼楚",
            "又选中了蹭名条目：{}",
            cands[chosen].title
        );
    }
}

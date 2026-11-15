/**
 * 播放状态与歌词调度。
 *
 * 播放状态来自系统 SMTC（Rust 侧轮询后 emit），歌词按「曲名+歌手+时长」
 * 到网易云 / lrclib 拉一次，之后进本地缓存。这里只做三件事：
 * 1. 收到 tick 就校准时钟
 * 2. 换歌就换歌词
 * 3. 找不到歌词时把原因写进 notice，别让词板空着不说话
 */

import { clock } from './clock'
import { runtime, settings, isTauriEnv } from './store'
import { fetchLyric, getNowPlaying, listenNative } from './bridge'
import { parseLyric, mergeTranslation } from './parser'
import { buildDemoLyric, DEMO_TRACK } from './demo'
import type { LyricDoc } from './types'

interface NowPlaying {
  title: string
  artist: string
  album: string
  duration: number
  position: number
  playing: boolean
}

let raw: { lrc: string; trans: string; yrc: string } | null = null
let loadToken = 0
let currentKey = ''

function buildDoc(): LyricDoc | null {
  if (!raw) return null
  const useYrc = raw.yrc.trim().length > 0
  const primary = useYrc ? raw.yrc : raw.lrc
  if (!primary.trim()) return null

  let doc = parseLyric(primary)
  // 逐字稿太少说明只是片段，退回普通 LRC
  if (useYrc && raw.lrc.trim()) {
    const plain = parseLyric(raw.lrc)
    if (doc.lines.length < plain.lines.length * 0.8) doc = plain
  }
  if (raw.trans.trim()) doc = mergeTranslation(doc, raw.trans)

  const off = settings.lyric.offsetMs
  if (off) {
    doc = {
      ...doc,
      lines: doc.lines.map((l) => ({
        ...l,
        start: Math.max(0, l.start + off),
        end: Math.max(0, l.end + off),
        words: l.words?.map((w) => ({
          ...w,
          start: Math.max(0, w.start + off),
          end: Math.max(0, w.end + off),
        })),
      })),
    }
  }
  return doc
}

export function reapplyOffset(): void {
  if (!raw) return
  runtime.doc = buildDoc()
}

async function loadLyric(np: NowPlaying): Promise<void> {
  const key = `${np.title}\u0001${np.artist}`
  if (key === currentKey) return
  currentKey = key
  // 换歌后必须丢掉上一首的进度基准，否则新歌起始进度恰好和旧值一样时
  // 会被当成"没变化"，时钟就留在上一首的位置
  lastRawMs = Number.NaN
  rawChangedAt = 0
  negStreak = 0
  moveStreak = 0
  pausedBase = Number.NaN
  const token = ++loadToken

  runtime.trackTitle = np.title
  runtime.trackArtist = np.artist
  runtime.trackAlbum = np.album
  runtime.notice = ''
  runtime.loading = true

  try {
    const payload = await fetchLyric(np.title, np.artist, np.duration)
    if (token !== loadToken) return

    if (!payload) {
      raw = null
      runtime.doc = null
      runtime.source = ''
      runtime.notice = `没有找到《${np.title}》的歌词`
      return
    }

    raw = { lrc: payload.lrc, trans: payload.trans, yrc: payload.yrc }
    runtime.doc = buildDoc()
    runtime.source = payload.source
    runtime.notice = runtime.doc ? '' : '歌词内容为空'
  } finally {
    if (token === loadToken) runtime.loading = false
  }
}

let lastRawMs = Number.NaN
let rawChangedAt = 0
let lastAdvanceAt = 0
/** 连续几次采样都说"我们比播放器超前" */
let negStreak = 0
/** 播放器报 Paused、进度却还在往前走的次数（见 applyTick 里的误报判断） */
let moveStreak = 0
/** 暂停期间上一次报上来的位置，用来分辨"重复采样"和"用户拖了进度条" */
let pausedBase = Number.NaN

/**
 * 用采样校准时钟。
 *
 * 正向（我们落后）照旧：超 400ms 直接跳，120~400ms 走 drift 平滑。
 *
 * 反向（我们超前）必须慎重，但也不能永远不理：以前一律忽略，
 * 于是"启动时锚在一个旧位置上"就再也回不来了 —— 因为之后两边都按 1× 走，
 * 差值恒定，永远等不到一次正向修正。现在让它连 3 次都说超前才回锚，
 * 单次滞后（timeline 落后于实际）仍然被忽略，不会重现"两句抽搐"。
 */
function calibrate(posMs: number, playing: boolean): void {
  const diff = posMs - clock.read()
  if (diff < -500) {
    if (diff < -8000 || ++negStreak >= 3) {
      clock.seek(posMs)
      negStreak = 0
    }
    return
  }
  negStreak = 0
  clock.sync(posMs)
}

function applyTick(np: NowPlaying): void {
  const now = performance.now()
  // SMTC 的 Position 单位是**秒**，歌词时间轴和时钟一律用**毫秒**。
  const rawMs = Math.max(0, np.position * 1000)

  // 播放器没刷新 timeline 时 position 会停住不动。这时候直接拿它校准，
  // 时钟就被按到"几秒前"的位置；而且之后两边都按 1× 往前走，
  // 这个偏移永远对不回来 —— 正是"偶尔不同步、且不会自己好"的来源。
  // 所以记下这个数值第一次出现的时刻，按经过的时间补足。
  const changed = rawMs !== lastRawMs
  if (changed) {
    const firstSeen = Number.isNaN(lastRawMs)
    lastRawMs = rawMs
    rawChangedAt = now
    if (!firstSeen) {
      lastAdvanceAt = now
      if (!np.playing) moveStreak++
    }
  }
  if (np.playing) moveStreak = 0

  // 以前用「2.5 秒内进度动过」推断在播 —— 播放器报暂停后那 2.5 秒时钟照样外推，
  // 歌词先滑到下一句，等推断结束时再回锚，看着就是"弹回去"。
  // 现在直接信播放器的暂停状态；只有那种播放中也报 Paused 的播放器
  // （位置却一直在前进）才在连续两次前进后重新认定在播。
  const misreport = !np.playing && moveStreak >= 2 && now - lastAdvanceAt < 2000
  const playing = np.playing || misreport
  const posMs = playing ? rawMs + (now - rawChangedAt) : rawMs

  // 先定"在不在播"，再校准：顺序反过来时，播放器误报 Paused 会让
  // 时钟走"暂停"分支，每 250ms 硬跳一次，歌词就来回抽搐。
  clock.setPlaying(playing)

  if (playing) {
    pausedBase = Number.NaN
    calibrate(posMs, true)
  } else {
    // 暂停：分辨"同一份采样又来一遍"和"位置真的变了"。
    // 前者是在重复报旧值，绝不能拿它把时钟往回拽；后者多半是用户拖了进度条。
    const repeated = !Number.isNaN(pausedBase) && Math.abs(posMs - pausedBase) <= 50
    const moved = !Number.isNaN(pausedBase) && !repeated
    pausedBase = posMs
    clock.syncPaused(posMs, moved)
    negStreak = 0
  }

  runtime.playing = np.playing
  runtime.position = np.position
  runtime.duration = np.duration
}

/** 演示模式：非 Tauri 环境（浏览器预览 / 调试）下自转 */
function startDemo(): void {
  raw = null
  runtime.doc = buildDemoLyric()
  runtime.trackTitle = DEMO_TRACK.title
  runtime.trackArtist = DEMO_TRACK.artist
  runtime.source = '演示'
  runtime.playing = true
  clock.reset(0)
  clock.setPlaying(true)
}

export async function initPlayer(): Promise<() => void> {
  if (!isTauriEnv()) {
    startDemo()
    return () => {}
  }

  const off = await listenNative({
    onTrackChanged: (p) => {
      void loadLyric(p as NowPlaying)
    },
    onTick: (p) => {
      applyTick(p as NowPlaying)
      // 兜底：如果因为事件丢失没换歌，这里补一次
      const np = p as NowPlaying
      if (`${np.title}\u0001${np.artist}` !== currentKey) void loadLyric(np)
    },
    onStopped: () => {
      currentKey = ''
      raw = null
      lastRawMs = Number.NaN
      rawChangedAt = 0
      lastAdvanceAt = 0
      negStreak = 0
      moveStreak = 0
      pausedBase = Number.NaN
      clock.setPlaying(false)
      runtime.doc = null
      runtime.playing = false
      runtime.trackTitle = ''
      runtime.trackArtist = ''
      runtime.source = ''
      runtime.notice = '暂无播放中的媒体'
    },
  })

  // 3 秒内没有任何播放器信息，给一句提示而不是空白
  setTimeout(() => {
    if (!runtime.doc && !currentKey && !runtime.notice) {
      runtime.notice = '在任意播放器里开始播放音乐'
    }
  }, 3000)

  // 事件只推变化：窗口刚起来时先拉一次快照，别等下一次换歌
  void getNowPlaying().then((np) => {
    if (!np) return
    applyTick(np)
    void loadLyric(np)
  })

  return off
}

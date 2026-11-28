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
import { fetchLyric, getNowPlaying, listenNative, getCover, logLine } from './bridge'
import { parseLyric, mergeTranslation } from './parser'
import { buildDemoLyric, DEMO_TRACK } from './demo'
import type { LyricDoc } from './types'

interface NowPlaying {
  app?: string
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
/** 上一次取词用的词源（空串 = 自动）。用户手动切源时靠它触发重取 */
let loadedPrefer = ''

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
  const prefer = settings.lyric.source
  const sameTrack = key === currentKey
  // 手动切换词源时，同一首歌也要重新取一次
  if (sameTrack && prefer === loadedPrefer) return
  /**
   * 只是换了词源：不重置封面、不置 loading。
   *
   * 换歌时那套"清空 + loading"是为了别让上一首的封面/歌词残留；
   * 但同一首歌换源时，旧歌词和封面都还是这首的，清掉反而让词板先空一下
   * 再填回来 —— 用户看到的就是"切个词源屏幕闪了一下"。
   */
  const soft = sameTrack
  currentKey = key
  loadedPrefer = prefer
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
  runtime.trackApp = np.app ?? ''
  if (!soft) {
    runtime.trackCover = ''
    runtime.coverColor = ''
    runtime.notice = ''
    runtime.loading = true
    void loadCover(token)
  }

  try {
    const payload = await fetchLyric(np.title, np.artist, np.duration, prefer)
    if (token !== loadToken) return

    if (!payload) {
      raw = null
      runtime.doc = null
      runtime.source = ''
      runtime.sourceId = ''
      // 手动指定了词源时把话说清楚：不是"没有歌词"，是这个源没有
      runtime.notice = prefer
        ? '当前词源无可用歌词'
        : `没有找到《${np.title}》的歌词`
      void logLine('WARN', `歌词为空：${np.title} - ${np.artist}（词源：${prefer || '自动'}）`)
      return
    }

    raw = { lrc: payload.lrc, trans: payload.trans, yrc: payload.yrc }
    runtime.doc = buildDoc()
    runtime.source = payload.source
    runtime.sourceId = payload.source_id || ''
    runtime.notice = runtime.doc ? '' : '歌词内容为空'
    void logLine(
      'INFO',
      `歌词已装载：${np.title}（${payload.source}，${runtime.doc?.lines.length ?? 0} 行）`,
    )
  } finally {
    if (token === loadToken) runtime.loading = false
  }
}

/**
 * 封面取不到时用的背景色（中灰）
 */
const NO_COVER_COLOR = '#808080'

/**
 * 从封面取一个适合做背景的主色。
 *
 * 缩到 24×24 再逐像素统计。跳过太暗、太亮和几乎无彩色的像素 ——
 * 封面常有黑边和白底，直接取平均会得到一片深灰/浅灰，看着像"取色没生效"。
 * 剩下的按饱和度加权平均；一个合格像素都没有（纯黑白封面）就退到中灰。
 */
function dominantColor(src: string): Promise<string> {
  return new Promise((resolve) => {
    const img = new Image()
    img.onload = () => {
      try {
        const n = 24
        const canvas = document.createElement('canvas')
        canvas.width = n
        canvas.height = n
        const ctx = canvas.getContext('2d')
        if (!ctx) return resolve(NO_COVER_COLOR)
        ctx.drawImage(img, 0, 0, n, n)
        const { data } = ctx.getImageData(0, 0, n, n)

        let r = 0
        let g = 0
        let b = 0
        let w = 0
        for (let i = 0; i < data.length; i += 4) {
          const R = data[i]
          const G = data[i + 1]
          const B = data[i + 2]
          const max = Math.max(R, G, B)
          const min = Math.min(R, G, B)
          const sat = max === 0 ? 0 : (max - min) / max
          const lum = (R * 299 + G * 587 + B * 114) / 1000
          if (lum < 24 || lum > 236 || sat < 0.12) continue
          // 饱和度高的权重更大；太亮太暗的再压一压
          const weight = sat * (1 - Math.abs(lum - 128) / 160)
          r += R * weight
          g += G * weight
          b += B * weight
          w += weight
        }
        if (w < 0.001) return resolve(NO_COVER_COLOR)
        const hex = (v: number) =>
          Math.max(0, Math.min(255, Math.round(v / w)))
            .toString(16)
            .padStart(2, '0')
        resolve(`#${hex(r)}${hex(g)}${hex(b)}`)
      } catch {
        resolve(NO_COVER_COLOR)
      }
    }
    img.onerror = () => resolve(NO_COVER_COLOR)
    img.src = src
  })
}

/**
 * 封面是 Rust 换歌时才去读一次流的，前端问到的时机可能比缓存早一点，
 * 所以取不到就隔一小会儿重试几次，别一直空着。
 */
async function loadCover(token: number): Promise<void> {
  for (let i = 0; i < 4; i++) {
    const url = await getCover()
    if (token !== loadToken) return
    if (url) {
      runtime.trackCover = url
      // 主色总是算一份：用户随时可能打开「从封面取色」，
      // 那时不该再等一次换歌。24×24 的采样，开销可以忽略。
      const color = await dominantColor(url)
      if (token === loadToken) runtime.coverColor = color
      return
    }
    await new Promise((r) => setTimeout(r, 700))
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
      // 兜底：如果因为事件丢失没换歌，这里补一次。
      // 词源被手动切换时，同一首歌也要重新取一次。
      const np = p as NowPlaying
      if (
        `${np.title}\u0001${np.artist}` !== currentKey ||
        settings.lyric.source !== loadedPrefer
      ) {
        void loadLyric(np)
      }
    },
    onStopped: () => {
      currentKey = ''
      loadedPrefer = ''
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
      runtime.trackApp = ''
      runtime.trackCover = ''
      runtime.coverColor = ''
      runtime.source = ''
      runtime.sourceId = ''
      runtime.notice = '暂无播放中的媒体'
    },
  })

  // 事件只推变化：窗口刚起来时先拉一次快照，别等下一次换歌
  void getNowPlaying().then((np) => {
    if (!np) return
    applyTick(np)
    void loadLyric(np)
  })

  return off
}

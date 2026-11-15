import type { LyricDoc, LyricLine, LyricWord } from './types'

const LINE_TIME_TAG = /\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g
const META_TAG = /^\[(ti|ar|al|by|offset|length):(.*)\]$/i
const YRC_LINE = /^\[(\d+),(\d+)\](.*)$/
const YRC_WORD = /\((\d+),(\d+),(-?\d+)\)([^()]*)/g

function toMs(min: string, sec: string, frac?: string): number {
  const ms = frac ? Number(frac.padEnd(3, '0').slice(0, 3)) : 0
  return Number(min) * 60000 + Number(sec) * 1000 + ms
}

/** 行歌词文本归一化：去掉首尾空白，压缩内部空白 */
function clean(text: string): string {
  return text.replace(/\s+/g, ' ').trim()
}

/** 给缺失 end 的行补齐结束时间 */
function fillEnds(lines: LyricLine[], totalDuration?: number): LyricLine[] {
  for (let i = 0; i < lines.length; i++) {
    const cur = lines[i]
    const next = lines[i + 1]
    if (!cur.end || cur.end <= cur.start) {
      cur.end = next ? next.start : cur.start + (totalDuration ? 0 : 4000)
    }
  }
  const last = lines[lines.length - 1]
  if (last && totalDuration && last.end < last.start) last.end = totalDuration
  return lines
}

/**
 * 解析标准 LRC / 增强 LRC。
 * 支持一行多个时间标签、元信息标签、offset。
 */
export function parseLrc(raw: string, totalDuration?: number): LyricDoc {
  const meta: LyricDoc['meta'] = {}
  const bucket: LyricLine[] = []
  let offset = 0

  const text = raw.replace(/^\uFEFF/, '')

  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line) continue

    const metaMatch = line.match(META_TAG)
    if (metaMatch) {
      const key = metaMatch[1].toLowerCase()
      const value = metaMatch[2].trim()
      if (key === 'ti') meta.title = value
      else if (key === 'ar') meta.artist = value
      else if (key === 'al') meta.album = value
      else if (key === 'offset') offset = Number(value) || 0
      continue
    }

    LINE_TIME_TAG.lastIndex = 0
    const stamps: number[] = []
    let m: RegExpExecArray | null
    while ((m = LINE_TIME_TAG.exec(line))) {
      stamps.push(toMs(m[1], m[2], m[3]))
    }
    if (!stamps.length) continue

    const content = clean(line.replace(LINE_TIME_TAG, ''))
    for (const start of stamps) {
      bucket.push({ start, end: 0, text: content })
    }
  }

  bucket.sort((a, b) => a.start - b.start)

  const adjusted = offset
    ? bucket.map((l) => ({
        ...l,
        start: Math.max(0, l.start - offset),
        end: Math.max(0, l.end - offset),
      }))
    : bucket

  return {
    lines: fillEnds(adjusted, totalDuration),
    meta,
    hasWordLevel: false,
  }
}

/**
 * 解析 YRC（网易云逐字歌词文本形式）。
 * 行： [行开始ms,行时长ms](词开始ms,词时长ms,0)词...
 */
export function parseYrc(raw: string, totalDuration?: number): LyricDoc {
  const lines: LyricLine[] = []
  let offset = 0

  for (const rawLine of raw.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line) continue

    const metaMatch = line.match(META_TAG)
    if (metaMatch) {
      if (metaMatch[1].toLowerCase() === 'offset') offset = Number(metaMatch[2]) || 0
      continue
    }

    const head = line.match(YRC_LINE)
    if (!head) continue

    const lineStart = Number(head[1])
    const lineDur = Number(head[2])
    const body = head[3]

    const words: LyricWord[] = []
    YRC_WORD.lastIndex = 0
    let w: RegExpExecArray | null
    while ((w = YRC_WORD.exec(body))) {
      const start = Number(w[1])
      const dur = Number(w[2])
      const txt = w[4]
      if (!txt) continue
      words.push({ start, end: start + dur, text: txt })
    }

    const plain = words.map((x) => x.text).join('')
    lines.push({
      start: lineStart,
      end: lineStart + lineDur,
      text: clean(plain),
      words: words.length ? words : undefined,
    })
  }

  lines.sort((a, b) => a.start - b.start)

  const adjusted = offset
    ? lines.map((l) => ({
        ...l,
        start: Math.max(0, l.start - offset),
        end: Math.max(0, l.end - offset),
        words: l.words?.map((w) => ({
          ...w,
          start: Math.max(0, w.start - offset),
          end: Math.max(0, w.end - offset),
        })),
      }))
    : lines

  return {
    lines: fillEnds(adjusted, totalDuration),
    meta: {},
    hasWordLevel: adjusted.some((l) => l.words?.length),
  }
}

/** 自动识别格式 */
export function parseLyric(raw: string, totalDuration?: number): LyricDoc {
  if (YRC_LINE.test(raw.trim().split(/\r?\n/).find((l) => l.trim()) ?? '')) {
    return parseYrc(raw, totalDuration)
  }
  return parseLrc(raw, totalDuration)
}

/**
 * 合并翻译（按起始时间就近匹配，容差 400ms）。
 */
export function mergeTranslation(doc: LyricDoc, transRaw: string): LyricDoc {
  const trans = parseLrc(transRaw)
  if (!trans.lines.length) return doc

  const byTime = new Map<number, string>()
  for (const t of trans.lines) {
    if (!t.text) continue
    byTime.set(t.start, t.text)
  }

  const lines = doc.lines.map((line) => {
    const exact = byTime.get(line.start)
    if (exact) return { ...line, trans: exact }
    let best: { diff: number; text: string } | null = null
    for (const t of trans.lines) {
      const diff = Math.abs(t.start - line.start)
      if (diff <= 400 && (!best || diff < best.diff)) best = { diff, text: t.text }
    }
    return best ? { ...line, trans: best.text } : line
  })

  return { ...doc, lines }
}

/**
 * 二分查找当前行索引。返回 -1 表示还没到第一句。
 */
export function findLineIndex(lines: LyricLine[], time: number): number {
  if (!lines.length) return -1
  if (time < lines[0].start) return -1
  let lo = 0
  let hi = lines.length - 1
  let ans = 0
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if (lines[mid].start <= time) {
      ans = mid
      lo = mid + 1
    } else {
      hi = mid - 1
    }
  }
  return ans
}

/** 当前行内进度 0~1 */
export function lineProgress(line: LyricLine | undefined, time: number): number {
  if (!line) return 0
  const span = Math.max(1, line.end - line.start)
  return Math.min(1, Math.max(0, (time - line.start) / span))
}

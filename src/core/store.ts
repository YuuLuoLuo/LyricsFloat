import { reactive } from 'vue'
import type { LyricDoc } from './types'

export interface AppSettings {
  window: {
    width: number
    height: number
  }
  appearance: {
    fontFamily: string
    fontSize: number
    fontWeight: number
    currentScale: number
    lineHeight: number
    blurStep: number
    blurMax: number
    fadeStep: number
    /** 水平对齐：只影响文本在行内的位置 */
    align: 'left' | 'center' | 'right'
    /** 垂直对齐：当前句贴顶，还是压在窗口正中 */
    vAlign: 'top' | 'center'
    /** 某句太长超出词板宽度时，自动把这一句缩到放得下 */
    autoFit: boolean
    shadow: number
    shadowColor: string
    /** 描边宽度（px），0 = 不描边 */
    strokeWidth: number
    strokeColor: string
    colorCurrent: string
    colorOther: string
    colorHighlight: string
  }
  /**
   * 歌词样式：四个开关互相独立，可以任意组合。
   * 多行滚动是所有样式的地基，不再作为可选项。
   */
  view: {
    /** 歌词旋转：以对齐那一侧为圆心，上下句模糊后叠在当前句下面 */
    rotate: boolean
    /** 歌词翻译：每句下面挂一行译文 */
    translation: boolean
    /** 逐字高亮：拿到逐字时间轴时按字点亮 */
    karaoke: boolean
  }
  glass: {
    tint: string
    alpha: number
  }
  behavior: {
    autostart: boolean
    alwaysOnTop: boolean
    locked: boolean
  }
  /** 歌词信息：词板上的曲名与封面 */
  info: {
    show: boolean
    /** 封面位置：左右两侧，或干脆不显示 */
    cover: 'left' | 'right' | 'hidden'
    /** 标题位置：词板顶部 / 底部，或不显示（纯歌词） */
    title: 'top' | 'bottom' | 'hidden'
  }
  lyric: {
    offsetMs: number
  }
}

export const DEFAULTS: AppSettings = {
  window: { width: 640, height: 230 },
  appearance: {
    fontFamily: 'Microsoft YaHei UI',
    fontSize: 30,
    fontWeight: 600,
    currentScale: 1.12,
    lineHeight: 54,
    blurStep: 0.7,
    blurMax: 2.2,
    fadeStep: 0.12,
    align: 'center',
    vAlign: 'center',
    autoFit: true,
    shadow: 0.55,
    shadowColor: '#000000',
    strokeWidth: 0,
    strokeColor: '#0a0a0c',
    colorCurrent: '#ffffff',
    colorOther: '#c9c9c4',
    colorHighlight: '#7fd4ff',
  },
  view: { rotate: false, translation: false, karaoke: false },
  glass: { tint: '#14141a', alpha: 0.42 },
  behavior: { autostart: false, alwaysOnTop: true, locked: false },
  info: { show: false, cover: 'left', title: 'top' },
  lyric: { offsetMs: 0 },
}

const KEY = 'lyricboard.settings.v3'

/**
 * 老版本用 `view.mode` 一个枚举选样式，现在拆成四个独立开关。
 * 读到旧结构就按原来的语义映射过来，存量配置不至于白丢：
 * 指针排布 -> 旋转，双语原文 -> 翻译，逐字 KTV / 双行卡农 -> 逐字高亮，
 * 卡片模式 -> 歌词信息。单行聚焦与纯文本是删掉的形式，退回默认（全关）。
 */
function migrate(raw: Record<string, unknown>): Record<string, unknown> {
  const view = raw.view as Record<string, unknown> | undefined
  if (!view || typeof view.mode !== 'string') return raw

  const mode = view.mode
  const card = (raw.card ?? {}) as Record<string, unknown>

  view.rotate = mode === 'pivot'
  view.translation = mode === 'bilingual' || view.showTranslation === true
  view.karaoke = mode === 'karaoke' || mode === 'duet'
  raw.info = {
    show: mode === 'card',
    cover: card.cover ?? 'left',
    title: card.title ?? 'top',
  }
  delete view.mode
  delete view.showTranslation
  delete raw.card
  return raw
}

/**
 * 按默认值的形状重建一份设置：
 * - 缺字段用默认值补（老版本存的设置没有新字段）
 * - 多出来的字段丢掉（radius / lines 这些已经删掉的项不该继续躺在文件里）
 */
function prune(base: unknown, val: unknown): unknown {
  if (!base || typeof base !== 'object' || Array.isArray(base)) {
    if (typeof val === typeof base) return val
    return base
  }
  const b = base as Record<string, unknown>
  const v = (val && typeof val === 'object' ? val : {}) as Record<string, unknown>
  const out: Record<string, unknown> = {}
  for (const k of Object.keys(b)) {
    out[k] = k in v ? prune(b[k], v[k]) : JSON.parse(JSON.stringify(b[k]))
  }
  return out
}

/** 外部来的设置（存盘 / 导入）统一走：迁移 -> 按形状重建 -> 收敛旧数值 */
function adopt(raw: unknown): AppSettings {
  const obj = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  return tame(prune(DEFAULTS, migrate(obj)) as AppSettings)
}

/**
 * 旧版本的默认值会把邻居行模糊/淡出到看不见（旧公式第三行只剩 0.04 不透明度）。
 * 存量设置里存着这些数值，光改 DEFAULTS 追不回来，读盘时统一收一下。
 */
function tame(s: AppSettings): AppSettings {
  const a = s.appearance
  a.blurStep = Math.max(0, Math.min(a.blurStep, 1.5))
  a.blurMax = Math.max(0, Math.min(a.blurMax, 2.5))
  a.fadeStep = Math.max(0.02, Math.min(a.fadeStep, 0.2))
  // 旋转样式没有"居中"这种画法：圆心必须落在某一侧，
  // 存量的 center（老版本允许）统一收到左边。
  if (s.view.rotate && a.align === 'center') a.align = 'left'
  return s
}

function load(): AppSettings {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return JSON.parse(JSON.stringify(DEFAULTS))
    return adopt(JSON.parse(raw))
  } catch {
    return JSON.parse(JSON.stringify(DEFAULTS))
  }
}

export const settings = reactive<AppSettings>(load())

let saving = false
let lastSerialized = ''

/**
 * 落盘钩子。store 只管内存与 localStorage，
 * exe 同目录的 json 由 persist.ts 注册进来写 —— 这样 store 不依赖 Tauri。
 */
let fileWriter: ((json: string) => void) | null = null

export function setFileWriter(fn: ((json: string) => void) | null): void {
  fileWriter = fn
}

/** 把外部设置（导入的备份 / 磁盘上的文件）合并进来，缺失字段用默认值补齐 */
export function adoptSettings(raw: unknown): boolean {
  if (!raw || typeof raw !== 'object') return false
  Object.assign(settings, adopt(raw))
  saveSettings()
  return true
}

/** 本地缓存里到底有没有存过设置（用来决定"要不要拿磁盘上的文件覆盖"） */
export function hasLocalSettings(): boolean {
  try {
    return localStorage.getItem(KEY) !== null
  } catch {
    return false
  }
}

export function settingsSnapshot(): string {
  return JSON.stringify(settings, null, 2)
}

export function saveSettings(): void {
  if (saving) return
  saving = true
  try {
    const text = JSON.stringify(settings)
    if (text !== lastSerialized) {
      lastSerialized = text
      localStorage.setItem(KEY, text)
      fileWriter?.(text)
    }
  } catch {
    /* 配额满等情况直接忽略，内存里的设置依然生效 */
  }
  saving = false
}

/** 应用另一窗口写入的设置（设置页改完 -> 词板立即跟着变） */
export function pullSettings(): void {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw || raw === lastSerialized) return
    lastSerialized = raw
    Object.assign(settings, adopt(JSON.parse(raw)))
  } catch {
    /* ignore */
  }
}

export function resetSettings(): void {
  Object.assign(settings, JSON.parse(JSON.stringify(DEFAULTS)))
  saveSettings()
}

export interface Runtime {
  locked: boolean
  revealed: boolean
  preview: boolean
  playing: boolean
  time: number
  /** SMTC 报上来的原始进度与总时长（秒），用于设置页显示"到底有没有在走" */
  position: number
  duration: number
  doc: LyricDoc | null
  loading: boolean
  trackTitle: string
  trackArtist: string
  trackAlbum: string
  source: string
  /** 没有播放器 / 找不到歌词时的提示 */
  notice: string
  /** 右键菜单是否展开（仅词板窗口） */
  menuOpen: boolean
}

export const runtime = reactive<Runtime>({
  locked: false,
  revealed: false,
  preview: false,
  playing: false,
  time: 0,
  position: 0,
  duration: 0,
  doc: null,
  loading: false,
  trackTitle: '',
  trackArtist: '',
  trackAlbum: '',
  source: '',
  notice: '',
  menuOpen: false,
})

/** 是否显示玻璃底。显形状态由 Rust 的悬停轮询决定（设置页打开时会强制显形） */
export function glassVisible(): boolean {
  return !runtime.locked && runtime.revealed
}

export function isTauriEnv(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

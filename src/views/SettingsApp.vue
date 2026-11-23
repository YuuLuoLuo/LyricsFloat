<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import LyricStage from '../components/LyricStage.vue'
import FontSelect from '../components/FontSelect.vue'
import { settings, runtime, saveSettings, resetSettings, pullSettings } from '../core/store'
import { clock } from '../core/clock'
import { findLineIndex } from '../core/parser'
import { initPlayer, reapplyOffset } from '../core/player'
import { exportSettingsFile, importSettingsFile, initSettingsFile } from '../core/persist'
import { ABOUT } from '../core/about'
import type { LyricLine } from '../core/types'
import type { PlayerInfo } from '../core/bridge'
import type { CandidateBoard } from '../core/bridge'
import {
  appVersion,
  checkUpdate,
  clearLyricCache,
  closeSettingsWindow,
  dataDir,
  exportLog,
  getAutostart,
  isTauri,
  lastCandidates,
  listFonts,
  listPlayers,
  minimizeSelf,
  onFileDrop,
  openDataDir,
  openUrl,
  peekBoard,
  quitApp,
  resetBoardPosition,
  setAlwaysOnTop,
  setAutostart,
  setLocked,
  setPlayerPin,
  setPlayerPref,
  settingsPickImport,
  startDragging,
} from '../core/bridge'

/** 常用字体（只显示系统里确实装了的），其余靠搜索 */
const CURATED = [
  'Microsoft YaHei UI',
  'Microsoft YaHei',
  'DengXian',
  'SimHei',
  'SimSun',
  'KaiTi',
  'FangSong',
  'HarmonyOS Sans SC',
  'Source Han Sans SC',
  'Noto Sans SC',
  'Segoe UI',
  'Arial',
  'Times New Roman',
  'Consolas',
]

const fonts = ref<string[]>([...CURATED])

const TABS = [
  { id: 'general', label: '常规' },
  { id: 'lyrics', label: '歌词' },
  { id: 'look', label: '外观' },
  { id: 'about', label: '关于' },
] as const

const tab = ref<(typeof TABS)[number]['id']>('general')

/* --------------------------------------------------------------- 预览 */

/**
 * 预览不放演示歌词 —— 只有真的拿到歌词时，才拿"当前这一句"来看效果。
 * 设置窗口自己也在跑播放器（同一个 SMTC 源、歌词走本地缓存），
 * 所以这里的时间轴和词板是同步的。
 *
 * 注意：词板在前奏/尾奏时会把"当前格"让空，但预览条只有 112px 高，
 * 让空会把这一句整个顶出去 —— 所以预览把时间夹在这一句之内，
 * 永远按"正在唱"的样子渲染。这里看的是排版，不是播放进度。
 */
/** 没在播放时预览用的示例文本：按当前样式排满三行，字体字号照样能看出来 */
const PREVIEW_TEXT = '这是一行示例文本'

const previewDoc = computed<{ lines: LyricLine[]; time: number } | null>(() => {
  const doc = runtime.doc
  const n = doc?.lines.length ?? 0
  if (!doc || !n) {
    const lines: LyricLine[] = [0, 1, 2].map((i) => ({
      start: i * 10_000,
      end: i * 10_000 + 8_000,
      text: PREVIEW_TEXT,
    }))
    // 时间落在中间那一行，三行里中间是"当前句"，上下两行是上下文
    return { lines, time: 11_000 }
  }
  const i = findLineIndex(doc.lines, runtime.time)
  // 预览要同时看到上下文：取一句当"当前句"，前后各带一行，共三行。
  // 首尾句没有邻居时往里收一格，保证永远有三行可看。
  let idx = i < 0 ? 0 : Math.min(i, n - 1)
  idx = Math.min(Math.max(idx, 1), Math.max(0, n - 2))
  const start = Math.max(0, Math.min(idx - 1, n - 3))
  const lines = doc.lines.slice(start, start + 3)
  const cur = doc.lines[idx]
  const lo = cur.start
  const hi = Math.max(lo, cur.end - 1)
  return { lines, time: Math.min(Math.max(runtime.time, lo), hi) }
})

/**
 * 预览的这三行里到底有没有译文。翻译开关开着但这首歌词没有译文时，
 * 预览不该空出译文那一行 —— 词板那边也是这么处理的（LyricStage 的 showTrans）。
 */
const previewHasTrans = computed(() =>
  (previewDoc.value?.lines ?? []).some((l) => (l.trans ?? '').trim() !== ''),
)

/**
 * 预览条高度跟着字号/行距走：要放下三行，不然第三行会被裁掉。
 * 预览只做排版参考，所以卡在 112~268px 之间，别让它把设置页挤没。
 */
const previewH = computed(() => {
  const ap = settings.appearance
  const unit = Math.max(ap.lineHeight, ap.fontSize * 1.25)
  const trans = settings.view.translation && previewHasTrans.value ? 1.5 : 1
  return Math.round(Math.min(268, Math.max(112, unit * trans * 3)))
})

let raf = 0
let last = 0
function tick(now: number) {
  const dt = last ? Math.min(64, now - last) : 16
  last = now
  clock.tick(dt)
  runtime.time = clock.read()
  raf = requestAnimationFrame(tick)
}

/**
 * 预览纸张跟着配色模板走：白字模板垫深色壁纸，黑字模板垫浅色壁纸。
 * 否则选了黑字，预览还压在深色底上，看着就是"一团看不见的字"。
 */
const previewPaper = computed(() => ({
  background:
    activePreset.value === 'dark'
      ? 'radial-gradient(680px 300px at 18% 0%, rgba(150,178,255,.55), transparent 62%),' +
        'radial-gradient(520px 260px at 88% 100%, rgba(255,196,180,.5), transparent 62%),' +
        'linear-gradient(150deg,#cfd8ef,#e6dcef 55%,#d7deec)'
      : 'radial-gradient(680px 300px at 18% 0%, rgba(96,124,246,.55), transparent 62%),' +
        'radial-gradient(520px 260px at 88% 100%, rgba(226,96,152,.4), transparent 62%),' +
        'linear-gradient(150deg,#1b2140,#2a2140 55%,#141a2c)',
}))

const previewGlass = computed(() => {
  const n = parseInt(settings.glass.tint.replace('#', '').padEnd(6, '0'), 16)
  const rgb = Number.isNaN(n)
    ? '20,20,26'
    : `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`
  return { background: `rgba(${rgb}, ${settings.glass.alpha})` }
})

const showTrans = computed(() => settings.view.translation && previewHasTrans.value)

/** 词板高度能放几行 —— 行数不再让用户手选，由高度和行距决定 */
const capacity = computed(() => {
  const head = settings.info.show ? 46 : 0
  const unit = Math.max(1, settings.appearance.lineHeight) * (showTrans.value ? 1.5 : 1)
  const usable = Math.max(0, settings.window.height - head)
  return Math.max(1, Math.floor(usable / unit))
})

/** 按样式开关决定哪些排版选项才有意义 */
const show = computed(() => ({
  // 旋转样式自带的模糊/淡出是画法的一部分，给滑块反而让人以为能调
  blur: !settings.view.rotate,
  fade: !settings.view.rotate,
  ktvColor: settings.view.karaoke,
  card: settings.info.show,
}))

/**
 * 水平对齐的可选项。旋转样式要拿一侧当圆心，没有"居中"这种画法，
 * 所以开着旋转时只留左 / 右。
 */
const H_ALIGNS = computed(() => {
  const all = [
    { k: 'left', label: '左' },
    { k: 'center', label: '中' },
    { k: 'right', label: '右' },
  ] as const
  return settings.view.rotate ? all.filter((it) => it.k !== 'center') : [...all]
})

/** 打开旋转时把居中的对齐收边 —— 居中没有圆心，画出来是绕中心扭 */
function toggleRotate(v: boolean): void {
  settings.view.rotate = v
  if (v && settings.appearance.align === 'center') settings.appearance.align = 'left'
}

/* --------------------------------------------------------------- 行为 */

const autostart = reactive({ value: false, busy: false })

const trackLine = computed(() => {
  if (runtime.trackTitle) {
    return runtime.trackArtist
      ? `${runtime.trackTitle} — ${runtime.trackArtist}`
      : runtime.trackTitle
  }
  return '未检测到正在播放的媒体'
})

const lyricState = computed(() => {
  if (runtime.doc) return `${runtime.doc.lines.length} 行`
  if (runtime.loading) return '正在获取歌词…'
  return runtime.notice || '—'
})

/** 曲库名字统一一下写法：lrclib 是开放曲库的英文名，小写摆着有点突兀 */
const SOURCE_LABEL: Record<string, string> = { lrclib: 'LRCLIB' }

/** 歌词源（只做标记，不给链接 —— 曲库页面没有稳定可跳转的地址） */
const lyricSource = computed(() => {
  if (!runtime.doc) return ''
  const s = runtime.source || ''
  return s ? (SOURCE_LABEL[s] ?? s) : '未知来源'
})

/* ------------------------------------------------------------- 歌词源 */

/** 三个曲库。id 与 Rust 侧一致；空串（自动）在模板里单独一个按钮 */
const SOURCES = [
  { id: 'netease', name: '网易云', note: '中文曲库大，无版权时只剩翻唱' },
  { id: 'lrclib', name: 'LRCLIB', note: '开放曲库，欧美与经典曲目最全' },
  { id: 'lrccx', name: 'lrc.cx', note: '酷狗镜像，中文冷门歌兜底' },
]

const board = ref<CandidateBoard | null>(null)

async function refreshBoard(): Promise<void> {
  if (!isTauri) return
  board.value = await lastCandidates()
}

/** 只显示当前这首歌的候选：Rust 那边切歌后会整份换掉清单 */
const candList = computed(() => {
  const b = board.value
  if (!b) return []
  if (b.track !== `${runtime.trackTitle}\u0001${runtime.trackArtist}`) return []
  return b.items
    .filter((c) => c.score >= 30 || c.chosen)
    .slice()
    .sort((a, b2) => b2.score - a.score)
    .slice(0, 8)
})

/**
 * 切换歌词曲库。设置存下就够了 —— 词板和设置页的播放循环
 * 每拍都会比对 `settings.lyric.source`，发现变了就自己重取这首。
 */
function pickSource(id: string): void {
  settings.lyric.source = id
  saveSettings()
}


function fmtTime(sec: number): string {
  const t = Math.max(0, Math.floor(sec))
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`
}

/** 带一位小数 —— 和下面的「时钟」用同一精度，两行才能直接对着看 */
function fmtTenth(sec: number): string {
  const t = Math.max(0, sec)
  const whole = Math.floor(t)
  const tenth = Math.floor((t - whole) * 10)
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, '0')}.${tenth}`
}

/** 直接把系统给的原始进度显示出来：进度不走时一眼能看出是播放器没给数 */
const progressText = computed(() => {
  if (!runtime.trackTitle) return '—'
  const pos = fmtTenth(runtime.position)
  if (!runtime.duration) return `${pos} · ${runtime.playing ? '播放中' : '已暂停'}`
  return `${pos} / ${fmtTime(runtime.duration)} · ${runtime.playing ? '播放中' : '已暂停'}`
})

/**
 * 歌词时钟读到的位置。它应该和上面「进度」几乎一致；
 * 若明显落后/超前，说明同步链路有问题，看一眼就能定位，不用再猜。
 */
const clockText = computed(() => {
  if (!runtime.trackTitle) return '—'
  return fmtTenth(runtime.time / 1000)
})

/* ----------------------------------------------------------- 配色模板 */

type PresetKey = 'light' | 'dark'

const PRESETS = {
  light: {
    colorCurrent: '#ffffff',
    colorOther: '#cfcfc8',
    colorHighlight: '#7fd4ff',
    strokeColor: '#0a0a0c',
    // 白字默认不描边：白字 + 深描边在深色壁纸上会显得字更细。
    // 需要时可以自己往上拉，描边是画在字形外面的，一定看得见。
    strokeWidth: 0,
    shadowColor: '#000000',
    shadow: 0.6,
  },
  dark: {
    colorCurrent: '#131317',
    colorOther: '#3a3a44',
    colorHighlight: '#1b5cff',
    // 黑字必须配白描边，否则深色壁纸上几乎不可读
    strokeColor: '#ffffff',
    strokeWidth: 2.4,
    shadowColor: '#ffffff',
    shadow: 0.28,
  },
} as const

const activePreset = computed<PresetKey | ''>(() => {
  const cur = settings.appearance.colorCurrent.toLowerCase()
  if (cur === PRESETS.light.colorCurrent) return 'light'
  if (cur === PRESETS.dark.colorCurrent) return 'dark'
  return ''
})

function applyPreset(key: PresetKey): void {
  Object.assign(settings.appearance, PRESETS[key])
}

/* ------------------------------------------------------------- 取色器 */

/**
 * 系统取色器由一个藏在窗口角落的 input 触发。
 * 以前把 input 绝对定位在色块里，聚焦时浏览器会把整页滚动过去，
 * 结果就是"打开调色板后设置页错位"。
 */
const palette = ref<HTMLInputElement | null>(null)
type ColorKey =
  | 'colorCurrent'
  | 'colorOther'
  | 'colorHighlight'
  | 'strokeColor'
  | 'shadowColor'
  | 'tint'
let paletteKey: ColorKey = 'colorCurrent'

function pickColor(key: ColorKey): void {
  paletteKey = key
  const el = palette.value
  if (!el) return
  el.value =
    key === 'tint' ? settings.glass.tint : settings.appearance[key as Exclude<ColorKey, 'tint'>]
  el.click()
}

function onPaletteInput(e: Event): void {
  const v = (e.target as HTMLInputElement).value
  if (paletteKey === 'tint') settings.glass.tint = v
  else settings.appearance[paletteKey] = v
}

async function toggleAutostart(v: boolean) {
  settings.behavior.autostart = v
  saveSettings()
  autostart.busy = true
  const res = isTauri ? await setAutostart(v) : true
  autostart.busy = false
  if (isTauri && res !== true) {
    autostart.value = !v
    settings.behavior.autostart = !v
  } else {
    autostart.value = v
  }
}

async function toggleAlwaysOnTop(v: boolean) {
  settings.behavior.alwaysOnTop = v
  saveSettings()
  void setAlwaysOnTop(v)
}

/* --------------------------------------------------------------- 播放器 */

/**
 * SMTC 里能读到播放信息的播放器列表。
 * 沙盒/浏览器预览里拿不到，此时只留「自动」一个选项。
 */
const players = ref<PlayerInfo[]>([])
let playerTimer: ReturnType<typeof setInterval> | null = null

async function refreshPlayers(): Promise<void> {
  if (!isTauri) return
  players.value = await listPlayers()
}

/** 当前歌词实际来自哪个播放器（读不到就直说） */
const playerLabel = computed(() => {
  if (runtime.trackApp) return settings.lyric.player ? `${runtime.trackApp}（已指定）` : runtime.trackApp
  return settings.lyric.player ? '该播放器当前没有在播' : '未检测到播放器'
})

function pickPlayer(id: string): void {
  settings.lyric.player = id
  saveSettings()
  void setPlayerPref(id)
}

/** 锁定后不再跟随"谁在播"：指定的播放器不在时词板就当作没有播放 */
function togglePin(v: boolean): void {
  settings.lyric.pinPlayer = v
  saveSettings()
  void setPlayerPin(v)
}

/** 锁定的播放器叫什么 —— 列表里没有（已退出）就退回系统给的 id */
const pinnedLabel = computed(() => {
  const id = settings.lyric.player
  if (!id) return '自动（未指定播放器）'
  return players.value.find((p) => p.id === id)?.name ?? id
})

/**
 * 「无播放时关闭词板」= 词板可能整个不在，尺寸/背景这些项就没意义了。
 *
 * 但只有"词板真的不在"时才该禁用：开关开着、此刻又有歌词在放，词板是在屏幕上的，
 * 调尺寸和背景当场就能看到效果，没理由拦着用户。
 * 判据跟 App.vue 里 syncPark 停放词板用的是同一个：`hideWhenIdle && !runtime.doc`。
 */
const parkMode = computed(() => settings.behavior.hideWhenIdle && !runtime.doc)

function toggleHideWhenIdle(v: boolean) {
  settings.behavior.hideWhenIdle = v
  saveSettings()
}

async function doExportLog(): Promise<void> {
  if (!isTauri) {
    say('浏览器预览里没有日志')
    return
  }
  const path = await exportLog()
  say(path ? `已导出：${path}` : '导出失败，请重试')
}

/** 清掉本地歌词缓存：匹配错过的歌、想换源重试时用 */
async function doClearCache(): Promise<void> {
  if (!isTauri) {
    say('浏览器预览里没有缓存')
    return
  }
  const r = await clearLyricCache()
  if (!r) {
    say('清除失败，请重试')
    return
  }
  if (!r.files) {
    say('缓存已经是空的')
    return
  }
  const mb = (r.bytes / 1048576).toFixed(1)
  say(`已清除 ${r.files} 个歌词缓存（${mb} MB）`)
}

function toggleLocked(v: boolean) {
  settings.behavior.locked = v
  saveSettings()
  void setLocked(v)
}

/** 词板被拖到屏幕外面时唯一的救回手段：挪回主屏顶部居中 */
function resetPos() {
  void resetBoardPosition()
}

function close() {
  void closeSettingsWindow()
}

function quit() {
  void quitApp()
}

async function onTitleDown(e: PointerEvent) {
  if (e.button !== 0) return
  const el = e.target as HTMLElement
  if (el.closest('button')) return
  await startDragging(e)
}

/* --------------------------------------------------------- 配置导入导出 */

const dataPath = ref('')
const toast = ref('')
let toastTimer: ReturnType<typeof setTimeout> | null = null

function say(msg: string): void {
  toast.value = msg
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = ''
  }, 4200)
}

/** 在资源管理器里打开数据目录 */
function openDataFolder(): void {
  void openDataDir().then((ok) => {
    if (!ok) say('打不开数据文件夹')
  })
}

async function doExport(): Promise<void> {
  const p = await exportSettingsFile()
  // 空串 = 用户在另存为里取消了，不是失败，不用弹提示
  if (p) say(`已导出到 ${p}`)
}

/** 点「导入设置文件」：弹系统的打开对话框 */
async function doPickImport(): Promise<void> {
  const file = await settingsPickImport()
  if (!file) return
  await doImport(file)
}

async function doImport(file: string): Promise<void> {
  const ok = await importSettingsFile(file)
  if (ok) {
    saveSettings()
    say('设置已导入')
  } else {
    say('导入失败：不是有效的设置文件')
  }
}

let offDrop: (() => void) | null = null

/* --------------------------------------------------------------- 关于 */

const version = ref('')
const repoUrl = computed(() => (ABOUT.repo ? `https://github.com/${ABOUT.repo}` : ''))

interface UpdateState {
  kind: 'idle' | 'latest' | 'new' | 'error'
  text: string
  url?: string
  notes?: string
  /** Release 对应的版本号，用来判断这份日志是不是"当前这版"的 */
  version?: string
}

const updateState = ref<UpdateState>({ kind: 'idle', text: '' })
const checking = ref(false)

/**
 * 极简 Markdown 渲染 —— 只覆盖更新日志里会出现的几种：
 * 标题、无序列表、粗体、行内代码、分隔线。
 *
 * 不为它引第三方库：正文就十几行，为这点东西加一个几十 KB 的依赖不划算。
 * 先整体转义 HTML 再替换标记，Release 正文里夹带的标签不会被当成 DOM 执行。
 */
function renderNotes(md: string): string {
  const esc = (s: string) =>
    s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
  const inline = (s: string) =>
    esc(s)
      .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
      .replace(/`([^`]+)`/g, '<code>$1</code>')

  const html: string[] = []
  let list: string[] = []
  const flush = () => {
    if (!list.length) return
    html.push(`<ul>${list.map((x) => `<li>${x}</li>`).join('')}</ul>`)
    list = []
  }

  for (const raw of md.split(/\r?\n/)) {
    const t = raw.trim()
    // 空行和 --- 分隔线都不占版面
    if (!t || /^([-*_])\1{2,}$/.test(t)) {
      flush()
      continue
    }
    const head = /^(#{1,6})\s+(.*)$/.exec(t)
    if (head) {
      flush()
      html.push(`<h4>${inline(head[2])}</h4>`)
      continue
    }
    const item = /^[-*+]\s+(.*)$/.exec(t)
    if (item) {
      list.push(inline(item[1]))
      continue
    }
    flush()
    html.push(`<p>${inline(t)}</p>`)
  }
  flush()
  return html.join('')
}

const notesHtml = computed(() => renderNotes(updateState.value.notes || ''))

/** 日志标题：分清楚这份是"你要升到的新版"还是"你现在这版"的 */
const notesTitle = computed(() => {
  const s = updateState.value
  if (!s.notes) return ''
  if (s.kind === 'new') return `更新内容（${s.version}）`
  if (s.version && version.value && s.version === version.value) return '本次更新内容'
  return `发布版 ${s.version} 的更新内容`
})

async function doCheckUpdate(): Promise<void> {
  if (!ABOUT.repo) {
    say('仓库发布后才能检查更新')
    return
  }
  if (checking.value) return
  checking.value = true
  updateState.value = { kind: 'idle', text: '正在检查…' }

  const { info, error } = await checkUpdate(ABOUT.repo)
  checking.value = false

  if (error || !info) {
    updateState.value = { kind: 'error', text: error || '没有读到版本信息' }
    return
  }
  // 已是最新时也把日志带出来：用户点"检查更新"就是想看这次改了什么，
  // 只回一句"已是最新"等于什么都没说。
  updateState.value = {
    kind: info.newer ? 'new' : 'latest',
    text: info.newer
      ? `发现新版本 ${info.latest}${info.published ? ` · ${info.published}` : ''}`
      : `已是最新版本（${info.latest}）`,
    url: info.newer ? info.url : undefined,
    notes: info.notes,
    version: info.latest,
  }
}

async function openLink(url: string): Promise<void> {
  if (!url) return
  if (isTauri) await openUrl(url)
  else window.open(url, '_blank')
}

/* --------------------------------------------------------------- 持久化 */

let saveTimer: ReturnType<typeof setTimeout> | null = null

// 时间偏移改了要重建本窗口的歌词时间轴，否则预览和词板会差一截
watch(
  () => settings.lyric.offsetMs,
  () => reapplyOffset(),
)

/**
 * 调宽高时让词板临时显形。
 *
 * 词板静止态是全透明的（只剩字），改尺寸的时候完全看不出面板边界，
 * 只能等松手把鼠标移上去才知道改成了什么样。所以拖动滑块期间
 * 让它按"悬停"的样子亮起来，停手 1.4 秒后自动收起。
 */
const PEEK_MS = 1400
watch(
  () => [settings.window.width, settings.window.height],
  () => {
    if (isTauri) void peekBoard(PEEK_MS)
  },
)

// 播放器列表只在「歌词」页可见时轮询，别的页面不白烧定时器
watch(
  tab,
  (v) => {
    if (playerTimer) {
      clearInterval(playerTimer)
      playerTimer = null
    }
    if (v !== 'lyrics') return
    void refreshPlayers()
    void refreshBoard()
    playerTimer = setInterval(() => {
      void refreshPlayers()
      void refreshBoard()
    }, 2000)
  },
  { immediate: true },
)

watch(
  settings,
  () => {
    if (saveTimer) clearTimeout(saveTimer)
    saveTimer = setTimeout(() => saveSettings(), 90)
  },
  { deep: true },
)

let stopPlayer: (() => void) | null = null
let syncTimer: ReturnType<typeof setInterval> | null = null
let onStorage: (() => void) | null = null

onMounted(async () => {
  raf = requestAnimationFrame(tick)
  if (isTauri) {
    const v = await getAutostart()
    if (typeof v === 'boolean') autostart.value = v
    version.value = (await appVersion()) || ''
  } else {
    autostart.value = settings.behavior.autostart
  }

  // 设置窗口自己接一路播放器（SMTC + 本地歌词缓存）：
  // 预览要显示"当前这一句"，就不能再拿演示歌词凑数。
  stopPlayer = await initPlayer()

  // 词板那边也可能改设置（托盘切换锁定），保持两边一致
  onStorage = () => pullSettings()
  window.addEventListener('storage', onStorage)
  syncTimer = setInterval(pullSettings, 900)

  // 播放来源偏好推给 Rust（词板窗口也会推，两边都推才不会互相覆盖）
  await setPlayerPref(settings.lyric.player)
  await setPlayerPin(settings.lyric.pinPlayer)
  await refreshPlayers()

  // 设置文件 + 系统字体列表
  await initSettingsFile()
  dataPath.value = (await dataDir()) || ''
  const list = await listFonts()
  if (list && list.length) {
    const merged = new Set<string>(list)
    // 当前选中的字体即使系统没报出来也别让它从列表里消失
    merged.add(settings.appearance.fontFamily)
    fonts.value = [...merged].sort((a, b) => a.localeCompare(b, 'zh-Hans-CN'))
  }

  // 拖一个 json 进来就能导入
  offDrop = await onFileDrop((paths) => {
    const f = paths.find((p) => p.toLowerCase().endsWith('.json'))
    if (!f) {
      say('请拖入 .json 设置文件')
      return
    }
    void doImport(f)
  })
})

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  if (saveTimer) clearTimeout(saveTimer)
  if (syncTimer) clearInterval(syncTimer)
  if (toastTimer) clearTimeout(toastTimer)
  stopPlayer?.()
  offDrop?.()
  if (playerTimer) clearInterval(playerTimer)
  if (onStorage) window.removeEventListener('storage', onStorage)
  saveSettings()
})
</script>

<template>
  <div class="win">
    <input ref="palette" class="palette" type="color" @input="onPaletteInput" />
    <header class="titlebar" @pointerdown="onTitleDown" @dblclick="minimizeSelf">
      <div class="brand">
        <img src="/icon.svg" alt="" class="logo" />
        <span>LyricsFloat 设置</span>
      </div>
      <div class="wctl">
        <button title="最小化" @click.stop="minimizeSelf">
          <svg viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.7" fill="none">
            <path d="M6 12h12" />
          </svg>
        </button>
        <button class="close" title="关闭" @click.stop="close">
          <svg viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.7" fill="none">
            <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />
          </svg>
        </button>
      </div>
    </header>

    <div class="main">
      <nav class="nav">
        <button
          v-for="t in TABS"
          :key="t.id"
          :class="{ on: tab === t.id }"
          @click="tab = t.id"
        >
          {{ t.label }}
        </button>
        <div class="nav-spacer"></div>
        <div class="now" :title="trackLine">
          <div class="dot" :class="{ live: runtime.playing }"></div>
          <div class="now-text">{{ trackLine }}</div>
        </div>
      </nav>

      <div class="body">
        <!-- 预览只在「外观」页出现：其它页没有样式可调，占着位置反而干扰 -->
        <div v-if="tab === 'look'" class="preview">
          <div class="paper" :style="[previewPaper, { height: `${previewH}px` }]">
            <div class="glasslayer" :style="previewGlass"></div>
            <LyricStage
              v-if="previewDoc"
              :lines="previewDoc.lines"
              :time="previewDoc.time"
              :height="previewH"
            />
            <div v-else class="paper-empty">{{ runtime.notice || '等待播放器' }}</div>
          </div>
        </div>

        <div class="scroll">
          <!-- ---------------------------------------------------- 常规 -->
          <section v-show="tab === 'general'">
            <h3>行为</h3>
            <div class="swrow">
              <label class="switch">
                <span>开机自动启动</span>
                <input
                  type="checkbox"
                  :checked="autostart.value"
                  :disabled="autostart.busy"
                  @change="toggleAutostart(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
              <label class="switch">
                <span>总在最前</span>
                <input
                  type="checkbox"
                  :checked="settings.behavior.alwaysOnTop"
                  @change="toggleAlwaysOnTop(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
            </div>
            <div class="swrow">
              <label class="switch">
                <span>锁定词板</span>
                <input
                  type="checkbox"
                  :checked="settings.behavior.locked"
                  @change="toggleLocked(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
              <label class="switch">
                <span>无播放时关闭词板</span>
                <input
                  type="checkbox"
                  :checked="settings.behavior.hideWhenIdle"
                  @change="toggleHideWhenIdle(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
            </div>
            <p v-if="settings.behavior.hideWhenIdle" class="hint">
              没有播放、或没有歌词时词板整个关闭。
            </p>
            <div class="row">
              <span class="lbl">词板位置</span>
              <div class="grow"></div>
              <button class="mini" @click="resetPos">移回屏幕中央</button>
            </div>

            <h3>设置项</h3>
            <div class="row">
              <span class="lbl">数据文件夹</span>
              <div class="grow path mono" :title="dataPath">
                {{ dataPath || '（浏览器预览无目录）' }}
              </div>
              <button class="mini" @click="openDataFolder">打开</button>
            </div>
            <p class="hint">设置、歌词缓存和日志都在这里。也可以把导出的设置文件拖进窗口导入。</p>
            <div class="danger-zone">
              <button class="ghost" @click="doExport">导出设置文件</button>
              <button class="ghost" @click="doPickImport">导入设置文件</button>
              <button class="ghost" @click="resetSettings">恢复默认</button>
            </div>

            <h3>歌词缓存</h3>
            <p class="hint">搜到的歌词存在本机，同一首歌下次不再联网。清掉后重新匹配。</p>
            <div class="danger-zone">
              <button class="ghost" @click="doClearCache">清除歌词缓存</button>
            </div>

            <h3>日志</h3>
            <p class="hint">
              导出到软件所在目录，内容含版本、系统、窗口与设置、播放器与歌词匹配过程、异常记录。
            </p>
            <div class="danger-zone">
              <button class="ghost" @click="doExportLog">导出日志</button>
            </div>

            <!-- 退出单独放在最底下，和"恢复默认"这类不退出进程的按钮分开 -->
            <div class="quitrow">
              <button class="danger" @click="quit">退出 LyricsFloat</button>
            </div>
          </section>

          <!-- ---------------------------------------------------- 歌词 -->
          <section v-show="tab === 'lyrics'">
            <h3>播放器</h3>
            <div class="row">
              <span class="lbl">当前播放器</span>
              <div class="grow muted">{{ playerLabel }}</div>
            </div>
            <div class="row">
              <span class="lbl">播放来源</span>
              <div class="chips grow" :class="{ off: settings.lyric.pinPlayer }">
                <button
                  class="chip"
                  :class="{ on: !settings.lyric.player }"
                  :disabled="settings.lyric.pinPlayer"
                  @click="pickPlayer('')"
                >
                  自动
                </button>
                <button
                  v-for="p in players"
                  :key="p.id"
                  class="chip"
                  :class="{ on: settings.lyric.player === p.id }"
                  :title="p.id"
                  :disabled="settings.lyric.pinPlayer"
                  @click="pickPlayer(p.id)"
                >
                  {{ p.name }}
                  <span v-if="p.playing" class="chip-dot"></span>
                </button>
              </div>
            </div>
            <p v-if="!players.length" class="hint">还没有读到播放器</p>
            <div class="swrow single">
              <label class="switch">
                <span>锁定已选择的播放器</span>
                <input
                  type="checkbox"
                  :checked="settings.lyric.pinPlayer"
                  @change="togglePin(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
            </div>
            <p v-if="settings.lyric.pinPlayer" class="hint">当前已锁定：{{ pinnedLabel }}</p>

            <h3>当前播放</h3>
            <div class="row">
              <span class="lbl">曲目</span>
              <div class="grow muted">{{ trackLine }}</div>
            </div>
            <div class="row">
              <span class="lbl">歌词</span>
              <div class="grow muted">{{ lyricState }}</div>
            </div>
            <div class="row">
              <span class="lbl">进度</span>
              <div class="grow muted mono">{{ progressText }}</div>
            </div>
            <div class="row">
              <span class="lbl">时钟</span>
              <div class="grow muted mono">{{ clockText }}</div>
            </div>

            <h3>歌词源</h3>
            <div class="row">
              <span class="lbl">当前</span>
              <div class="grow">
                <span v-if="lyricSource" class="srcbadge">{{ lyricSource }}</span>
                <span v-else-if="runtime.loading" class="muted">正在获取…</span>
                <span v-else class="muted">{{ runtime.notice || '无可用歌词' }}</span>
              </div>
            </div>
            <div class="chips srcpick">
              <button class="chip" :class="{ on: !settings.lyric.source }" @click="pickSource('')">
                自动
              </button>
              <button
                v-for="s in SOURCES"
                :key="s.id"
                class="chip"
                :class="{ on: settings.lyric.source === s.id }"
                :title="s.note"
                @click="pickSource(s.id)"
              >
                {{ s.name }}
              </button>
            </div>
            <p v-if="settings.lyric.source && !runtime.loading && !runtime.doc" class="hint">
              当前词源无可用歌词
            </p>

            <div v-if="candList.length" class="candbox">
              <div v-for="(c, i) in candList" :key="i" class="cand" :class="{ cur: c.chosen }">
                <span class="csrc">{{ c.source }}</span>
                <span class="cname" :title="c.title">{{ c.title || '未知曲目' }}</span>
                <span class="cart" :title="c.artist">{{ c.artist || '未知歌手' }}</span>
                <span class="cdur">{{ c.duration > 1 ? fmtTime(c.duration) : '—' }}</span>
                <span class="ctag" :class="{ off: !c.synced }">
                  {{ c.synced ? '时间轴' : '纯文本' }}
                </span>
              </div>
            </div>
            <p v-else class="hint">还没有搜索结果。</p>
            <p v-if="lyricSource" class="hint">
              歌词来自第三方曲库，可能存在错字、缺句或时间轴偏差；整体快了慢了可用下方「时间偏移」微调。
            </p>

            <h3>同步</h3>
            <div class="row">
              <span class="lbl">时间偏移</span>
              <input
                v-model.number="settings.lyric.offsetMs"
                class="range"
                type="range"
                min="-3000"
                max="3000"
                step="50"
              />
              <span class="val">{{ settings.lyric.offsetMs }} ms</span>
            </div>
          </section>

          <!-- ---------------------------------------------------- 外观 -->
          <section v-show="tab === 'look'">
            <h3>歌词样式</h3>
            <div class="swrow">
              <label class="switch">
                <span>歌词旋转</span>
                <input
                  type="checkbox"
                  :checked="settings.view.rotate"
                  @change="toggleRotate(($event.target as HTMLInputElement).checked)"
                />
                <i></i>
              </label>
              <label class="switch">
                <span>歌词翻译</span>
                <input v-model="settings.view.translation" type="checkbox" />
                <i></i>
              </label>
            </div>
            <div class="swrow">
              <label class="switch">
                <span>逐字高亮</span>
                <input v-model="settings.view.karaoke" type="checkbox" />
                <i></i>
              </label>
              <label class="switch">
                <span>显示歌词信息</span>
                <input v-model="settings.info.show" type="checkbox" />
                <i></i>
              </label>
            </div>

            <template v-if="show.card">
              <h3>歌词信息布局</h3>
              <div class="row">
                <span class="lbl">位置</span>
                <div class="seg grow">
                  <button
                    :class="{ on: settings.info.pos === 'top' }"
                    @click="settings.info.pos = 'top'"
                  >
                    顶部
                  </button>
                  <button
                    :class="{ on: settings.info.pos === 'bottom' }"
                    @click="settings.info.pos = 'bottom'"
                  >
                    底部
                  </button>
                </div>
                <span class="val"></span>
              </div>
              <div class="row">
                <span class="lbl">封面</span>
                <div class="seg grow">
                  <button
                    :class="{ on: settings.info.cover === 'left' }"
                    @click="settings.info.cover = 'left'"
                  >
                    左侧
                  </button>
                  <button
                    :class="{ on: settings.info.cover === 'right' }"
                    @click="settings.info.cover = 'right'"
                  >
                    右侧
                  </button>
                  <button
                    :class="{ on: settings.info.cover === 'hidden' }"
                    @click="settings.info.cover = 'hidden'"
                  >
                    不显示
                  </button>
                </div>
                <span class="val"></span>
              </div>
              <div class="row">
                <span class="lbl">文本位置</span>
                <div class="seg grow">
                  <button
                    :class="{ on: settings.info.text === 'left' }"
                    @click="settings.info.text = 'left'"
                  >
                    左
                  </button>
                  <button
                    :class="{ on: settings.info.text === 'center' }"
                    @click="settings.info.text = 'center'"
                  >
                    中
                  </button>
                  <button
                    :class="{ on: settings.info.text === 'right' }"
                    @click="settings.info.text = 'right'"
                  >
                    右
                  </button>
                  <button
                    :class="{ on: settings.info.text === 'hidden' }"
                    @click="settings.info.text = 'hidden'"
                  >
                    不显示
                  </button>
                </div>
                <span class="val"></span>
              </div>
            </template>

            <h3>词板尺寸</h3>
            <p v-if="parkMode" class="hint">请先关闭「无播放时关闭词板」再调整相关设置</p>
            <div class="dimmed" :class="{ on: parkMode }">
            <div class="row">
              <span class="lbl">宽度</span>
              <input
                v-model.number="settings.window.width"
                class="range"
                type="range"
                min="320"
                max="1400"
                step="10"
              />
              <span class="val">{{ settings.window.width }}</span>
            </div>
            <div class="row">
              <span class="lbl">高度</span>
              <input
                v-model.number="settings.window.height"
                class="range"
                type="range"
                min="90"
                max="620"
                step="10"
              />
              <span class="val">{{ settings.window.height }}</span>
            </div>
            <div class="row">
              <span class="lbl">可显示</span>
              <div class="grow muted">当前大约可显示 {{ capacity }} 行歌词</div>
            </div>
            </div>

            <h3>字体</h3>
            <div class="row">
              <span class="lbl">字体</span>
              <FontSelect
                v-model="settings.appearance.fontFamily"
                :fonts="fonts"
                :curated="CURATED"
              />
              <span class="val"></span>
            </div>
            <div class="row">
              <span class="lbl">字号</span>
              <input
                v-model.number="settings.appearance.fontSize"
                class="range"
                type="range"
                min="14"
                max="96"
                step="1"
              />
              <span class="val">{{ settings.appearance.fontSize }}</span>
            </div>
            <div class="row">
              <span class="lbl">字重</span>
              <input
                v-model.number="settings.appearance.fontWeight"
                class="range"
                type="range"
                min="300"
                max="900"
                step="100"
              />
              <span class="val">{{ settings.appearance.fontWeight }}</span>
            </div>
            <div class="row">
              <span class="lbl">行距</span>
              <input
                v-model.number="settings.appearance.lineHeight"
                class="range"
                type="range"
                min="26"
                max="140"
                step="1"
              />
              <span class="val">{{ settings.appearance.lineHeight }}</span>
            </div>
            <label class="switch">
              <span>文本超出时自动缩小</span>
              <input v-model="settings.appearance.autoFit" type="checkbox" />
              <i></i>
            </label>

            <h3>对齐</h3>
            <div class="row">
              <span class="lbl">水平</span>
              <div class="seg grow">
                <button
                  v-for="al in H_ALIGNS"
                  :key="al.k"
                  :class="{ on: settings.appearance.align === al.k }"
                  @click="settings.appearance.align = al.k"
                >
                  {{ al.label }}
                </button>
              </div>
              <span class="val"></span>
            </div>
            <div class="row">
              <span class="lbl">垂直</span>
              <div class="seg grow">
                <button
                  :class="{ on: settings.appearance.vAlign === 'top' }"
                  @click="settings.appearance.vAlign = 'top'"
                >
                  顶部对齐
                </button>
                <button
                  :class="{ on: settings.appearance.vAlign === 'center' }"
                  @click="settings.appearance.vAlign = 'center'"
                >
                  垂直居中
                </button>
              </div>
              <span class="val"></span>
            </div>

            <h3>颜色</h3>
            <div class="row">
              <span class="lbl">模板</span>
              <div class="seg grow">
                <button :class="{ on: activePreset === 'light' }" @click="applyPreset('light')">
                  白字
                </button>
                <button :class="{ on: activePreset === 'dark' }" @click="applyPreset('dark')">
                  黑字
                </button>
              </div>
              <span class="val"></span>
            </div>
            <div class="row">
              <span class="lbl">当前行</span>
              <button type="button" class="color" @click="pickColor('colorCurrent')">
                <span :style="{ background: settings.appearance.colorCurrent }"></span>
                <em>{{ settings.appearance.colorCurrent }}</em>
              </button>
              <span class="val"></span>
            </div>
            <div class="row">
              <span class="lbl">其余行</span>
              <button type="button" class="color" @click="pickColor('colorOther')">
                <span :style="{ background: settings.appearance.colorOther }"></span>
                <em>{{ settings.appearance.colorOther }}</em>
              </button>
              <span class="val"></span>
            </div>
            <div v-if="show.ktvColor" class="row">
              <span class="lbl">逐字高亮</span>
              <button type="button" class="color" @click="pickColor('colorHighlight')">
                <span :style="{ background: settings.appearance.colorHighlight }"></span>
                <em>{{ settings.appearance.colorHighlight }}</em>
              </button>
              <span class="val"></span>
            </div>

            <h3>描边与阴影</h3>
            <div class="row">
              <span class="lbl">描边宽度</span>
              <input
                v-model.number="settings.appearance.strokeWidth"
                class="range"
                type="range"
                min="0"
                max="5"
                step="0.1"
              />
              <span class="val">{{ settings.appearance.strokeWidth.toFixed(1) }}</span>
            </div>
            <div class="row">
              <span class="lbl">描边颜色</span>
              <button type="button" class="color" @click="pickColor('strokeColor')">
                <span :style="{ background: settings.appearance.strokeColor }"></span>
                <em>{{ settings.appearance.strokeColor }}</em>
              </button>
              <span class="val"></span>
            </div>
            <div class="row">
              <span class="lbl">阴影强度</span>
              <input
                v-model.number="settings.appearance.shadow"
                class="range"
                type="range"
                min="0"
                max="1"
                step="0.01"
              />
              <span class="val">{{ settings.appearance.shadow.toFixed(2) }}</span>
            </div>
            <div class="row">
              <span class="lbl">阴影颜色</span>
              <button type="button" class="color" @click="pickColor('shadowColor')">
                <span :style="{ background: settings.appearance.shadowColor }"></span>
                <em>{{ settings.appearance.shadowColor }}</em>
              </button>
              <span class="val"></span>
            </div>

            <template v-if="show.blur || show.fade">
              <h3>动效</h3>
              <div class="row">
                <span class="lbl">当前行放大</span>
                <input
                  v-model.number="settings.appearance.currentScale"
                  class="range"
                  type="range"
                  min="1"
                  max="1.6"
                  step="0.01"
                />
                <span class="val">{{ settings.appearance.currentScale.toFixed(2) }}</span>
              </div>
              <div v-if="show.blur" class="row">
                <span class="lbl">模糊强度</span>
                <input
                  v-model.number="settings.appearance.blurStep"
                  class="range"
                  type="range"
                  min="0"
                  max="2.5"
                  step="0.1"
                />
                <span class="val">{{ settings.appearance.blurStep.toFixed(1) }}</span>
              </div>
              <div v-if="show.fade" class="row">
                <span class="lbl">淡出速度</span>
                <input
                  v-model.number="settings.appearance.fadeStep"
                  class="range"
                  type="range"
                  min="0.04"
                  max="0.35"
                  step="0.01"
                />
                <span class="val">{{ settings.appearance.fadeStep.toFixed(2) }}</span>
              </div>
            </template>

            <h3>背景样式</h3>
            <p v-if="parkMode" class="hint">请先关闭「无播放时关闭词板」再调整相关设置</p>
            <div class="dimmed" :class="{ on: parkMode }">
            <div class="row">
              <span class="lbl">不透明度</span>
              <input
                v-model.number="settings.glass.alpha"
                class="range"
                type="range"
                min="0"
                max="0.92"
                step="0.01"
              />
              <span class="val">{{ settings.glass.alpha.toFixed(2) }}</span>
            </div>
            <div class="row">
              <span class="lbl">色调</span>
              <button type="button" class="color" @click="pickColor('tint')">
                <span :style="{ background: settings.glass.tint }"></span>
                <em>{{ settings.glass.tint }}</em>
              </button>
              <span class="val"></span>
            </div>
            </div>
          </section>

          <!-- ---------------------------------------------------- 关于 -->
          <section v-show="tab === 'about'">
            <div class="about-head">
              <img src="/icon.svg" alt="" class="about-logo" />
              <div class="about-title">
                <div class="about-name">
                  LyricsFloat
                  <span class="about-ver">{{ version ? `v${version}` : '—' }}</span>
                </div>
              </div>
            </div>

            <h3>版本</h3>
            <div class="row">
              <span class="lbl">当前版本</span>
              <div class="grow muted mono">{{ version || '—' }}</div>
              <button class="mini" :disabled="checking || !repoUrl" @click="doCheckUpdate">
                {{ checking ? '检查中…' : '检查更新' }}
              </button>
            </div>
            <p v-if="updateState.text" class="hint" :class="`up-${updateState.kind}`">
              {{ updateState.text }}
            </p>
            <div v-if="updateState.url" class="update-actions">
              <button class="primary" @click="openLink(updateState.url)">前往下载</button>
            </div>
            <div v-if="notesHtml" class="notesblock">
              <div class="noteshead">{{ notesTitle }}</div>
              <!-- eslint-disable-next-line vue/no-v-html -- renderNotes 已先转义 HTML -->
              <div class="notes" v-html="notesHtml"></div>
            </div>
            <p v-else-if="updateState.kind === 'latest' || updateState.kind === 'new'" class="hint">
              这个版本没有填写更新说明。
            </p>
            <p v-if="!repoUrl" class="hint">仓库还没发布，暂时无法检查更新。</p>

            <h3>项目</h3>
            <div class="row">
              <span class="lbl">仓库</span>
              <div class="grow muted mono" :title="repoUrl">{{ repoUrl || '待发布' }}</div>
              <button v-if="repoUrl" class="mini" @click="openLink(repoUrl)">打开</button>
            </div>
            <div class="row">
              <span class="lbl">作者</span>
              <div class="grow muted">{{ ABOUT.author }}</div>
            </div>

            <h3>使用与数据</h3>
            <p class="hint">设置只存在本机，不上传任何使用数据。</p>
          </section>
        </div>
      </div>
    </div>

    <transition name="toast">
      <div v-if="toast" class="toast">{{ toast }}</div>
    </transition>
  </div>
</template>

<style scoped>
.win {
  position: relative;
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: rgba(18, 19, 23, 0.97);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.09);
  color: rgba(255, 255, 255, 0.88);
}

.titlebar {
  height: 44px;
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 14px;
  background: rgba(255, 255, 255, 0.035);
  box-shadow: inset 0 -1px 0 rgba(255, 255, 255, 0.06);
}

.brand {
  display: flex;
  align-items: center;
  gap: 9px;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.72);
}

.logo {
  width: 18px;
  height: 18px;
}

.wctl {
  display: flex;
  gap: 2px;
}

.wctl button {
  width: 34px;
  height: 28px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: rgba(255, 255, 255, 0.6);
  cursor: pointer;
}

.wctl button:hover {
  background: rgba(255, 255, 255, 0.09);
  color: #fff;
}

.wctl button.close:hover {
  background: #c0392b;
  color: #fff;
}

.wctl svg {
  width: 14px;
  height: 14px;
}

.main {
  flex: 1;
  display: flex;
  min-height: 0;
}

.nav {
  width: 132px;
  flex: none;
  padding: 12px 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  background: rgba(255, 255, 255, 0.02);
  box-shadow: inset -1px 0 0 rgba(255, 255, 255, 0.06);
}

.nav button {
  height: 32px;
  padding: 0 11px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: rgba(255, 255, 255, 0.6);
  font-size: 12.5px;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition: all 130ms ease;
}

.nav button:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #fff;
}

.nav button.on {
  background: rgba(127, 212, 255, 0.15);
  color: #a6e3ff;
}

.nav-spacer {
  flex: 1;
}

.now {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 8px 2px;
  min-width: 0;
}

.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.24);
  flex: none;
}

.dot.live {
  background: #6ee7a8;
  box-shadow: 0 0 8px rgba(110, 231, 168, 0.8);
}

.now-text {
  font-size: 10.5px;
  line-height: 1.4;
  color: rgba(255, 255, 255, 0.34);
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}

.body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

/* 预览条固定在顶部：调任何参数都能立刻在同一视野里看到结果 */
.preview {
  flex: none;
  position: relative;
  padding: 16px 22px 14px;
  box-shadow: inset 0 -1px 0 rgba(255, 255, 255, 0.06);
  background: rgba(14, 15, 19, 0.6);
}

.paper {
  position: relative;
  height: 112px;
  overflow: hidden;
  transition: height 180ms ease;
}

.glasslayer {
  position: absolute;
  inset: 0;
}

.scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 4px 26px 30px;
}

.scroll::-webkit-scrollbar {
  width: 9px;
}

.scroll::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.13);
  border-radius: 5px;
}

/* 分组标题：比正文（12.5px）大一号并加粗，扫视时一眼分开各段 */
h3 {
  font-size: 13.5px;
  font-weight: 600;
  letter-spacing: 0.01em;
  color: rgba(255, 255, 255, 0.74);
  margin: 24px 0 12px;
}

section > h3:first-child {
  margin-top: 8px;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 30px;
  margin-bottom: 10px;
}

.lbl {
  width: 78px;
  flex: none;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.62);
}

.grow {
  flex: 1;
  min-width: 0;
}

.muted {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.44);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 歌词源：纯标记，不是按钮 —— 不做 hover，也不给可点的暗示 */
.srcbadge {
  display: inline-block;
  padding: 1px 8px;
  border-radius: 5px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.05);
  font-size: 11.5px;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.6);
}

/* 歌词源：曲库切换 + 候选清单 */
.srcpick {
  margin-top: 7px;
}

.candbox {
  margin: 8px 0 4px;
  border: 1px solid rgba(255, 255, 255, 0.07);
  border-radius: 7px;
  overflow: hidden;
}

.cand {
  display: grid;
  grid-template-columns: 50px minmax(0, 1.5fr) minmax(0, 1fr) 46px 48px;
  gap: 8px;
  align-items: center;
  padding: 5px 9px;
  font-size: 11.5px;
  line-height: 1.5;
  color: rgba(255, 255, 255, 0.54);
  border-top: 1px solid rgba(255, 255, 255, 0.05);
}

.cand:first-child {
  border-top: 0;
}

/* 真正用上的那条 */
.cand.cur {
  background: rgba(127, 212, 255, 0.1);
  color: rgba(255, 255, 255, 0.9);
}

.csrc {
  color: rgba(255, 255, 255, 0.38);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cand.cur .csrc {
  color: #7fd4ff;
}

.cname,
.cart {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cdur,
.ctag {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.ctag.off {
  color: rgba(255, 255, 255, 0.26);
}

.path {
  font-size: 11.5px;
  color: rgba(255, 255, 255, 0.5);
  direction: rtl;
  text-align: left;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hint {
  margin: 2px 0 0;
  font-size: 11.5px;
  line-height: 1.65;
  color: rgba(255, 255, 255, 0.34);
}

.mini {
  height: 24px;
  padding: 0 10px;
  flex: none;
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(255, 255, 255, 0.05);
  color: rgba(255, 255, 255, 0.66);
  font-size: 11.5px;
  font-family: inherit;
  cursor: pointer;
}

.mini:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.mini:disabled {
  opacity: 0.4;
  cursor: default;
}

/* 播放器选择：名字长短不齐，用可以换行的 chip 而不是等宽分段按钮 */
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding: 2px 0;
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 12px;
  border-radius: 7px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(255, 255, 255, 0.05);
  color: rgba(255, 255, 255, 0.6);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chip:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.chip.on {
  border-color: rgba(127, 212, 255, 0.5);
  background: rgba(127, 212, 255, 0.16);
  color: #cfeeff;
}

.chip-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #6ee7a8;
  flex: none;
}

/* 锁定了播放器之后来源不可改：整排压暗且不吃点击 */
.chips.off {
  opacity: 0.4;
  pointer-events: none;
}

.chip:disabled {
  cursor: default;
}

/* ------------------------------------------------------------- 关于页 */

.about-head {
  display: flex;
  align-items: center;
  gap: 14px;
  margin: 10px 0 4px;
}

.about-logo {
  width: 44px;
  height: 44px;
  flex: none;
}

.about-title {
  min-width: 0;
}

.about-name {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 16px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.92);
  letter-spacing: 0.02em;
}

.about-ver {
  font-size: 11px;
  font-weight: 400;
  color: rgba(255, 255, 255, 0.36);
  font-variant-numeric: tabular-nums;
}

/* 检查结果要能一眼分出"有新版本"和"检查失败" */
.hint.up-new {
  color: #9fe3b4;
}

.hint.up-error {
  color: #ff9d9d;
}

.update-actions {
  display: flex;
  gap: 8px;
  margin-top: 4px;
}

/* Release 说明是纯文本（还带换行），别按 HTML 渲染 */
.notesblock {
  margin-top: 10px;
}

.noteshead {
  font-size: 11.5px;
  color: rgba(255, 255, 255, 0.42);
  margin-bottom: 5px;
}

.notes {
  margin: 0;
  padding: 10px 12px;
  max-height: 148px;
  overflow-y: auto;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
  font-size: 11.5px;
  line-height: 1.65;
  color: rgba(255, 255, 255, 0.64);
  word-break: break-word;
}

.notes :deep(h4) {
  margin: 8px 0 3px;
  font-size: 12px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.84);
}

.notes :deep(h4:first-child) {
  margin-top: 0;
}

.notes :deep(p) {
  margin: 3px 0;
}

.notes :deep(ul) {
  margin: 3px 0;
  padding-left: 16px;
  list-style: disc;
}

.notes :deep(li) {
  margin: 1px 0;
}

.notes :deep(strong) {
  font-weight: 600;
  color: rgba(255, 255, 255, 0.86);
}

.notes :deep(code) {
  padding: 0 4px;
  border-radius: 3px;
  background: rgba(255, 255, 255, 0.08);
  font-family: 'Cascadia Mono', Consolas, monospace;
  font-size: 11px;
}

.val {
  width: 46px;
  flex: none;
  text-align: right;
  font-size: 11px;
  color: rgba(255, 255, 255, 0.4);
  font-variant-numeric: tabular-nums;
}

.range {
  flex: 1;
  -webkit-appearance: none;
  appearance: none;
  height: 3px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.15);
  outline: none;
}

.range::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  background: #dcdcd8;
  cursor: pointer;
  border: none;
}

.seg {
  display: flex;
  gap: 4px;
}

.seg button {
  flex: 1;
  height: 26px;
  border-radius: 6px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.045);
  color: rgba(255, 255, 255, 0.6);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
}

.seg button.on {
  background: rgba(127, 212, 255, 0.16);
  border-color: rgba(127, 212, 255, 0.48);
  color: #a6e3ff;
}

/* 系统取色器：input 放在窗口外的一角，点击由色块按钮用 click() 触发，
   这样聚焦时不会把整页滚动过去 */
.palette {
  position: fixed;
  left: 0;
  top: 0;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
  z-index: -1;
}

.paper-empty {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.34);
}

.mono {
  font-variant-numeric: tabular-nums;
}

.color {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  cursor: pointer;
  flex: 1;
  padding: 0;
  border: none;
  background: none;
  font-family: inherit;
  color: inherit;
  text-align: left;
}

.color span {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.24);
  flex: none;
}

.color em {
  font-style: normal;
  font-size: 11px;
  color: rgba(255, 255, 255, 0.4);
  font-variant-numeric: tabular-nums;
}

.switch {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12.5px;
  color: rgba(255, 255, 255, 0.62);
  margin-bottom: 0;
  cursor: pointer;
  position: relative;
  gap: 10px;
  /* 开关一律单行：名字长到换行会让整行高度和邻居错开 */
  white-space: nowrap;
}

.switch > span {
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 开关两列排布：一行两个，比竖着排一长串好扫 */
.swrow {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px 26px;
  margin-bottom: 12px;
}

/* 单独一个开关时占满整行（标签在左、开关在右），别缩在左半边 */
.swrow.single {
  grid-template-columns: 1fr;
  margin-top: 14px;
}

.switch input {
  position: absolute;
  opacity: 0;
  pointer-events: none;
}

.switch i {
  width: 34px;
  height: 19px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.14);
  position: relative;
  transition: background 160ms ease;
  flex: none;
}

.switch i::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: #fff;
  transition: transform 160ms ease;
}

.switch input:checked + i {
  background: rgba(127, 212, 255, 0.62);
}

.switch input:checked + i::after {
  transform: translateX(15px);
}

.danger-zone {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 22px;
}

/* 退出单独一行放在最底下：它和"恢复默认"这类不退出进程的操作不是一回事 */
.quitrow {
  margin-top: 26px;
  padding-top: 16px;
  border-top: 1px solid rgba(255, 255, 255, 0.07);
}

.ghost,
.danger {
  height: 30px;
  padding: 0 13px;
  border-radius: 7px;
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(255, 255, 255, 0.05);
  color: rgba(255, 255, 255, 0.72);
}

.ghost:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.danger {
  border-color: rgba(255, 96, 96, 0.32);
  color: #ff9d9d;
}

.danger:hover {
  background: rgba(255, 92, 92, 0.16);
  color: #ffb3b3;
}

/* 「前往下载」这种主操作：只有它在更新提示里出现，稍微显眼一点 */
.primary {
  height: 30px;
  padding: 0 15px;
  border-radius: 7px;
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  border: 1px solid rgba(127, 212, 255, 0.42);
  background: rgba(127, 212, 255, 0.16);
  color: #cfeeff;
}

.primary:hover {
  background: rgba(127, 212, 255, 0.26);
  color: #fff;
}

/* 词板隐藏时这些分组没有意义：整组压暗并吃掉鼠标事件（防止误拖滑块） */
.dimmed {
  transition: opacity 160ms ease;
}

.dimmed.on {
  opacity: 0.32;
  pointer-events: none;
  user-select: none;
}

.toast {
  position: absolute;
  left: 50%;
  bottom: 26px;
  transform: translateX(-50%);
  max-width: 78%;
  padding: 9px 15px;
  border-radius: 9px;
  background: rgba(28, 30, 38, 0.97);
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.1),
    0 14px 32px rgba(0, 0, 0, 0.5);
  color: rgba(255, 255, 255, 0.88);
  font-size: 12px;
  line-height: 1.5;
  z-index: 950;
  word-break: break-all;
}

.toast-enter-active,
.toast-leave-active {
  transition:
    opacity 180ms ease,
    transform 180ms ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(6px);
}
</style>

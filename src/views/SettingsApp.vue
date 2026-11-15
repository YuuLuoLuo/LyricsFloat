<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import LyricStage from '../components/LyricStage.vue'
import FontSelect from '../components/FontSelect.vue'
import { settings, runtime, saveSettings, resetSettings, pullSettings } from '../core/store'
import { clock } from '../core/clock'
import { findLineIndex } from '../core/parser'
import { initPlayer, reapplyOffset } from '../core/player'
import { exportSettingsToDesktop, importSettingsFile, initSettingsFile, settingsFilePath } from '../core/persist'
import { ABOUT } from '../core/about'
import type { LyricLine } from '../core/types'
import {
  appVersion,
  checkUpdate,
  closeSettingsWindow,
  getAutostart,
  isTauri,
  listFonts,
  minimizeSelf,
  onFileDrop,
  openUrl,
  peekBoard,
  quitApp,
  setAlwaysOnTop,
  setAutostart,
  setLocked,
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
const previewDoc = computed<{ lines: LyricLine[]; time: number } | null>(() => {
  const doc = runtime.doc
  if (!doc || !doc.lines.length) return null
  const i = findLineIndex(doc.lines, runtime.time)
  const idx = i < 0 ? 0 : Math.min(i, doc.lines.length - 1)
  const line = doc.lines[idx]
  const lo = line.start
  const hi = Math.max(line.start, line.end - 1)
  return { lines: [line], time: Math.min(Math.max(runtime.time, lo), hi) }
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

const showTrans = computed(() => settings.view.translation)

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
  if (runtime.doc) return `${runtime.doc.lines.length} 行 · ${runtime.source || '未知来源'}`
  if (runtime.loading) return '正在获取歌词…'
  return runtime.notice || '—'
})

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

function toggleLocked(v: boolean) {
  settings.behavior.locked = v
  saveSettings()
  void setLocked(v)
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

const pathText = ref('')
const toast = ref('')
let toastTimer: ReturnType<typeof setTimeout> | null = null

function say(msg: string): void {
  toast.value = msg
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = ''
  }, 4200)
}

async function copyPath(): Promise<void> {
  if (!pathText.value) return
  try {
    await navigator.clipboard.writeText(pathText.value)
    say('路径已复制')
  } catch {
    // WebView 里偶尔没有剪贴板权限，退回到 textarea + execCommand
    const ta = document.createElement('textarea')
    ta.value = pathText.value
    ta.style.position = 'fixed'
    ta.style.opacity = '0'
    document.body.appendChild(ta)
    ta.select()
    try {
      document.execCommand('copy')
      say('路径已复制')
    } catch {
      say('复制失败，请手动选中')
    }
    ta.remove()
  }
}

/** 导入没有文件选择框（不引额外插件），靠把 json 拖进窗口 */
function hintImport(): void {
  say('把 .json 设置文件拖进窗口即可导入')
}

async function doExport(): Promise<void> {
  const p = await exportSettingsToDesktop()
  say(p ? `已导出到 ${p}` : '导出失败：桌面不可写')
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
}

const updateState = ref<UpdateState>({ kind: 'idle', text: '' })
const checking = ref(false)

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
  updateState.value = info.newer
    ? {
        kind: 'new',
        text: `发现新版本 ${info.latest}${info.published ? ` · ${info.published}` : ''}`,
        url: info.url,
        notes: info.notes,
      }
    : { kind: 'latest', text: `已是最新版本（${info.latest}）` }
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

  // 设置文件 + 系统字体列表
  await initSettingsFile()
  pathText.value = settingsFilePath()
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
        <span>歌词板设置</span>
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
        <!-- 预览固定在顶部，不随设置滚动 -->
        <div class="preview">
          <div class="paper" :style="previewPaper">
            <div class="glasslayer" :style="previewGlass"></div>
            <LyricStage
              v-if="previewDoc"
              :lines="previewDoc.lines"
              :time="previewDoc.time"
              :height="112"
            />
            <div v-else class="paper-empty">{{ runtime.notice || '等待播放器' }}</div>
          </div>
        </div>

        <div class="scroll">
          <!-- ---------------------------------------------------- 常规 -->
          <section v-show="tab === 'general'">
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

            <h3>行为</h3>
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
            <label class="switch">
              <span>锁定词板</span>
              <input
                type="checkbox"
                :checked="settings.behavior.locked"
                @change="toggleLocked(($event.target as HTMLInputElement).checked)"
              />
              <i></i>
            </label>

            <h3>设置文件</h3>
            <div class="row">
              <span class="lbl">位置</span>
              <div class="grow path mono" :title="pathText">{{ pathText || '（浏览器预览无文件）' }}</div>
              <button class="mini" @click="copyPath">复制</button>
            </div>
            <p class="hint">
              配置随软件本体保存。把导出的设置文件拖入窗口即可导入。
            </p>
            <div class="danger-zone">
              <button class="ghost" @click="doExport">导出到桌面</button>
              <button class="ghost" @click="hintImport">导入…</button>
              <button class="ghost" @click="resetSettings">恢复默认</button>
              <button class="danger" @click="quit">退出歌词板</button>
            </div>
          </section>

          <!-- ---------------------------------------------------- 外观 -->
          <section v-show="tab === 'look'">
            <h3>歌词样式</h3>
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
            <p class="hint">
              <template v-if="settings.view.rotate">
                以对齐那一侧为圆心旋转：居左绕左边、居右绕右边，上下句模糊后压在当前句下面。
              </template>
              <template v-else>几种样式可以同时开启。高度压到只剩一句，就是单行显示。</template>
            </p>

            <template v-if="show.card">
              <h3>歌词信息布局</h3>
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
                <span class="lbl">标题</span>
                <div class="seg grow">
                  <button
                    :class="{ on: settings.info.title === 'top' }"
                    @click="settings.info.title = 'top'"
                  >
                    顶部
                  </button>
                  <button
                    :class="{ on: settings.info.title === 'bottom' }"
                    @click="settings.info.title = 'bottom'"
                  >
                    底部
                  </button>
                  <button
                    :class="{ on: settings.info.title === 'hidden' }"
                    @click="settings.info.title = 'hidden'"
                  >
                    不显示
                  </button>
                </div>
                <span class="val"></span>
              </div>
            </template>

            <h3>词板尺寸</h3>
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
              <div class="grow muted">{{ capacity }} 行歌词（随高度自动）</div>
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

            <h3>玻璃背景</h3>
            <div class="row">
              <span class="lbl">浓度</span>
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
          </section>

          <!-- ---------------------------------------------------- 关于 -->
          <section v-show="tab === 'about'">
            <div class="about-head">
              <img src="/icon.svg" alt="" class="about-logo" />
              <div class="about-title">
                <div class="about-name">
                  歌词板
                  <span class="about-ver">{{ version ? `v${version}` : '—' }}</span>
                </div>
                <div class="about-desc">{{ ABOUT.tagline }}</div>
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
              <button class="ghost" @click="openLink(updateState.url)">打开发布页</button>
            </div>
            <pre v-if="updateState.notes" class="notes">{{ updateState.notes }}</pre>
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
            <p class="hint">
              鼠标移到词板上才会显形，此时可以拖动或右键；锁定后点击直接穿过去。
              歌词取自公开歌词库，找不到时词板会写明原因。
            </p>
            <p class="hint">设置只存在本机，不收集也不上传任何使用数据。</p>
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

h3 {
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0.09em;
  color: rgba(255, 255, 255, 0.38);
  margin: 22px 0 12px;
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

.about-desc {
  margin-top: 4px;
  font-size: 11.5px;
  line-height: 1.55;
  color: rgba(255, 255, 255, 0.42);
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
.notes {
  margin: 10px 0 0;
  padding: 10px 12px;
  max-height: 148px;
  overflow-y: auto;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
  font-family: inherit;
  font-size: 11.5px;
  line-height: 1.65;
  color: rgba(255, 255, 255, 0.6);
  white-space: pre-wrap;
  word-break: break-word;
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
  margin-bottom: 12px;
  cursor: pointer;
  position: relative;
  gap: 10px;
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
  gap: 8px;
  margin-top: 22px;
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

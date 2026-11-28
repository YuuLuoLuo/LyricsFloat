<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import LyricStage from './LyricStage.vue'
import MediaBar from './MediaBar.vue'
import MediaProgress from './MediaProgress.vue'
import { settings, runtime, glassVisible } from '../core/store'
import {
  applyBoard,
  isTauri,
  openSettings,
  quitApp,
  savePosition,
  onMoved,
  setLocked,
  setGlassColor,
  setMediaClickable,
  setMediaRects,
  startDragging,
} from '../core/bridge'

const revealed = computed(() => glassVisible())

/** 背景色：「从封面取色」开着就用封面主色，没有封面时用中灰 */
const glassTint = computed(() => {
  if (!settings.glass.autoColor) return settings.glass.tint
  return runtime.coverColor || '#808080'
})

const glassStyle = computed(() => {
  const { alpha } = settings.glass
  return { background: `rgba(${hexToRgb(glassTint.value)}, ${alpha})` }
})

/**
 * 背景色变了就通知 Rust 换亚克力底色。
 * 走 `set_glass_color` 而不是 `apply_board`：后者会顺带 resize，
 * 换歌时（取色变了）会闪一下。
 */
watch(
  () => [glassTint.value, settings.glass.alpha] as const,
  ([tint, alpha]) => {
    if (isTauri) void setGlassColor(tint, alpha)
  },
)

function hexToRgb(hex: string): string {
  const h = hex.replace('#', '')
  const full =
    h.length === 3
      ? h
          .split('')
          .map((c) => c + c)
          .join('')
      : h
  const n = parseInt(full, 16)
  if (Number.isNaN(n)) return '20, 20, 26'
  return `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`
}

/** #rgb / #rrggbb -> [r, g, b] */
function rgbOf(hex: string): [number, number, number] {
  const h = hex.replace('#', '')
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h
  const n = parseInt(full, 16)
  if (Number.isNaN(n)) return [128, 128, 128]
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

/** 感知亮度（0~1），够用来判断"两个颜色会不会糊在一起" */
function luma([r, g, b]: [number, number, number]): number {
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255
}

function toHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`
}

/**
 * 取色时自动躲开词板背景。
 *
 * 深色封面配深色背景、浅色配浅色，控件会"糊在背景里看不见"。这里比一下亮度，
 * 差得不够就按背景的反方向推（背景暗就提亮，背景亮就压暗）。
 */
function standOut(color: string, bg: string, gap = 0.26): [number, number, number] {
  const fg = rgbOf(color)
  const bgL = luma(rgbOf(bg))
  const fgL = luma(fg)
  if (Math.abs(fgL - bgL) >= gap) return fg
  const dir = bgL < 0.5 ? 1 : -1
  const target = Math.min(1, Math.max(0, bgL + dir * gap))
  const k = fgL > 0.02 ? target / fgL : 1
  return fg.map((v) => Math.min(255, Math.max(0, Math.round(v * k)))) as [number, number, number]
}

const mediaStyle = computed(() => {
  const t = cardText.value
  const leftSide = t === 'hidden' ? cardCover.value === 'right' : t === 'right'
  const style: Record<string, string | number> = { order: mediaOrder.value }
  // 文本显示时由 meta 的 flex:1 把控件顶到另一侧；文本不显示时没有那个撑开的
  // 元素，得自己用 auto margin 顶过去。
  if (t === 'hidden' && !leftSide) style.marginLeft = 'auto'
  if (t === 'hidden' && leftSide) style.marginRight = 'auto'
  return style
})
const accentHex = computed(() =>
  toHex(standOut(runtime.coverColor || '#8a8f9c', glassTint.value)),
)

/**
 * 控件底色。默认是固定的暗色块；开启跟随取色时用封面色，
 * 先压暗一档（按钮是"底"，不能比上面的图标还亮），再确保和词板背景分得开。
 */
const mediaBtnBg = computed(() => {
  if (!settings.media.tint) return 'rgba(0, 0, 0, 0.30)'
  const raw = runtime.coverColor || '#8a8f9c'
  const dark = rgbOf(raw).map((v) => Math.round(v * 0.7)) as [number, number, number]
  const [r, g, b] = standOut(toHex(dark), glassTint.value, 0.16)
  return `rgba(${r}, ${g}, ${b}, 0.66)`
})

/** 进度条颜色：不开取色就是白色（不可手改） */
const progressColor = computed(() => (settings.media.barTint ? accentHex.value : '#ffffff'))

async function toggleLock() {
  if (isTauri) {
    const { toggleLocked } = await import('../core/bridge')
    const next = await toggleLocked()
    runtime.locked = next ?? !runtime.locked
  } else {
    runtime.locked = !runtime.locked
    void setLocked(runtime.locked)
  }
  runtime.menuOpen = false
}

/* --------------------------------------------------------------- 拖动 */

async function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return
  const el = e.target as HTMLElement
  if (el.closest('button, .ctx')) return
  // 未显形时窗口是穿透的，也不该拖动
  if (!revealed.value) return
  await startDragging(e)
}

/* --------------------------------------------------------- 右键菜单 */

function onContextMenu(e: MouseEvent) {
  e.preventDefault()
  runtime.menuOpen = !runtime.menuOpen
}

function closeMenu() {
  runtime.menuOpen = false
}

function openSettingsFromMenu() {
  runtime.menuOpen = false
  void openSettings()
}

function quitFromMenu() {
  runtime.menuOpen = false
  void quitApp()
}

/* --------------------------------------------------------------- 位置 */

let offMoved: (() => void) | null = null
let saveTimer: ReturnType<typeof setTimeout> | null = null

/** 窗口尺寸变了，控件矩形跟着变，锁定时可点要靠它才算得准 */
function onResize(): void {
  void nextTick(reportRects)
}

onMounted(async () => {
  window.addEventListener('resize', onResize)
  void nextTick(reportRects)
  if (!isTauri) return
  // 窗口尺寸以 settings 为准，挂载时对齐一次
  void applyBoard(settings)
  offMoved = await onMoved(() => {
    if (saveTimer) clearTimeout(saveTimer)
    saveTimer = setTimeout(() => void savePosition(), 500)
  })
  window.addEventListener('blur', closeMenu)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', onResize)
  offMoved?.()
  if (saveTimer) clearTimeout(saveTimer)
  window.removeEventListener('blur', closeMenu)
})

const stageDoc = computed(() => runtime.doc)

/**
 * 舞台时间。歌词加载期间（换歌了但新歌词还没拉回来）冻住不动。
 *
 * 新歌词要联网拉，没播放过的歌要 1~2 秒，这期间 `runtime.doc` 还是上一首的。
 * 若让时间继续走，旧歌词会跟着新歌的进度往上滚 —— 看着就像"歌词根本没换，
 * 只是在乱滚"。冻住之后旧歌词原地停着，等新 doc 到位再一次性切过去。
 *
 * `loadLyric` 里 `runtime.doc = ...` 和 `runtime.loading = false` 是同一段同步代码，
 * 中间没有 await，所以不存在"新 doc 已经上了、loading 还是 true"的中间帧：
 * 渲染时两者总是一起变，doc 一换时间就跟着恢复。
 * 加载失败或没有歌词时 loading 也会变 false，不会把自己冻死。
 */
const stageTime = ref(runtime.time)
watch(
  () => [runtime.time, runtime.loading] as const,
  ([t, loading]) => {
    if (!loading) stageTime.value = t
  },
)

/* ------------------------------------------------------- 暂停自动隐藏 */

/**
 * 暂停一段时间后不画歌词（窗口还在、仍然穿透，只是看不见）。
 *
 * 不做窗口级 hide：那样会把"无播放时关闭词板"的停放状态、悬停轮询全搅在一起，
 * 而且恢复时还要重新走一遍显形流程。这里只是 CSS 不透明度，恢复是即时的。
 */
const pausedHidden = ref(false)
let pauseTimer: number | null = null

function clearPauseTimer(): void {
  if (pauseTimer !== null) {
    clearTimeout(pauseTimer)
    pauseTimer = null
  }
}

/**
 * 暂停隐藏的重置规则：
 * - 非锁定：鼠标移到词板上（revealed）就算"在看"，立刻放出来；离开后重新计时
 * - 锁定：revealed 永远是 false，所以只有重新播放才会放出来
 */
watch(
  () => [
    runtime.playing,
    revealed.value,
    settings.behavior.hideOnPause,
    settings.behavior.pauseDelaySec,
    !!stageDoc.value,
  ] as const,
  ([playing, revealNow, on, sec, hasDoc]) => {
    clearPauseTimer()
    if (!on || playing || !hasDoc || (revealNow && !runtime.locked)) {
      pausedHidden.value = false
      return
    }
    const ms = Math.max(1, sec) * 1000
    pauseTimer = window.setTimeout(() => {
      pausedHidden.value = true
    }, ms)
  },
  { immediate: true },
)

/** 锁定时不看鼠标（revealed 恒假），所以这里不再跟着 revealed 收起 */
const pausedLyricsHidden = computed(() => pausedHidden.value)

/* ------------------------------------------------------------ 歌词信息 */

/**
 * 「显示歌词信息」三项独立：整条在顶部/底部、封面在文字左/右、
 * 文本在剩余空间里左/中/右。文本隐藏时只剩封面，封面也隐藏时退化成纯歌词。
 */
const infoOn = computed(() => settings.info.show)
const cardPos = computed(() => settings.info.pos)
const cardCover = computed(() => settings.info.cover)
const cardText = computed(() => settings.info.text)

/* ------------------------------------------------------------ 媒体控件 */

const mediaOn = computed(() => settings.media.show)
const mediaParts = computed(() => settings.media.parts)

/** 三个按钮里只要还有一个开着，按钮组就要渲染 */
const mediaBarOn = computed(
  () =>
    mediaOn.value &&
    (mediaParts.value.prev || mediaParts.value.toggle || mediaParts.value.next),
)
/**
 * 进度条是独立的第四个部件，但它挂在信息栏那一块上（顶/底、避让、跟随都按信息栏算）。
 * 关掉「显示歌词信息」时信息栏整条不在，进度条也不能自己留着 ——
 * 它是 `<MediaProgress>` 独立渲染的，不跟着 cardHead 走，所以这里显式带上 infoOn。
 */
const mediaProgressOn = computed(() => infoOn.value && mediaOn.value && mediaParts.value.bar)

const cardHead = computed(
  () =>
    infoOn.value &&
    (cardCover.value !== 'hidden' ||
      cardText.value !== 'hidden' ||
      mediaBarOn.value ||
      mediaProgressOn.value),
)

/** hover 模式：只有鼠标停在词板上时控件才显形 */
const mediaHoverOnly = computed(() => settings.media.mode === 'hover')

/**
 * 媒体控件开着时文本不再允许居中 —— 控件要贴在文本的反侧，
 * 文本居中就没地方摆了（设置页里那一项也会被禁用）。
 */
const effectiveText = computed(() => {
  const t = cardText.value
  return mediaOn.value && t === 'center' ? 'left' : t
})

/**
 * 按钮组在文本的反侧：文本左 -> 控件右；文本右 -> 控件左。
 *
 * 但**封面永远占最外侧**（最左或最右），控件只能排在封面内侧：
 * - 封面在左、控件也在左 -> 封面 0、控件 1（不能把控件排到 0，否则封面被挤到里面）
 * - 封面在右、控件在右 -> 控件 3、封面 4
 * 文本隐藏时按封面反侧摆控件，省得和封面抢同一侧。
 */
const mediaOrder = computed(() => {
  const t = cardText.value
  const leftSide = t === 'hidden' ? cardCover.value === 'right' : t === 'right'
  if (leftSide) return cardCover.value === 'left' ? 1 : 0
  return cardCover.value === 'right' ? 3 : 4
})

/**
 * 信息栏实际停在哪一侧。
 *
 * 有歌词时用用户设的 `pos`；没歌词时（只有信息栏 + 控件留在屏幕上）用 `emptyPos` ——
 * 无歌词状态常常是"歌刚切、歌词还没回来"或纯音乐，此时按歌词位置摆反而更别扭。
 */
const headSide = computed(() =>
  stageDoc.value ? settings.info.pos : settings.info.emptyPos,
)

/**
 * 信息栏顺序：舞台固定 0，所以顶部必须是负数。
 * 之前用 top=1 / bottom=3，结果"顶部"永远排到舞台后面 —— 只剩底部能用。
 */
const headOrder = computed(() => (headSide.value === 'top' ? -2 : 2))

/**
 * 进度条顺序。规则是「同侧时让到内侧，异侧时贴边」：
 * - 顶部：-1（信息栏在顶部时自然落在它下面，信息在底部时贴面板顶）
 * - 底部：1（信息栏在底部时落在它上面，信息在顶部时贴面板底）
 * - 跟随：跟信息栏同侧，所以直接按信息栏的位置算
 */
const barOrder = computed(() => {
  const bar = settings.media.bar === 'follow' ? headSide.value : settings.media.bar
  return bar === 'top' ? -1 : 1
})

/**
 * 把控件的实际位置报给 Rust。
 *
 * 锁定且允许点击时，窗口不整体穿透，改由 WM_NCHITTEST 按这些矩形放行 ——
 * 不报的话 Rust 不知道控件在哪，只能整窗穿透，按钮就点不动。
 */
function reportRects(): void {
  if (!isTauri) return
  const dpr = window.devicePixelRatio || 1
  const rects: [number, number, number, number][] = []
  for (const sel of ['.media-bar', '.mprogress']) {
    const el = document.querySelector(sel) as HTMLElement | null
    if (!el) continue
    const r = el.getBoundingClientRect()
    if (r.width <= 0 || r.height <= 0) continue
    rects.push([
      Math.round(r.left * dpr),
      Math.round(r.top * dpr),
      Math.round(r.right * dpr),
      Math.round(r.bottom * dpr),
    ])
  }
  void setMediaRects(rects)
}

watch(
  () => [settings.media.show, settings.media.mode, settings.media.lockedClickable] as const,
  () => {
    // 只有「一直显示 + 锁定可点」组合才有意义：
    // hover 模式下锁定时控件本来就不显形，让点击生效等于留一块隐形热区。
    const on =
      settings.media.show &&
      settings.media.mode === 'always' &&
      settings.media.lockedClickable
    if (isTauri) void setMediaClickable(on)
    void nextTick(reportRects)
  },
  { immediate: true },
)

// 位置相关的设置一变就要重新上报（文本/封面位置改了，控件会挪地方）
watch(
  () => [cardPos.value, cardText.value, cardCover.value, settings.media.bar] as const,
  () => void nextTick(reportRects),
)

/** 没有封面时整块按文本位置对齐 */
const headStyle = computed(() => {
  const color = { '--info-color': settings.info.color }
  if (cardCover.value !== 'hidden') return color
  const t = cardText.value
  return {
    ...color,
    justifyContent: t === 'center' ? 'center' : t === 'right' ? 'flex-end' : 'flex-start',
  }
})

const metaStyle = computed(() => ({
  textAlign: effectiveText.value === 'hidden' ? 'left' : effectiveText.value,
}))

/** 播放器给了封面就铺上去，没给就保持默认的渐变色块 */
const coverStyle = computed(() =>
  runtime.trackCover ? { backgroundImage: `url(${runtime.trackCover})` } : undefined,
)


/** 信息栏挪到底部时，锁定按钮也换到右上角，别和底部那行挤在一起 */
const lockOnTop = computed(() => cardHead.value && headSide.value === 'bottom')
</script>

<template>
  <div
    class="board"
    :class="{
      revealed,
      locked: runtime.locked,
      'glass-always': settings.glass.always,
      'pause-hidden': pausedLyricsHidden,
      'pause-hidden-all': pausedLyricsHidden && settings.behavior.hideMediaOnPause,
    }"
    @pointerdown="onPointerDown"
    @contextmenu="onContextMenu"
  >
    <div class="glass" :style="glassStyle"></div>

    <div class="content" :class="{ 'card-mode': infoOn, 'media-hover': mediaHoverOnly }">
      <div
        v-if="cardHead"
        class="card-head"
        :class="[`pos-${headSide}`, `cover-${cardCover}`]"
        :style="{ ...headStyle, order: headOrder }"
      >
        <div
          v-if="cardCover !== 'hidden'"
          class="cover"
          :class="{ 'has-img': !!runtime.trackCover }"
          :style="coverStyle"
        ></div>
        <div v-if="cardText !== 'hidden'" class="meta" :style="metaStyle">
          <div class="title">{{ runtime.trackTitle || '未在播放' }}</div>
          <div class="artist">{{ runtime.trackArtist || '等待播放器' }}</div>
        </div>
        <MediaBar
          v-if="mediaBarOn"
          class="media-slot"
          :style="mediaStyle"
          :bg="mediaBtnBg"
          :parts="mediaParts"
        />
      </div>

      <div class="stage-wrap">
        <LyricStage v-if="stageDoc" :lines="stageDoc.lines" :time="stageTime" />
      </div>

      <MediaProgress
        v-if="mediaProgressOn"
        class="mprogress"
        :style="{ order: barOrder }"
        :color="progressColor"
      />
    </div>

    <button
      v-if="revealed"
      class="lock"
      :class="{ off: runtime.locked, top: lockOnTop }"
      :title="runtime.locked ? '解锁词板' : '锁定词板'"
      @click.stop="toggleLock"
    >
      <svg
        v-if="runtime.locked"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.7"
        stroke-linecap="round"
      >
        <rect x="4.5" y="10.5" width="15" height="10" rx="2.6" />
        <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5" />
      </svg>
      <svg
        v-else
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.7"
        stroke-linecap="round"
      >
        <rect x="4.5" y="10.5" width="15" height="10" rx="2.6" />
        <path d="M8 10.5V7a4 4 0 0 1 7.6-1.4" />
      </svg>
    </button>

    <transition name="pop">
      <div v-if="runtime.menuOpen" class="ctx" @click.stop>
        <button @click="toggleLock">{{ runtime.locked ? '解锁词板' : '锁定词板' }}</button>
        <button @click="openSettingsFromMenu">打开设置…</button>
        <div class="sep"></div>
        <button class="danger" @click="quitFromMenu">退出</button>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.board {
  position: relative;
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  isolation: isolate;
  cursor: default;
}

/* 玻璃底：只有一层纯色填充。
   刻意不加 border / inset 描边 —— 那会在词板四周留一圈可见的边框；
   也刻意不用 backdrop-filter —— 透明窗口下它会强制图层合成成白底。
   模糊由系统的亚克力负责。 */
.glass {
  position: absolute;
  inset: 0;
  opacity: 0;
  pointer-events: none;
  transition: opacity 190ms ease;
  z-index: -1;
}

.board.revealed .glass {
  opacity: 1;
}

/* 背景常显：锁定或鼠标不在时也把背景留着（此时窗口是"看得见但点不到"） */
.board.glass-always .glass {
  opacity: 1;
}

.content {
  position: relative;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
}

/* 舞台必须拿到「确定的高度」：里面的行数、居中位置都按真实高度算 */
.stage-wrap {
  flex: 1;
  min-height: 0;
  position: relative;
}

/* 信息栏在底部时，整块排到舞台后面去（顺序由 inline order 决定，这里不写死） */

/* 封面在右：靠 order 排到最后，不用 row-reverse（反转会把控件的顺序也带着翻） */
/* 封面永远占最外侧：最左 0 / 最右 4，控件只能排进内侧 */
.card-head.cover-right .cover {
  order: 4;
}

.card-head.cover-left .cover {
  order: 0;
}

.card-head .meta {
  order: 2;
}

.card-head {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 46px;
  flex: none;
  padding: 0 20px;
  box-sizing: border-box;
}

.cover {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  background: linear-gradient(135deg, #3a3f52, #1d2030);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1);
  flex: none;
}

/* 真封面：铺满并居中裁切；描边保留一圈，浅色封面也能和玻璃底分开 */
.cover.has-img {
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;
}

.meta {
  min-width: 0;
  flex: 1;
}

/* 媒体按钮组：位置由 inline order/margin 决定，这里只管不被压缩 */
.media-slot {
  flex: none;
}

/* hover 模式：鼠标不在词板上时控件淡出。
   锁定时本来就不显形，所以这条只在解锁态有实际效果。 */
.content.media-hover .media-slot,
.content.media-hover .mprogress {
  opacity: 0;
  pointer-events: none;
  transition: opacity 160ms ease;
}

.board.revealed .content.media-hover .media-slot,
.board.revealed .content.media-hover .mprogress {
  opacity: 1;
  pointer-events: auto;
}

/* 暂停超时：只藏歌词，信息栏和控件看设置 */
.stage-wrap {
  transition: opacity 240ms ease;
}

.board.pause-hidden .stage-wrap {
  opacity: 0;
}

.board.pause-hidden-all .card-head,
.board.pause-hidden-all .mprogress {
  opacity: 0;
  pointer-events: none;
  transition: opacity 240ms ease;
}

.title {
  font-size: 13px;
  font-weight: 500;
  color: var(--info-color, #ffffff);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.artist {
  font-size: 11px;
  color: var(--info-color, #ffffff);
  opacity: 0.62;
  margin-top: 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.empty {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 0 24px;
}

.empty-text {
  font-size: 13px;
  color: rgba(255, 255, 255, 0.4);
  text-align: center;
  line-height: 1.6;
}

.lock {
  position: absolute;
  right: 10px;
  bottom: 9px;
  width: 26px;
  height: 26px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: 7px;
  background: rgba(255, 255, 255, 0.09);
  color: rgba(255, 255, 255, 0.82);
  cursor: pointer;
  transition:
    background 140ms ease,
    color 140ms ease;
}

.lock:hover {
  background: rgba(255, 255, 255, 0.2);
  color: #fff;
}

/* 卡片标题在底部时，按钮让到右上角 */
.lock.top {
  top: 9px;
  bottom: auto;
}

.lock svg {
  width: 15px;
  height: 15px;
}

.ctx {
  position: absolute;
  right: 8px;
  bottom: 40px;
  min-width: 132px;
  padding: 5px;
  border-radius: 9px;
  background: rgba(26, 27, 33, 0.96);
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.09),
    0 12px 32px rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
  gap: 1px;
  z-index: 20;
}

.ctx button {
  height: 27px;
  padding: 0 9px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: rgba(255, 255, 255, 0.84);
  font-size: 12px;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
}

.ctx button:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

.ctx button.danger:hover {
  background: rgba(255, 92, 92, 0.18);
  color: #ff9d9d;
}

.sep {
  height: 1px;
  margin: 4px 6px;
  background: rgba(255, 255, 255, 0.08);
}

.pop-enter-active,
.pop-leave-active {
  transition:
    opacity 120ms ease,
    transform 120ms ease;
}

.pop-enter-from,
.pop-leave-to {
  opacity: 0;
  transform: translateY(4px) scale(0.98);
}
</style>

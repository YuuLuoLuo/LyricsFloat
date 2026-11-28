<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import LyricStage from './LyricStage.vue'
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

onMounted(async () => {
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

/* ------------------------------------------------------------ 歌词信息 */

/**
 * 「显示歌词信息」三项独立：整条在顶部/底部、封面在文字左/右、
 * 文本在剩余空间里左/中/右。文本隐藏时只剩封面，封面也隐藏时退化成纯歌词。
 */
const infoOn = computed(() => settings.info.show)
const cardPos = computed(() => settings.info.pos)
const cardCover = computed(() => settings.info.cover)
const cardText = computed(() => settings.info.text)
const cardHead = computed(
  () => infoOn.value && (cardCover.value !== 'hidden' || cardText.value !== 'hidden'),
)

/** 没有封面时整块按文本位置对齐 */
const headStyle = computed(() => {
  if (cardCover.value !== 'hidden') return {}
  const t = cardText.value
  return {
    justifyContent: t === 'center' ? 'center' : t === 'right' ? 'flex-end' : 'flex-start',
  }
})

const metaStyle = computed(() => ({
  textAlign: cardText.value === 'hidden' ? 'left' : cardText.value,
}))

/** 播放器给了封面就铺上去，没给就保持默认的渐变色块 */
const coverStyle = computed(() =>
  runtime.trackCover ? { backgroundImage: `url(${runtime.trackCover})` } : undefined,
)


/** 信息栏挪到底部时，锁定按钮也换到右上角，别和底部那行挤在一起 */
const lockOnTop = computed(() => cardHead.value && cardPos.value === 'bottom')
</script>

<template>
  <div
    class="board"
    :class="{ revealed, locked: runtime.locked, 'glass-always': settings.glass.always }"
    @pointerdown="onPointerDown"
    @contextmenu="onContextMenu"
  >
    <div class="glass" :style="glassStyle"></div>

    <div class="content" :class="{ 'card-mode': infoOn }">
      <div
        v-if="cardHead"
        class="card-head"
        :class="[`pos-${cardPos}`, `cover-${cardCover}`]"
        :style="headStyle"
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
      </div>

      <div class="stage-wrap">
        <LyricStage v-if="stageDoc" :lines="stageDoc.lines" :time="stageTime" />
        <div v-else class="empty">
          <div class="empty-text">{{ runtime.loading ? '正在获取歌词…' : runtime.notice }}</div>
        </div>
      </div>
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

/* 信息栏在底部时，整块排到舞台后面去（顺序反转，不挪 DOM） */
.card-head.pos-bottom {
  order: 2;
}

/* 封面在右：反转主轴，就是 [文本][封面] */
.card-head.cover-right {
  flex-direction: row-reverse;
}

.card-head.cover-left {
  justify-content: flex-start;
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

.title {
  font-size: 13px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.92);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.artist {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.5);
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

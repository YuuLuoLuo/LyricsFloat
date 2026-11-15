<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
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
  startDragging,
} from '../core/bridge'

const revealed = computed(() => glassVisible())

const glassStyle = computed(() => {
  const { tint, alpha } = settings.glass
  return { background: `rgba(${hexToRgb(tint)}, ${alpha})` }
})

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
  await startDragging()
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
const cardMode = computed(() => settings.view.mode === 'card')

/* ------------------------------------------------------------ 卡片布局 */

/**
 * 卡片模式的封面 / 标题位置都由设置决定：
 * 封面可以在左、在右或干脆不显示，标题可以在顶部、底部或不显示。
 * 两者都不显示时就退化成纯歌词，不再占那 46px。
 */
const cardCover = computed(() => settings.card.cover)
const cardTitle = computed(() => settings.card.title)
const cardHead = computed(
  () => cardMode.value && (cardCover.value !== 'hidden' || cardTitle.value !== 'hidden'),
)

/** 标题挪到底部时，锁定按钮也换到右上角，别和底部那行标题/封面挤在一起 */
const lockOnTop = computed(() => cardHead.value && cardTitle.value === 'bottom')
</script>

<template>
  <div
    class="board"
    :class="{ revealed, locked: runtime.locked }"
    @pointerdown="onPointerDown"
    @contextmenu="onContextMenu"
  >
    <div class="glass" :style="glassStyle"></div>

    <div class="content" :class="{ 'card-mode': cardMode }">
      <div
        v-if="cardHead"
        class="card-head"
        :class="[`title-${cardTitle}`, `cover-${cardCover}`]"
      >
        <div v-if="cardCover !== 'hidden'" class="cover"></div>
        <div v-if="cardTitle !== 'hidden'" class="meta">
          <div class="title">{{ runtime.trackTitle || '未在播放' }}</div>
          <div class="artist">{{ runtime.trackArtist || '等待播放器' }}</div>
        </div>
      </div>

      <div class="stage-wrap">
        <LyricStage v-if="stageDoc" :lines="stageDoc.lines" :time="runtime.time" />
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

/* 标题在底部时，头部排到舞台后面去（顺序反转，不挪 DOM） */
.card-head.title-bottom {
  order: 2;
}

/* 封面在右：反转主轴再贴右端，就是 [标题][封面] */
.card-head.cover-right {
  flex-direction: row-reverse;
  justify-content: flex-start;
}

.card-head.cover-left {
  justify-content: flex-start;
}

.card-head.cover-hidden {
  justify-content: center;
  text-align: center;
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

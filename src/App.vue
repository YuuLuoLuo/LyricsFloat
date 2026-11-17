<script setup lang="ts">
import { onBeforeUnmount, onMounted, watch } from 'vue'
import Board from './components/Board.vue'
import { clock } from './core/clock'
import { runtime, settings, pullSettings, saveSettings } from './core/store'
import { initPlayer, reapplyOffset } from './core/player'
import { initSettingsFile } from './core/persist'
import {
  isTauri,
  listenNative,
  restorePosition,
  setBoardParked,
  showSelf,
  applyBoard,
  setLocked,
  setAlwaysOnTop,
} from './core/bridge'

let raf = 0
let offNative: (() => void) | null = null
let poll: ReturnType<typeof setInterval> | null = null
let resizeTimer: ReturnType<typeof setTimeout> | null = null
let parkTimer: ReturnType<typeof setTimeout> | null = null
let parked = false
let lastFrame = 0

function loop(now: number) {
  const dt = lastFrame ? Math.min(64, now - lastFrame) : 16
  lastFrame = now
  // 偏差衰减必须每帧推进，否则真实进度永远追不上外推
  clock.tick(dt)
  runtime.time = clock.read()
  raf = requestAnimationFrame(loop)
}

/**
 * 无播放时自动隐藏。
 *
 * 判定用 `runtime.doc`：没有播放器、或者拿到歌但没匹配到歌词，都是 null，
 * 两种情况都该把窗口收起来（用户要的就是"没东西看的时候别占着"）。
 * 换歌的瞬间 doc 会短暂变空，所以藏之前等 900ms，避免每切一首歌闪一下。
 */
function syncPark(): void {
  const want = settings.behavior.hideWhenIdle && !runtime.doc
  if (want === parked) return
  if (parkTimer) clearTimeout(parkTimer)
  parkTimer = setTimeout(
    () => {
      if (!isTauri || want === parked) return
      parked = want
      void setBoardParked(want)
    },
    want ? 900 : 0,
  )
}

/** 设置页改完尺寸/玻璃色 -> 由词板窗口统一落地，避免两个窗口抢着 resize */
function scheduleResize() {
  if (!isTauri) return
  if (resizeTimer) clearTimeout(resizeTimer)
  if (parkTimer) clearTimeout(parkTimer)
  resizeTimer = setTimeout(() => void applyBoard(settings), 40)
}

watch(
  () => settings.lyric.offsetMs,
  () => reapplyOffset(),
)

watch(
  () => [settings.window.width, settings.window.height, settings.glass.tint, settings.glass.alpha],
  scheduleResize,
)

watch(() => [settings.behavior.hideWhenIdle, !!runtime.doc], syncPark)

onMounted(async () => {
  raf = requestAnimationFrame(loop)
  // 先把磁盘上的设置接上（首次运行会顺手落一份），再拿尺寸去开窗口
  await initSettingsFile()
  const stopPlayer = await initPlayer()
  offNative = stopPlayer

  if (isTauri) {
    const off = await listenNative({
      onReveal: (v) => {
        runtime.revealed = v
      },
      onLocked: (v) => {
        runtime.locked = v
        if (v) runtime.revealed = false
        if (settings.behavior.locked !== v) {
          settings.behavior.locked = v
          saveSettings()
        }
      },
    })
    offNative = () => {
      stopPlayer()
      off()
    }

    await applyBoard(settings)
    await setAlwaysOnTop(settings.behavior.alwaysOnTop)
    if (settings.behavior.locked) await setLocked(true)
    await restorePosition()
    await showSelf()
    // 启动时就该按当前状态决定要不要藏（比如开机自启时根本没有播放器）
    syncPark()

    // 跨窗口同步：storage 事件即时，轮询兜住极端情况
    window.addEventListener('storage', pullSettings)
    poll = setInterval(pullSettings, 900)
  } else {
    runtime.revealed = true
  }

  window.addEventListener('keydown', onKey)
})

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') runtime.menuOpen = false
}

onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  offNative?.()
  if (poll) clearInterval(poll)
  if (resizeTimer) clearTimeout(resizeTimer)
  window.removeEventListener('storage', pullSettings)
  window.removeEventListener('keydown', onKey)
  saveSettings()
})
</script>

<template>
  <Board />
</template>

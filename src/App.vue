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
  logLine,
  restorePosition,
  setBoardParked,
  setPlayerPin,
  setPlayerPref,
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
 * 无播放时关闭词板。
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

// 歌词来源：词板窗口也推一次，保证「只开词板、不打开设置」时偏好同样生效
watch(
  () => settings.lyric.player,
  (v) => void setPlayerPref(v),
)
watch(
  () => settings.lyric.pinPlayer,
  (v) => void setPlayerPin(v),
)

onMounted(async () => {
  raf = requestAnimationFrame(loop)
  // 先把磁盘上的设置接上（首次运行会顺手落一份），再拿尺寸去开窗口
  await initSettingsFile()
  // 设置摘要进日志：复盘"某个设置下才出问题"时最省事的一条线索
  void logLine(
    'INFO',
    `设置：${settings.window.width}x${settings.window.height} 字号${settings.appearance.fontSize} ` +
      `行距${settings.appearance.lineHeight} 对齐${settings.appearance.align} ` +
      `旋转${settings.view.rotate ? '开' : '关'} 翻译${settings.view.translation ? '开' : '关'} ` +
      `逐字${settings.view.karaoke ? '开' : '关'} 信息${settings.info.show ? '开' : '关'} ` +
      `锁定${settings.behavior.locked ? '开' : '关'} 关闭词板${settings.behavior.hideWhenIdle ? '开' : '关'} ` +
      `来源${settings.lyric.player || '自动'}${settings.lyric.pinPlayer ? '(锁定)' : ''}`,
  )
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

    await setPlayerPref(settings.lyric.player)
    await setPlayerPin(settings.lyric.pinPlayer)
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

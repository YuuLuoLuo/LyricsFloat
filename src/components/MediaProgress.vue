<script setup lang="ts">
/**
 * 进度条。未播放部分是一根细线，已播放部分更粗、带发光。
 *
 * 拖拽时用本地比例显示，松手才真正 seek —— 等播放器回报位置会有明显延迟，
 * 拖的时候跟手一点心里才踏实。
 */
import { computed, ref } from 'vue'
import { runtime } from '../core/store'
import { mediaSeek } from '../core/bridge'

/** 已播放部分的颜色。由 Board.vue 算：默认白色，开启取色时用封面主色 */
const props = defineProps<{ color: string }>()

const dragging = ref(false)
const dragRatio = ref(0)

/** 发光用同一颜色的低透明度版本，所以得把 #rrggbb 拆成 r,g,b */
const fillStyle = computed(() => {
  const hex = props.color.replace('#', '')
  const full =
    hex.length === 3
      ? hex
          .split('')
          .map((c) => c + c)
          .join('')
      : hex
  const n = parseInt(full, 16)
  const rgb = Number.isNaN(n)
    ? '255, 255, 255'
    : `${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}`
  return {
    background: props.color,
    boxShadow: `0 0 7px rgba(${rgb}, 0.6), 0 0 14px rgba(${rgb}, 0.24)`,
  }
})

const ratio = computed(() => {
  if (dragging.value) return dragRatio.value
  if (!runtime.duration) return 0
  return Math.min(1, Math.max(0, runtime.position / runtime.duration))
})

function ratioAt(e: PointerEvent): number {
  const el = e.currentTarget as HTMLElement
  const r = el.getBoundingClientRect()
  if (r.width <= 0) return 0
  return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width))
}

function onDown(e: PointerEvent): void {
  if (!runtime.duration) return
  dragging.value = true
  dragRatio.value = ratioAt(e)
  try {
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  } catch {
    /* 个别环境不支持指针捕获，拖拽会差一点但还能用 */
  }
}

function onMove(e: PointerEvent): void {
  if (!dragging.value) return
  dragRatio.value = ratioAt(e)
}

function onUp(e: PointerEvent): void {
  if (!dragging.value) return
  dragging.value = false
  const pos = dragRatio.value * runtime.duration
  void mediaSeek(pos)
  try {
    ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
  } catch {
    /* ignore */
  }
}
</script>

<template>
  <div
    class="track"
    :class="{ dragging }"
    @pointerdown.stop="onDown"
    @pointermove.stop="onMove"
    @pointerup.stop="onUp"
    @pointercancel.stop="onUp"
    @click.stop
  >
    <div class="fill" :style="{ width: `${ratio * 100}%`, ...fillStyle }"></div>
  </div>
</template>

<style scoped>
/* 线只有 2px，命中区给到 10px，不然根本拖不住 */
.track {
  position: relative;
  height: 10px;
  display: flex;
  align-items: center;
  cursor: pointer;
  flex: none;
}

.track::before {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  border-radius: 1px;
  background: rgba(255, 255, 255, 0.16);
}

.fill {
  position: absolute;
  left: 0;
  height: 3px;
  border-radius: 2px;
  pointer-events: none;
  transition: width 140ms linear;
}

/* 拖动时别插值，否则手指和线会打架 */
.track.dragging .fill {
  transition: none;
}
</style>

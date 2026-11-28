<script setup lang="ts">
/**
 * 媒体操控：上一曲 / 播放暂停 / 下一曲。
 *
 * 只负责发命令 —— 位置由 Board.vue 按「歌词信息文本在哪一侧」摆，
 * 这里不做任何布局判断。
 */
import { runtime } from '../core/store'
import { mediaNext, mediaPrev, mediaToggle } from '../core/bridge'

function prev(): void {
  void mediaPrev()
}

function toggle(): void {
  void mediaToggle()
}

function next(): void {
  void mediaNext()
}
</script>

<template>
  <div class="media-bar">
    <button class="mbtn" title="上一曲" @click.stop="prev">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M7 6v12" />
        <path d="M18 6.5 10 12l8 5.5z" fill="currentColor" stroke="none" />
      </svg>
    </button>
    <button class="mbtn play" :title="runtime.playing ? '暂停' : '播放'" @click.stop="toggle">
      <svg
        v-if="runtime.playing"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.2"
        stroke-linecap="round"
      >
        <path d="M9.5 6v12" />
        <path d="M14.5 6v12" />
      </svg>
      <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M8.5 5.5 19 12 8.5 18.5z" fill="currentColor" stroke="none" />
      </svg>
    </button>
    <button class="mbtn" title="下一曲" @click.stop="next">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M17 6v12" />
        <path d="M6 6.5 14 12l-8 5.5z" fill="currentColor" stroke="none" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.media-bar {
  display: flex;
  align-items: center;
  gap: 1px;
  flex: none;
}

.mbtn {
  width: 26px;
  height: 26px;
  display: grid;
  place-items: center;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: rgba(255, 255, 255, 0.72);
  cursor: pointer;
  transition:
    background 120ms ease,
    color 120ms ease;
}

.mbtn:hover {
  background: rgba(255, 255, 255, 0.13);
  color: #fff;
}

.mbtn:active {
  background: rgba(255, 255, 255, 0.2);
}

.mbtn svg {
  width: 15px;
  height: 15px;
}

/* 播放/暂停是主操作，给一个常驻的浅底 */
.mbtn.play {
  background: rgba(255, 255, 255, 0.1);
}

.mbtn.play:hover {
  background: rgba(255, 255, 255, 0.22);
}
</style>

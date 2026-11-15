<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    modelValue: string
    fonts: string[]
    /** 置顶的常用字体（只显示系统里确实装了的） */
    curated?: string[]
  }>(),
  { curated: () => [] },
)

const emit = defineEmits<{ (e: 'update:modelValue', v: string): void }>()

const open = ref(false)
const query = ref('')
const trigger = ref<HTMLButtonElement | null>(null)
const popup = ref<HTMLElement | null>(null)
const search = ref<HTMLInputElement | null>(null)
const active = ref(-1)
const geo = ref({ top: 0, left: 0, width: 0, up: false })

const PREVIEW = '永远相信 美好的事'

const matched = computed(() => {
  const k = query.value.trim().toLowerCase()
  if (!k) return props.fonts
  return props.fonts.filter((f) => f.toLowerCase().includes(k))
})

/** 没搜索时把常用字体单独分组放最上面，省得每次都在几百个里翻 */
const groups = computed(() => {
  if (query.value.trim()) return [{ label: '', items: matched.value }]
  const set = new Set(props.fonts)
  const common = props.curated.filter((f) => set.has(f))
  if (!common.length) return [{ label: '', items: matched.value }]
  const rest = matched.value.filter((f) => !common.includes(f))
  return [
    { label: '常用', items: common },
    { label: `全部字体 · ${rest.length}`, items: rest },
  ]
})

/** 扁平化，方便上下键逐项移动 */
const flat = computed(() => groups.value.flatMap((g) => g.items))

function place(): void {
  const el = trigger.value
  if (!el) return
  const r = el.getBoundingClientRect()
  // 面板固定在视口上（Teleport 到 body），这样不会被设置页的滚动容器裁掉
  const H = 328
  const below = window.innerHeight - r.bottom
  const up = below < H && r.top > below
  geo.value = {
    top: up ? r.top - 6 : r.bottom + 6,
    left: r.left,
    width: Math.max(240, r.width),
    up,
  }
}

function openNow(): void {
  place()
  open.value = true
  query.value = ''
  active.value = Math.max(0, flat.value.indexOf(props.modelValue))
  void nextTick(() => {
    search.value?.focus()
    scrollIntoView()
  })
}

function close(): void {
  open.value = false
}

function toggle(): void {
  if (open.value) close()
  else openNow()
}

function choose(font: string): void {
  emit('update:modelValue', font)
  close()
}

function scrollIntoView(): void {
  const list = popup.value?.querySelector('.opt.on') as HTMLElement | null
  list?.scrollIntoView({ block: 'nearest' })
}

function move(step: number): void {
  if (!flat.value.length) return
  active.value = Math.min(flat.value.length - 1, Math.max(0, active.value + step))
  void nextTick(scrollIntoView)
}

function onKey(e: KeyboardEvent): void {
  if (!open.value) {
    if (e.key === 'Enter' || e.key === 'ArrowDown') {
      e.preventDefault()
      openNow()
    }
    return
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    close()
  } else if (e.key === 'ArrowDown') {
    e.preventDefault()
    move(1)
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    move(-1)
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const f = flat.value[active.value]
    if (f) choose(f)
  }
}

function onDocDown(e: PointerEvent): void {
  if (!open.value) return
  const t = e.target as Node
  if (popup.value?.contains(t) || trigger.value?.contains(t)) return
  close()
}

function onViewportChange(): void {
  if (open.value) place()
}

watch(query, () => {
  active.value = 0
})

onMounted(() => {
  window.addEventListener('pointerdown', onDocDown, true)
  window.addEventListener('resize', onViewportChange)
  window.addEventListener('scroll', onViewportChange, true)
})

onBeforeUnmount(() => {
  window.removeEventListener('pointerdown', onDocDown, true)
  window.removeEventListener('resize', onViewportChange)
  window.removeEventListener('scroll', onViewportChange, true)
})
</script>

<template>
  <div class="fs">
    <button
      ref="trigger"
      type="button"
      class="trigger"
      :class="{ open }"
      :title="modelValue"
      @click="toggle"
      @keydown="onKey"
    >
      <span class="name" :style="{ fontFamily: `'${modelValue}', sans-serif` }">{{ modelValue }}</span>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
        <path d="M7 10l5 5 5-5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="popup"
        class="pop"
        :class="{ up: geo.up }"
        :style="{ top: `${geo.top}px`, left: `${geo.left}px`, width: `${geo.width}px` }"
      >
        <div class="search">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <circle cx="11" cy="11" r="6.2" />
            <path d="M15.6 15.6L20 20" stroke-linecap="round" />
          </svg>
          <input
            ref="search"
            v-model="query"
            type="text"
            placeholder="搜索字体…"
            spellcheck="false"
            @keydown="onKey"
          />
        </div>

        <div class="list">
          <template v-for="(g, gi) in groups" :key="gi">
            <div v-if="g.label" class="group">{{ g.label }}</div>
            <button
              v-for="f in g.items"
              :key="f"
              type="button"
              class="opt"
              :class="{ on: f === modelValue, act: flat[active] === f }"
              @click="choose(f)"
              @pointerenter="active = flat.indexOf(f)"
            >
              <span class="opt-name">{{ f }}</span>
              <span class="opt-demo" :style="{ fontFamily: `'${f}', sans-serif` }">{{ PREVIEW }}</span>
              <svg
                v-if="f === modelValue"
                class="tick"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.2"
              >
                <path d="M5 12.5l4.5 4.5L19 7" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </template>
          <div v-if="!matched.length" class="empty">没有匹配的字体</div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.fs {
  flex: 1;
  min-width: 0;
}

.trigger {
  width: 100%;
  height: 28px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px;
  border-radius: 7px;
  border: 1px solid rgba(255, 255, 255, 0.11);
  background: rgba(255, 255, 255, 0.05);
  color: rgba(255, 255, 255, 0.84);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  text-align: left;
}

.trigger:hover,
.trigger.open {
  background: rgba(255, 255, 255, 0.09);
  border-color: rgba(255, 255, 255, 0.2);
}

.trigger .name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.trigger svg {
  width: 14px;
  height: 14px;
  flex: none;
  opacity: 0.6;
  transition: transform 140ms ease;
}

.trigger.open svg {
  transform: rotate(180deg);
}

.pop {
  position: fixed;
  z-index: 900;
  display: flex;
  flex-direction: column;
  max-height: 328px;
  border-radius: 10px;
  background: rgba(30, 32, 39, 0.985);
  box-shadow:
    0 0 0 1px rgba(255, 255, 255, 0.1),
    0 18px 46px rgba(0, 0, 0, 0.6);
  overflow: hidden;
  animation: pop-in 120ms ease;
}

.pop.up {
  transform: translateY(-100%);
}

@keyframes pop-in {
  from {
    opacity: 0;
    transform: translateY(-4px) scale(0.99);
  }
}

.pop.up {
  animation-name: pop-in-up;
}

@keyframes pop-in-up {
  from {
    opacity: 0;
    transform: translateY(-100%) translateY(4px) scale(0.99);
  }
}

.search {
  flex: none;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 8px 10px;
  box-shadow: inset 0 -1px 0 rgba(255, 255, 255, 0.07);
}

.search svg {
  width: 13px;
  height: 13px;
  flex: none;
  color: rgba(255, 255, 255, 0.34);
}

.search input {
  flex: 1;
  min-width: 0;
  height: 22px;
  border: none;
  outline: none;
  background: transparent;
  color: #fff;
  font-size: 12px;
  font-family: inherit;
}

.search input::placeholder {
  color: rgba(255, 255, 255, 0.28);
}

.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px;
}

.list::-webkit-scrollbar {
  width: 9px;
}

.list::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.16);
  border-radius: 5px;
  border: 2px solid transparent;
  background-clip: padding-box;
}

.group {
  padding: 7px 8px 5px;
  font-size: 10.5px;
  letter-spacing: 0.08em;
  color: rgba(255, 255, 255, 0.3);
}

.opt {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 30px;
  padding: 0 8px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: rgba(255, 255, 255, 0.78);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
  text-align: left;
}

.opt.act {
  background: rgba(255, 255, 255, 0.08);
}

.opt.on {
  background: rgba(127, 212, 255, 0.14);
  color: #bfe9ff;
}

.opt-name {
  flex: none;
  max-width: 46%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 每个选项用它自己的字体渲染一句样例，选之前就知道长什么样 */
.opt-demo {
  flex: 1;
  min-width: 0;
  text-align: right;
  font-size: 14px;
  color: rgba(255, 255, 255, 0.44);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.opt.on .opt-demo {
  color: rgba(191, 233, 255, 0.66);
}

.tick {
  width: 13px;
  height: 13px;
  flex: none;
  color: #7fd4ff;
}

.empty {
  padding: 18px 10px;
  text-align: center;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.32);
}
</style>

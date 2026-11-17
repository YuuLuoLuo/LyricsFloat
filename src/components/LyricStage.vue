<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { settings } from '../core/store'
import { findLineIndex } from '../core/parser'
import type { LyricLine, LyricWord } from '../core/types'

const props = withDefaults(
  defineProps<{
    lines: LyricLine[]
    time: number
    /** 固定高度（设置页预览用）；0 = 跟随父容器实际高度 */
    height?: number
  }>(),
  { height: 0 },
)

const a = computed(() => settings.appearance)
/** 四个样式开关，互相独立 */
const rotateOn = computed(() => settings.view.rotate)
const showTrans = computed(() => settings.view.translation)
const karaokeOn = computed(() => settings.view.karaoke)

/** 左右内边距（.line-inner 的 padding），算可用宽度时要扣掉 */
const H_PAD = 26

/**
 * 高度和宽度都必须用「渲染出来的真实尺寸」，不能拿设置里的数值算：
 * 窗口改尺寸、DPI 缩放、父容器留白都会让两者对不上，
 * 对不上的结果就是行数多算、整块歌词偏下被切掉。
 */
const root = ref<HTMLElement | null>(null)
const measured = ref(0)
const measuredW = ref(0)
let ro: ResizeObserver | null = null

onMounted(() => {
  const el = root.value
  if (!el) return
  measured.value = el.clientHeight
  measuredW.value = el.clientWidth
  if (typeof ResizeObserver === 'undefined') return
  ro = new ResizeObserver(() => {
    measured.value = el.clientHeight
    measuredW.value = el.clientWidth
  })
  ro.observe(el)
})

onBeforeUnmount(() => ro?.disconnect())

const stageHeight = computed(() => {
  if (props.height > 0) return props.height
  if (measured.value > 0) return measured.value
  return Math.max(0, settings.window.height)
})

const stageWidth = computed(() => {
  if (measuredW.value > 0) return measuredW.value
  return Math.max(0, settings.window.width)
})

const boxStyle = computed(() =>
  props.height > 0 ? { height: `${props.height}px` } : { height: '100%' },
)

/**
 * 当前行索引。三种状态：
 * - `-1`：前奏，第一句还没开口
 * - `0..n-1`：正常，某一句正在唱
 * - `n`：尾奏，最后一句也唱完了
 *
 * 前后两端都不点亮任何一行 —— 歌曲开头和结尾时"当前格"是空的，
 * 首句/尾句只作为普通句子出现，不会被当成正在唱的那句。
 *
 * 带迟滞：时间要明确越过下一句起点（多走 HYST_MS）才换行，往回同理。
 * 不加迟滞的话，时钟在句子边界上抖几十毫秒，高亮就会在相邻两句之间
 * 反复横跳 —— 也就是"上下两句抽搐"。两端同理，否则末尾会反复闪。
 */
const HYST_MS = 150
const cur = ref(-1)
let curState = -1

/** 有没有"正在唱的那一句" */
const hasCur = computed(() => cur.value >= 0 && cur.value < props.lines.length)

/** 直接定位（换歌词时用），同样认前后两端的空档 */
function locate(lines: LyricLine[], t: number): number {
  const n = lines.length
  if (!n) return -1
  if (t < lines[0].start) return -1
  if (t >= lines[n - 1].end) return n
  return findLineIndex(lines, t)
}

function stepLine(prev: number, lines: LyricLine[], t: number): number {
  const n = lines.length
  if (!n) return -1
  const first = lines[0]
  const last = lines[n - 1]

  // 尾奏：最后一句唱完之后留一片空白，不再点亮末句
  if (t >= last.end + HYST_MS) return n
  if (prev === n) {
    // 还在末尾附近就保持空白；只有在真的往回拖时才回到最后一句
    return t >= last.end - HYST_MS ? n : n - 1
  }
  // 前奏：第一句还没到，当前格也空着
  if (t < first.start - HYST_MS) return -1

  let i = prev < 0 ? 0 : Math.min(prev, n - 1)
  while (i + 1 < n && t >= lines[i + 1].start + HYST_MS) i++
  while (i > 0 && t < lines[i].start - HYST_MS) i--
  return i
}

// 换歌词（换歌）时直接定位，不参与迟滞
watch(
  () => props.lines,
  (lines) => {
    curState = locate(lines, props.time)
    cur.value = curState
  },
  { immediate: true },
)

watch(
  () => props.time,
  (t) => {
    const next = stepLine(curState, props.lines, t)
    if (next === curState) return
    curState = next
    cur.value = next
  },
  { immediate: true },
)

/**
 * 单行占位高度。
 *
 * 开翻译时每一行下面还挂着译文，按纯歌词的行距排会挤在一起，放大一档。
 * 旋转样式刻意压紧行距：上下句要"叠"在当前句下面才像指针扫过；
 * 此时若还开着翻译，每一格都是「原文 + 译文」两块，压太紧会互相盖住，
 * 所以收缩幅度比纯歌词小一点。
 */
const unitH = computed(() => {
  // 行高按「自适应缩完之后」的字号算：字号开到很大时每句都会被缩小，
  // 若还按原始字号排行距，两句之间就空出一大片。
  // 下限仍是用户设的行距，正常字号下排版不变。
  const lh = Math.max(1, a.value.lineHeight, denseSize.value * 1.25)
  const trans = showTrans.value ? 1.5 : 1
  const tight = rotateOn.value ? (showTrans.value ? 0.86 : 0.78) : 1
  return lh * trans * tight
})

/**
 * 行数完全由高度决定：能塞几行就显示几行。
 * 用户调词板高度就是在调"看得见几句" —— 高度压到只放得下一句，
 * 就是旧版的「单行聚焦」，不需要再单独选行数。
 */
const capacity = computed(() => {
  const u = Math.max(1, unitH.value)
  return Math.max(1, Math.floor(stageHeight.value / u))
})

/**
 * 单行实际占位。高度不够时按实际空间压缩行距 ——
 * 宁可挤一点，也不要让上下两边各切掉半行。
 */
const rowH = computed(() => Math.min(unitH.value, stageHeight.value / capacity.value))

/**
 * 可见区间：以当前句为锚点。
 *
 * `cur` 可能是 -1（前奏）或 n（尾奏），这时"当前句"落在所有句子之外，
 * 轨道整体多挪一行 —— 正中那一格自然就空出来了。
 *
 * - 垂直居中：上下各铺半个窗口（多渲染的会被 overflow 裁掉）
 * - 顶部对齐：下面要铺满整个窗口，上面留一两句给"刚唱过"的过渡
 */
const range = computed(() => {
  const total = props.lines.length
  const i = cur.value
  const per = Math.max(1, rowH.value)
  const down = Math.ceil(stageHeight.value / per) + 1
  const up =
    a.value.vAlign === 'top' ? 1 : Math.ceil(stageHeight.value / 2 / per) + 1
  return { start: Math.max(0, i - up), end: Math.min(total, i + down + 1) }
})

const rows = computed(() =>
  props.lines.slice(range.value.start, range.value.end).map((line, i) => ({
    line,
    index: range.value.start + i,
  })),
)

const railY = computed(() => {
  // 这里故意用未夹紧的 cur：前奏是 -1、尾奏是 n，
  // 轨道会多滚一行，正好把"当前格"让空。
  const i = cur.value
  const per = rowH.value
  if (a.value.vAlign === 'top') {
    // 贴顶：当前句压在窗口上沿，唱过的句子滚出可视区，下面全是还没唱的。
    // 留一点余量，免得放大后的当前句被上沿切掉。
    const pad = Math.max(0, (per * (Math.max(1, a.value.currentScale) - 1)) / 2) + 2
    return pad - i * per
  }
  // 居中：当前句永远压在窗口正中，上面是被唱过的，下面是还没唱的
  return (stageHeight.value - per) / 2 - i * per
})

/** 每行的缩放/模糊/透明度。文本布局也要用这个 scale，所以抽出来复用 */
interface Look {
  scale: number
  blur: number
  opacity: number
  visible: boolean
  /** 绕对齐边旋转的角度（只用于旋转样式） */
  rotate: number
  /** 层叠顺序：当前句要压在上下句之上 */
  z: number
}

function rowLook(index: number): Look {
  // 距离按「真实的当前行」算：前奏时 cur=-1，第一句天然是"下一句"（d=1），
  // 于是它自动退成普通句子，不会被当成正在唱的那句。
  const d = index - cur.value
  const ad = Math.abs(d)
  const cfg = a.value
  let scale = 1
  let blur = 0
  let opacity = 1
  let rotate = 0
  let z = 0

  if (rotateOn.value) {
    // 指针式：以「对齐边」（看左边或看右边）为圆心，
    // 越远的句子转得越多、糊得越深，并整个叠在当前句下面。
    //
    // 旋转方向必须镜像：右锚点时若是同号，下方的句子会绕着右边
    // 往上甩，看上去像是绕左边转 —— 也就是"锚点在左不在右"。
    const ANGLE = 3
    const dir = cfg.align === 'right' ? -1 : 1
    rotate = d * ANGLE * dir
    scale = ad === 0 ? cfg.currentScale : Math.max(0.62, 0.95 - ad * 0.06)
    // 模糊只求"退到后面去"，别糊成一团反而盖住当前句
    blur = ad === 0 ? 0 : Math.min(6, 1.2 + ad * 1)
    opacity = ad === 0 ? 1 : Math.max(0.12, 0.52 - ad * 0.1)
    z = Math.max(1, 5 - ad)
  } else {
    scale = ad === 0 ? cfg.currentScale : Math.max(0.72, 1 - ad * 0.05)
    blur = ad === 0 ? 0 : Math.min(cfg.blurMax, ad * cfg.blurStep)
    // 以前这里是 1 - ad*fadeStep*2，第三行就淡到 0.04，等于看不见。
    // 现在只按一侧衰减，并留一个下限：远处的句子要"退后"，但必须还能读。
    opacity = ad === 0 ? 1 : Math.max(0.22, 1 - ad * cfg.fadeStep)
  }
  return { scale, blur, opacity, visible: true, rotate, z }
}

/**
 * 缩放/旋转的固定点必须落在**文字那一侧的边上**。
 *
 * 1) 不能一律 `center center`：居左时整行绕中点放大，左端会被推出窗口。
 * 2) 也不能用 `left/right center`：内边距在元素内部，跟着一起放大，
 *    当前行放大 1.4 倍时右端反而往内缩了 10px，和上下句对不齐 ——
 *    看着就是"锚点没钉在边上"。所以按 px 指定：正好压在文字边上。
 */
function scaleOrigin(): string {
  const al = a.value.align
  if (al === 'left') return `${H_PAD}px center`
  if (al === 'right') return `${Math.max(0, stageWidth.value - H_PAD)}px center`
  return 'center center'
}

function lineStyle(index: number) {
  const look = rowLook(index)
  // 只露半个字的行会在窗口边缘拖出一条模糊的亮边（看着就像一条白色虚线）。
  // 干脆整行不画：这些行本来就是淡出/模糊的上下文，少一行没有观感损失。
  const top = index * rowH.value + railY.value
  if (top < -0.5 || top + rowH.value > stageHeight.value + 0.5) {
    look.visible = false
  }
  return {
    top: `${index * rowH.value}px`,
    height: `${rowH.value}px`,
    transformOrigin: scaleOrigin(),
    transform:
      look.rotate !== 0
        ? `rotate(${look.rotate.toFixed(2)}deg) scale(${look.scale.toFixed(3)})`
        : `scale(${look.scale.toFixed(3)})`,
    zIndex: look.z || undefined,
    filter: look.blur > 0 ? `blur(${look.blur.toFixed(2)}px)` : 'none',
    opacity: look.visible ? look.opacity : 0,
    visibility: (look.visible ? 'visible' : 'hidden') as 'visible' | 'hidden',
  }
}

/* --------------------------------------------------------- 自适应字号 */

/**
 * 用 canvas 量文字宽度，而不是往 DOM 里塞一个隐形元素去量：
 * 后者会触发额外布局，词板每秒重排几十次，代价太明显。
 * 量出来的比例按（文本 + 字体 + 字号 + 字重 + 可用宽度）缓存，
 * 同一句在窗口尺寸不变时不会重复量。
 */
let ctx: CanvasRenderingContext2D | null = null
try {
  ctx = document.createElement('canvas').getContext('2d')
} catch {
  ctx = null
}
const fitCache = new Map<string, number>()

function fontStack(): string {
  return `"${a.value.fontFamily}", "Microsoft YaHei UI", "PingFang SC", system-ui, sans-serif`
}

/**
 * 只按宽度缩。行高要由「缩完的字号」决定，不能反过来把行高掺进来
 * （否则行高和字号互相影响，极端字号下行距会算得过大）。
 */
function fitWidth(text: string, size: number, weight: number, scale = 1): number {
  if (!a.value.autoFit || !ctx || !text) return 1
  const avail = (stageWidth.value - H_PAD * 2) / Math.max(0.5, scale)
  if (avail <= 0) return 1

  const key = `w\u0001${text}\u0001${a.value.fontFamily}\u0001${size}\u0001${weight}\u0001${Math.round(avail)}`
  const hit = fitCache.get(key)
  if (hit !== undefined) return hit

  ctx.font = `${weight} ${size}px ${fontStack()}`
  // letter-spacing: 0.01em，再留 2% 余量给抗锯齿与描边
  const w = ctx.measureText(text).width * 1.01 * 1.02
  // 下限压得很低：字号极端大时宁可小，也不能溢出界面
  const r = w > avail ? Math.max(0.12, avail / w) : 1
  if (fitCache.size > 1200) fitCache.clear()
  fitCache.set(key, r)
  return r
}

/**
 * 整首歌里「自适应缩完之后最大」的那句字号。
 *
 * 行距要按它排：用户把字号拉到 200、实际每一句都被缩到 90 时，
 * 行距若还按 200 算，两句之间会空出一大块 —— 正是"行距过大"的来源。
 */
const denseSize = computed(() => {
  const base = a.value.fontSize
  if (!a.value.autoFit) return base
  const scale = Math.max(1, a.value.currentScale)
  let max = 0
  for (const l of props.lines) {
    const s = base * fitWidth(l.text || '♪', base, a.value.fontWeight, scale)
    if (s > max) max = s
  }
  return max > 0 ? max : base
})

/**
 * 这句最终要缩到多少。返回 1 表示不用缩。
 *
 * 横向按可用宽度缩；纵向再确认「缩完的字号 × 行盒」塞得进这一行
 * （只有窗口矮到连一行都放不下时才会用到纵向）。
 */
function fitRatio(text: string, size: number, weight: number, scale: number): number {
  const rw = fitWidth(text, size, weight, scale)
  if (!a.value.autoFit || !ctx) return rw
  const availH = Math.max(1, rowH.value) / Math.max(0.5, scale)
  // 开翻译时这一格里还有译文那一行
  const blockH = size * rw * 1.25 * (showTrans.value ? 1.55 : 1)
  if (blockH <= availH) return rw
  return Math.max(0.1, rw * (availH / blockH))
}

/* ------------------------------------------------------- 描边与阴影 */

function rgba(hex: string, alpha: number): string {
  const h = (hex || '#000000').replace('#', '')
  const full =
    h.length === 3
      ? h
          .split('')
          .map((c) => c + c)
          .join('')
      : h.padEnd(6, '0')
  const n = parseInt(full, 16)
  if (Number.isNaN(n)) return `rgba(0,0,0,${alpha})`
  const r = (n >> 16) & 255
  const g = (n >> 8) & 255
  const b = n & 255
  return `rgba(${r},${g},${b},${Math.max(0, Math.min(1, alpha)).toFixed(3)})`
}

/**
 * 描边用「八向硬阴影」画，而不是 `-webkit-text-stroke`。
 *
 * 后者是居中描在字形轮廓上的：paint-order 一旦没生效，描边就被字身盖掉，
 * 白白调了宽度却看不出任何变化（白字 + 深色描边正是这个现象）。
 * 八向偏移的硬阴影是画在字形**外面**的，一定能看见，也不受字体渲染影响。
 */
function outlineShadows(width: number, color: string): string[] {
  if (width <= 0) return []
  const w = Math.min(6, width)
  const out: string[] = []
  const dirs = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [-1, 0],
    [1, 0],
    [-1, 1],
    [0, 1],
    [1, 1],
  ]
  for (const [dx, dy] of dirs) {
    out.push(`${(dx * w).toFixed(2)}px ${(dy * w).toFixed(2)}px 0 ${color}`)
  }
  // 四个半向补一层，圆角处不至于看出锯齿
  const half = w * 0.55
  for (const [dx, dy] of [
    [-1, -0.4],
    [1, 0.4],
    [-0.4, 1],
    [0.4, -1],
  ]) {
    out.push(`${(dx * half).toFixed(2)}px ${(dy * half).toFixed(2)}px 0 ${color}`)
  }
  return out
}

function textStyle(index: number) {
  const look = rowLook(index)
  const cfg = a.value
  const isCur = hasCur.value && index === cur.value
  const weight = isCur ? cfg.fontWeight : Math.max(300, cfg.fontWeight - 200)
  const text = props.lines[index]?.text || '♪'
  const size = Math.max(8, Math.round(cfg.fontSize * fitRatio(text, cfg.fontSize, weight, look.scale)))

  const shadows: string[] = []
  if (cfg.strokeWidth > 0) {
    shadows.push(...outlineShadows(cfg.strokeWidth, cfg.strokeColor))
  }
  if (cfg.shadow > 0) {
    const base = cfg.shadow * (isCur ? 1 : 0.72)
    // 贴边的紧实阴影负责"轮廓"，大半径的柔影负责"浮起来"
    shadows.push(`0 0 ${Math.max(1, size * 0.06).toFixed(1)}px ${rgba(cfg.shadowColor, Math.min(1, base * 0.95))}`)
    shadows.push(
      `0 ${(size * 0.05).toFixed(1)}px ${(size * 0.45).toFixed(1)}px ${rgba(cfg.shadowColor, base * 0.72)}`,
    )
  }

  return {
    fontSize: `${size}px`,
    fontFamily: fontStack(),
    fontWeight: String(weight),
    color: isCur ? cfg.colorCurrent : cfg.colorOther,
    textShadow: shadows.length ? shadows.join(', ') : 'none',
  }
}

function transStyle(index: number) {
  const look = rowLook(index)
  const base = Math.round(a.value.fontSize * 0.52)
  const text = props.lines[index]?.trans || ''
  const size = Math.max(8, Math.round(base * fitRatio(text, base, 400, look.scale)))
  return { fontSize: `${size}px` }
}

function wordState(w: LyricWord): 'done' | 'active' | 'todo' {
  if (props.time >= w.end) return 'done'
  if (props.time >= w.start) return 'active'
  return 'todo'
}

function displayText(line: LyricLine): string {
  return line.text || '♪'
}
</script>

<template>
  <div ref="root" class="stage" :style="[boxStyle, { '--hl': a.colorHighlight }]">
    <div class="rail" :style="{ transform: `translate3d(0, ${railY}px, 0)` }">
      <div
        v-for="row in rows"
        :key="row.index"
        class="line"
        :class="{ 'is-current': hasCur && row.index === cur }"
        :style="lineStyle(row.index)"
      >
        <div class="line-inner" :style="{ textAlign: a.align }">
          <div class="txt" :style="textStyle(row.index)">
            <template v-if="karaokeOn && hasCur && row.index === cur && row.line.words?.length">
              <span
                v-for="(w, wi) in row.line.words"
                :key="wi"
                class="w"
                :class="wordState(w)"
                >{{ w.text }}</span
              >
            </template>
            <template v-else>{{ displayText(row.line) }}</template>
          </div>
          <div v-if="showTrans && row.line.trans" class="trans" :style="transStyle(row.index)">
            {{ row.line.trans }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.stage {
  position: relative;
  width: 100%;
  overflow: hidden;
  contain: layout paint;
}

.rail {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  transition: transform 480ms cubic-bezier(0.22, 0.61, 0.36, 1);
  will-change: transform;
}

.line {
  position: absolute;
  left: 0;
  right: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  transform-origin: center center;
  transition:
    transform 320ms cubic-bezier(0.22, 0.61, 0.36, 1),
    opacity 320ms ease,
    filter 320ms ease;
  will-change: transform, opacity, filter;
}

.line-inner {
  width: 100%;
  padding: 0 26px;
  box-sizing: border-box;
}

.txt {
  display: block;
  line-height: 1.25;
  white-space: nowrap;
  letter-spacing: 0.01em;
}

.trans {
  margin-top: 4px;
  color: rgba(255, 255, 255, 0.58);
  font-weight: 400;
  line-height: 1.3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 1px 6px rgba(0, 0, 0, 0.55);
}

.w {
  display: inline-block;
  transition:
    color 130ms linear,
    opacity 130ms linear;
}

.w.todo {
  opacity: 0.42;
}

.w.active {
  color: var(--hl, #7fd4ff);
  opacity: 1;
}
</style>

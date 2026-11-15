import type { LyricDoc, LyricLine, LyricWord } from './types'

interface Seed {
  t: number
  d: number
  text: string
  trans?: string
}

const SEEDS: Seed[] = [
  { t: 2000, d: 4200, text: '夜色把城市调成了静音', trans: 'The night turns the city to mute' },
  { t: 6400, d: 4200, text: '我数着路灯一盏一盏地熄', trans: 'Counting streetlights going out' },
  { t: 10800, d: 4400, text: '风从窗缝里借走一点体温', trans: 'Wind borrows warmth through the crack' },
  { t: 15400, d: 4200, text: '留下没说出口的那半句', trans: 'Leaving half a sentence unsaid' },
  { t: 20000, d: 3800, text: '如果明天还没来', trans: 'If tomorrow never comes' },
  { t: 24000, d: 3600, text: '就让我先学会等待', trans: 'Let me learn to wait first' },
  { t: 28000, d: 4200, text: '把心事折成一只纸船', trans: 'Folding my thoughts into a paper boat' },
  { t: 32400, d: 4400, text: '放进这条不肯睡去的河', trans: 'Setting it on this sleepless river' },
  { t: 37000, d: 4000, text: '水面晃着碎掉的月亮', trans: 'The water sways a broken moon' },
  { t: 41200, d: 4200, text: '像我没说完整的愿望', trans: 'Like a wish I never finished' },
  { t: 45600, d: 4000, text: '天亮之前 就这么漂着', trans: 'Drifting like this until dawn' },
  { t: 49800, d: 3600, text: '等一句回答', trans: 'Waiting for an answer' },
]

/** 把一行文本按字数平均切成词级时间，用于演示逐字效果 */
function splitWords(text: string, start: number, end: number): LyricWord[] {
  const chars = Array.from(text)
  const total = Math.max(1, end - start)
  const per = total / chars.length
  return chars.map((ch, i) => ({
    start: Math.round(start + per * i),
    end: Math.round(start + per * (i + 1)),
    text: ch,
  }))
}

export function buildDemoLyric(): LyricDoc {
  const lines: LyricLine[] = SEEDS.map((s) => ({
    start: s.t,
    end: s.t + s.d,
    text: s.text,
    trans: s.trans,
    words: splitWords(s.text, s.t, s.t + s.d),
  }))

  return {
    lines,
    meta: { title: '不肯睡去的河', artist: '歌词板 Demo' },
    hasWordLevel: true,
  }
}

export const DEMO_TRACK = {
  title: '不肯睡去的河',
  artist: '歌词板 Demo',
  album: 'P0 技术验证',
  duration: 56000,
}

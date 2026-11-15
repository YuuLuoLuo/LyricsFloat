export interface LyricWord {
  start: number
  end: number
  text: string
}

export interface LyricLine {
  start: number
  end: number
  text: string
  trans?: string
  words?: LyricWord[]
}

export interface LyricDoc {
  lines: LyricLine[]
  meta: {
    title?: string
    artist?: string
    album?: string
    offset?: number
  }
  hasWordLevel: boolean
}

export type StageMode =
  | 'scroll'
  | 'focus'
  | 'duet'
  | 'karaoke'
  | 'bilingual'
  | 'pivot'
  | 'card'
  | 'plain'

export interface TrackInfo {
  title: string
  artist: string
  album?: string
  duration: number
}

export const MODE_LABELS: Record<StageMode, string> = {
  scroll: '多行滚动',
  focus: '单行聚焦',
  duet: '双行卡农',
  karaoke: '逐字 KTV',
  bilingual: '双语原文',
  pivot: '指针排布',
  card: '卡片模式',
  plain: '纯文本',
}

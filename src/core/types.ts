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

export interface TrackInfo {
  title: string
  artist: string
  album?: string
  duration: number
}

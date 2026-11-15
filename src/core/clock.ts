/**
 * 播放时钟：锚点外推 + 偏差平滑收敛。
 *
 * 设计要点：
 * - 以 (锚点位置, 锚点单调时刻) 外推当前时间，不依赖 setInterval 累加
 * - 收到真实进度时算偏差：超过阈值直接跳到锚点，否则记成 drift 并逐帧衰减
 * - 外部采样频率可以很低（250ms），中间由 rAF 驱动
 *
 * 注意 drift 的衰减必须在 tick() 里做，且 tick 必须被每帧调用 ——
 * 否则偏差永远不会收敛，表现就是歌词越走越偏。
 */
export class PlayClock {
  private anchorPos = 0
  private anchorWall = 0
  private running = false
  private drift = 0
  /** 超过这个偏差直接跳，不慢慢追（用户多半是拖了进度条） */
  private readonly seekThreshold = 400

  reset(pos = 0): void {
    this.anchorPos = pos
    this.anchorWall = performance.now()
    this.drift = 0
  }

  setPlaying(playing: boolean): void {
    if (playing === this.running) return
    if (playing) {
      this.anchorWall = performance.now()
      this.running = true
    } else {
      this.anchorPos = this.read()
      this.running = false
      this.drift = 0
    }
  }

  get isPlaying(): boolean {
    return this.running
  }

  /** 用真实播放进度校准 */
  sync(pos: number): void {
    const diff = pos - this.read()

    // 暂停时只认大幅跳变（拖了进度条）。逐拍 seek 会让时钟一格一格往前蹦，
    // 歌词看着就是在抖。走到这里说明调用方没区分"重复采样"，
    // 一律不许往回拽。
    if (!this.running) {
      this.syncPaused(pos, false)
      return
    }

    // 读数比外推值小：几乎总是"采样延迟 / 播放器 timeline 滞后"，
    // 而不是真的倒回去了。以前把它当误差吸收，等于每 250ms 把时钟往回拽一次，
    // 时间就卡在句子边界上来回跳 —— 也就是"上下两句抽搐"。
    // 只有大跨度倒退（用户拖了进度条）才真的回跳。
    if (diff < 0) {
      if (diff < -8000) this.seek(pos)
      return
    }

    if (diff > this.seekThreshold) {
      this.seek(pos)
      return
    }

    // 小于噪声阈值的差异不校正：校正本身就会让画面轻微地来回动
    if (diff < 120) return
    this.drift = Math.min(400, this.drift + diff * 0.5)
  }

  /**
   * 暂停状态的校准。
   *
   * 暂停后播放器报的位置天然带一点滞后（它只记着上次刷新时的数），
   * 拿它逐拍修正，时钟就会被一次次往回拽 —— 歌词向下一句滑过去再弹回来。
   *
   * 所以分两种情形：
   * - `honorBackward = false`：刚停下、或者播放器重复报同一个值。
   *   这时只认"往前跳"（用户往后拖进度条不常见），绝不往回走。
   * - `honorBackward = true`：暂停状态下位置**变了**，说明用户拖了进度条，
   *   前后都跟。
   */
  syncPaused(pos: number, honorBackward: boolean): void {
    const diff = pos - this.read()
    if (diff > this.seekThreshold) {
      this.seek(pos)
      return
    }
    if (honorBackward && diff < -this.seekThreshold) this.seek(pos)
  }

  /** 每帧调用（dt 单位 ms），让偏差指数衰减到 0 */
  tick(dt: number): void {
    if (!this.drift) return
    if (Math.abs(this.drift) < 1) {
      this.drift = 0
      return
    }
    this.drift *= Math.exp(-dt / 160)
  }

  read(): number {
    const base = this.running
      ? this.anchorPos + (performance.now() - this.anchorWall)
      : this.anchorPos
    return Math.max(0, base + this.drift)
  }

  seek(pos: number): void {
    this.anchorPos = Math.max(0, pos)
    this.anchorWall = performance.now()
    this.drift = 0
  }
}

/** 全局单例 */
export const clock = new PlayClock()

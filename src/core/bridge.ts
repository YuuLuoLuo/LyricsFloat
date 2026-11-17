/**
 * 平台桥接层。
 *
 * 浏览器里跑时全部退化为 no-op：页面背景当作壁纸，玻璃用 backdrop-filter 模拟，
 * 不装 Rust 也能预览观感。Tauri 下磨砂与穿透完全由 Rust 主导。
 */

import type { AppSettings } from './store'

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

type Invoker = <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T>

let invoker: Invoker | null = null

async function call<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isTauri) return null
  try {
    if (!invoker) {
      const mod = await import('@tauri-apps/api/core')
      invoker = mod.invoke as Invoker
    }
    return await invoker<T>(cmd, args)
  } catch (err) {
    console.warn(`[bridge] ${cmd} failed`, err)
    void logLine('WARN', `命令失败：${cmd} → ${String(err)}`)
    return null
  }
}

export interface LyricPayload {
  lrc: string
  trans: string
  yrc: string
  source: string
  matched_title: string
  matched_artist: string
}

export function fetchLyric(title: string, artist: string, duration: number) {
  return call<LyricPayload | null>('fetch_lyric', { title, artist, duration })
}

export interface NowPlayingInfo {
  title: string
  artist: string
  album: string
  duration: number
  position: number
  playing: boolean
}

/** 后开的窗口（设置页）拿不到 track-changed，挂载时主动问一次当前曲目 */
export function getNowPlaying() {
  return call<NowPlayingInfo | null>('now_playing')
}

/** 当前曲目的封面（data URL）。播放器没给封面时返回 null */
export function getCover() {
  return call<string | null>('now_cover')
}

export function setLocked(locked: boolean) {
  return call('set_locked', { locked })
}

export function toggleLocked() {
  return call<boolean>('toggle_lock_cmd')
}

export function setAlwaysOnTop(on: boolean) {
  return call('set_always_on_top', { on })
}

/** 尺寸 / 玻璃色一次性同步给原生层（高频调用，设置页里做了防抖） */
export function applyBoard(s: AppSettings) {
  return call('apply_board', {
    width: s.window.width,
    height: s.window.height,
    tint: s.glass.tint,
    alpha: s.glass.alpha,
  })
}

export function openSettings() {
  return call('open_settings')
}

export function closeSettingsWindow() {
  return call('close_settings')
}

export function quitApp() {
  return call('quit_app')
}

export function setDragging(on: boolean) {
  return call('set_dragging', { on })
}

/** 让词板临时显形一会儿（调宽高时看清面板边界），连续调用会把截止时间往后推 */
export function peekBoard(ms: number) {
  return call('peek_board', { ms })
}

export function getAutostart() {
  return call<boolean>('get_autostart')
}

export function setAutostart(enabled: boolean) {
  return call('set_autostart', { enabled })
}

/* --------------------------------------------------------- 字体与配置 */

/** 系统已安装字体（DirectWrite 枚举，浏览器环境下返回 null） */
export function listFonts() {
  return call<string[]>('list_fonts')
}

/** 设置文件的实际路径（exe 同目录优先，写不了就退到 APPDATA） */
export function settingsPath() {
  return call<string>('settings_path_cmd')
}

export function settingsLoadFile() {
  return call<string | null>('settings_load')
}

export function settingsSaveFile(json: string) {
  return call<string>('settings_save', { json })
}

/** 导出到桌面，返回写出的完整路径 */
export function settingsExportFile(json: string) {
  return call<string>('settings_export', { json })
}

/** 读一个备份文件（拖进窗口时用），返回文件文本 */
export function settingsImportFile(path: string) {
  return call<string | null>('settings_import_file', { path })
}

/** 文件被拖进窗口。注意窗口要开 dragDropEnabled，Tauri 才会发这个事件 */
export async function onFileDrop(cb: (paths: string[]) => void): Promise<() => void> {
  if (!isTauri) return () => {}
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    return await getCurrentWindow().onDragDropEvent((e) => {
      if (e.payload.type === 'drop') cb(e.payload.paths)
    })
  } catch (err) {
    console.warn('[bridge] onFileDrop failed', err)
    return () => {}
  }
}

/* ------------------------------------------------------------- 关于页 */

export function appVersion() {
  return call<string>('app_version')
}

/** 交给系统默认浏览器打开（只放行 http(s)） */
export function openUrl(url: string) {
  return call('open_url', { url })
}

export interface UpdateInfo {
  current: string
  latest: string
  url: string
  notes: string
  published: string
  newer: boolean
}

/** 读 GitHub 最新 Release。失败时 command 会 reject，这里统一返回 null 由调用方提示 */
export async function checkUpdate(repo: string): Promise<{ info?: UpdateInfo; error?: string }> {
  if (!isTauri) return { error: '仅桌面版支持检查更新' }
  try {
    const mod = await import('@tauri-apps/api/core')
    const info = await mod.invoke<UpdateInfo>('check_update', { repo })
    return { info }
  } catch (err) {
    return { error: typeof err === 'string' ? err : '检查更新失败' }
  }
}

/* --------------------------------------------------------------- 窗口 */

export async function currentLabel(): Promise<string> {
  if (!isTauri) {
    return window.location.hash === '#settings' ? 'settings' : 'board'
  }
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    return getCurrentWindow().label
  } catch {
    return 'board'
  }
}

/**
 * 拖动窗口（词板 / 设置窗共用）。
 *
 * 刻意不用 tauri 的 startDragging：它底层是
 * `SendMessage(WM_NCLBUTTONDOWN, HTCAPTION)` 冒充"按住标题栏拖动"，
 * DWM 会画出一条标题栏帧并残留在屏幕上（点一下词板就冒出一条，
 * 要等下次 resize 才消失）。改成自己跟 pointermove 挪窗口，不碰非客户区。
 */
export async function startDragging(e?: PointerEvent): Promise<void> {
  if (!isTauri) return
  if (!e) return
  const target = e.currentTarget as HTMLElement | null
  try {
    target?.setPointerCapture(e.pointerId)
  } catch {
    /* 某些环境不支持指针捕获，窗口跟手会差一点，但不影响 */
  }
  const startX = e.screenX
  const startY = e.screenY
  let winX = 0
  let winY = 0
  const pos = await call<[number, number]>('window_logical_pos')
  if (!pos) return
  winX = pos[0]
  winY = pos[1]
  void call('set_dragging', { on: true })

  const onMove = (ev: PointerEvent) => {
    void call('move_window_logical', {
      x: winX + ev.screenX - startX,
      y: winY + ev.screenY - startY,
    })
  }
  const finish = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', finish)
    window.removeEventListener('pointercancel', finish)
    try {
      target?.releasePointerCapture(e.pointerId)
    } catch {
      /* ignore */
    }
    void call('set_dragging', { on: false })
  }
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', finish)
  window.addEventListener('pointercancel', finish)
}

export async function showSelf(): Promise<void> {
  if (!isTauri) return
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const w = getCurrentWindow()
    await w.show()
    await w.setFocus()
  } catch (err) {
    console.warn('[bridge] show failed', err)
  }
}

export async function hideSelf(): Promise<void> {
  if (!isTauri) return
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().hide()
  } catch {
    /* ignore */
  }
}

export async function minimizeSelf(): Promise<void> {
  if (!isTauri) return
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().minimize()
  } catch {
    /* ignore */
  }
}

/**
 * 前端日志：Rust 那边看不到界面层的失败（IPC、解析、DOM），
 * 统一走这个命令写进同一份内存日志。
 */
export async function logLine(level: 'INFO' | 'WARN' | 'ERROR', msg: string): Promise<void> {
  if (!isTauri) return
  try {
    const mod = await import('@tauri-apps/api/core')
    await mod.invoke('log_line', { level, msg })
  } catch {
    /* 记日志失败就沉默，绝不递归 */
  }
}

/** 导出内存日志到软件目录，返回写出的路径（失败返回空串） */
export async function exportLog(): Promise<string> {
  if (!isTauri) return ''
  try {
    const mod = await import('@tauri-apps/api/core')
    return (await mod.invoke<string>('export_log')) || ''
  } catch (err) {
    console.warn('[bridge] export_log failed', err)
    return ''
  }
}

/** 系统里所有能读到播放信息的播放器 */
export interface PlayerInfo {
  id: string
  name: string
  playing: boolean
}

export async function listPlayers(): Promise<PlayerInfo[]> {
  if (!isTauri) return []
  const res = await call<PlayerInfo[]>('list_players')
  return res ?? []
}

/** 指定歌词来源播放器；空串 = 自动 */
export async function setPlayerPref(id: string): Promise<void> {
  await call('set_player_pref', { id })
}

/** 锁定已选择的播放器：true = 它不在时也不退回自动 */
export async function setPlayerPin(pin: boolean): Promise<void> {
  await call('set_player_pin', { pin })
}

/** 无播放时关闭词板：true = 把词板窗口整个藏起来 */
export async function setBoardParked(parked: boolean): Promise<void> {
  await call('set_board_parked', { parked })
}

const POS_KEY = 'lyricsfloat.pos'
/** 改名前的键，读到就当新键用并迁过去 */
const POS_KEY_OLD = 'lyricboard.pos'

function readPos(): string | null {
  try {
    const raw = localStorage.getItem(POS_KEY)
    if (raw) return raw
    const old = localStorage.getItem(POS_KEY_OLD)
    if (old) {
      localStorage.setItem(POS_KEY, old)
      localStorage.removeItem(POS_KEY_OLD)
      return old
    }
  } catch {
    /* ignore */
  }
  return null
}

/** 记住词板位置：拖动结束后写盘，启动时还原 */
export async function savePosition(): Promise<void> {
  if (!isTauri) return
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const p = await getCurrentWindow().outerPosition()
    localStorage.setItem(POS_KEY, JSON.stringify({ x: p.x, y: p.y }))
  } catch {
    /* ignore */
  }
}

export async function restorePosition(): Promise<boolean> {
  if (!isTauri) return false
  const raw = readPos()
  if (!raw) return false
  try {
    const { x, y } = JSON.parse(raw) as { x: number; y: number }
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const { PhysicalPosition } = await import('@tauri-apps/api/dpi')
    await getCurrentWindow().setPosition(new PhysicalPosition(Math.round(x), Math.round(y)))
    return true
  } catch {
    return false
  }
}

/**
 * 把词板挪回主屏顶部居中 —— 拖出屏幕外之后唯一的找回入口。
 * 后端负责搬窗口并显形一下，这里把新位置写进记忆，免得下次启动又跳回原地。
 */
export async function resetBoardPosition(): Promise<boolean> {
  if (!isTauri) return false
  const pos = await call<[number, number] | null>('reset_board_position')
  if (!Array.isArray(pos) || pos.length !== 2) return false
  try {
    localStorage.setItem(POS_KEY, JSON.stringify({ x: pos[0], y: pos[1] }))
  } catch {
    /* ignore */
  }
  return true
}

export async function onMoved(cb: () => void): Promise<() => void> {
  if (!isTauri) return () => {}
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    return await getCurrentWindow().onMoved(cb)
  } catch {
    return () => {}
  }
}

/* --------------------------------------------------------------- 事件 */

export interface NativeHandlers {
  onReveal?: (on: boolean) => void
  onLocked?: (locked: boolean) => void
  onOpenSettings?: () => void
  onTrackChanged?: (p: unknown) => void
  onTick?: (p: unknown) => void
  onStopped?: () => void
}

export async function listenNative(h: NativeHandlers): Promise<() => void> {
  if (!isTauri) return () => {}
  try {
    const { listen } = await import('@tauri-apps/api/event')
    const offs = [
      await listen<boolean>('hover-changed', (e) => h.onReveal?.(e.payload)),
      await listen<boolean>('locked-changed', (e) => h.onLocked?.(e.payload)),
      await listen('open-settings', () => h.onOpenSettings?.()),
      await listen('track-changed', (e) => h.onTrackChanged?.(e.payload)),
      await listen('player-tick', (e) => h.onTick?.(e.payload)),
      await listen('player-stopped', () => h.onStopped?.()),
    ]
    return () => offs.forEach((fn) => fn())
  } catch (err) {
    console.warn('[bridge] listenNative failed', err)
    return () => {}
  }
}

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

export async function startDragging(): Promise<void> {
  if (!isTauri) return
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    await getCurrentWindow().startDragging()
  } catch (err) {
    console.warn('[bridge] startDragging failed', err)
  }
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

const POS_KEY = 'lyricboard.pos'

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
  const raw = localStorage.getItem(POS_KEY)
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

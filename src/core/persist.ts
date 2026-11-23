/**
 * 设置落盘。
 *
 * 结论先说：以前设置**只**在 WebView2 的 localStorage 里
 * （`%LOCALAPPDATA%\com.yuluo.lyricsfloat\EBWebView\Default\Local Storage\leveldb`）。
 * 那份数据确实跟着应用标识走，换 exe 不丢，但它没法备份、换机器就没了、
 * 清一次缓存全空。
 *
 * 现在改成：**应用数据目录的 `settings.json` 为准**，localStorage 只当缓存。
 * 绿色版把 exe + json 一起拷走，就是完整的配置。
 */

import {
  isTauri,
  settingsExportDialog,
  settingsImportFile,
  settingsLoadFile,
  settingsPath,
  settingsSaveFile,
} from './bridge'
import { adoptSettings, hasLocalSettings, saveSettings, setFileWriter, settingsSnapshot } from './store'

let ready = false
let path = ''
let pending: string | null = null
let timer: number | null = null

export function settingsFilePath(): string {
  return path
}

/** 导出的备份是 `{ schema, settings }` 包装过的，导入时两种形态都认 */
function unwrap(text: string): unknown | null {
  try {
    const raw = JSON.parse(text) as Record<string, unknown>
    if (raw && typeof raw === 'object' && raw.settings && typeof raw.settings === 'object') {
      return raw.settings
    }
    return raw
  } catch {
    return null
  }
}

function flush(): void {
  if (!pending) return
  const json = pending
  pending = null
  void settingsSaveFile(json)
}

/**
 * 启动时调用一次：把磁盘上的设置读进来，之后每次保存都同步写回磁盘。
 * 磁盘上没有文件（首次运行 / 从旧版本升级）就用当前内存里的值建一份，
 * 这样旧用户存量的 localStorage 设置不会丢。
 */
export async function initSettingsFile(): Promise<void> {
  if (ready) return
  ready = true

  if (!isTauri) {
    // 浏览器预览：没有文件可读，直接接上 localStorage 就行
    return
  }

  path = (await settingsPath()) || ''
  const text = await settingsLoadFile()

  // 本地缓存才是"当前这一份"：两个窗口共享同一份 localStorage，
  // 它是会话里最新的状态。只有本地缓存为空（清过缓存 / 换了机器 /
  // 拷了 exe + json 过来）时才用磁盘文件恢复。
  // 反过来让文件覆盖本地，会在刚改完设置、写盘还没落地时把改动吞掉。
  if (text && !hasLocalSettings()) {
    const raw = unwrap(text)
    if (raw) adoptSettings(raw)
  }

  // 注册写入钩子：高频改动（拖滑块）合并成 350ms 一次写盘
  setFileWriter((json) => {
    pending = json
    if (timer) clearTimeout(timer)
    timer = window.setTimeout(flush, 350)
  })

  // 没有文件（首次运行）或写入失败时，这里兜一次底
  if (!text) void settingsSaveFile(settingsSnapshot())
}

/** 导出设置：弹系统的「另存为」让用户选位置，返回写出的路径；取消返回空串 */
export async function exportSettingsFile(): Promise<string> {
  saveSettings()
  return (await settingsExportDialog(settingsSnapshot())) || ''
}

/** 从某个 json 文件导入设置 */
export async function importSettingsFile(file: string): Promise<boolean> {
  const text = await settingsImportFile(file)
  if (!text) return false
  const raw = unwrap(text)
  if (!raw) return false
  return adoptSettings(raw)
}

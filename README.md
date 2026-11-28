# LyricsFloat

> Windows 桌面歌词小工具。

透明、无边框、始终置顶。歌词跟着系统里正在播放的歌走。

<p align="center">
  <a href="https://github.com/YuuLuoLuo/LyricsFloat/releases/latest/download/LyricsFloat.exe">
    <img alt="下载最新版" src="https://img.shields.io/badge/%E4%B8%8B%E8%BD%BD%E6%9C%80%E6%96%B0%E7%89%88-Windows%20x64-2ea44f?style=for-the-badge&logo=windows&logoColor=white">
  </a>
</p>

<p align="center">
  <img alt="平台" src="https://img.shields.io/badge/platform-Windows%2010%2F11-0078D7">
  <img alt="许可" src="https://img.shields.io/badge/license-MIT-green">
</p>

---

## 特性

- **常驻桌面**：透明无边框，总在最前。锁定时鼠标点击全部穿透。
- **悬停显形**：鼠标靠近才出现，移开淡出。
- **歌词自动匹配**：读取系统播放信息，从多个曲库取词，结果存本地。
- **多种歌词样式**：旋转、翻译、逐字高亮、封面与曲名，四个开关任意组合。
- **媒体控件**：词板上直接上一曲、暂停、下一曲，另带进度条。
- **可调外观**：字体、行距、对齐、颜色、描边、阴影、动效、背景。
- **暂停自动隐藏**：暂停一段时间后收起，时间自己填。
- **无播放时关闭词板**：没在播或没歌词时，整个窗口收起。
- **多个播放器**：可以只认其中一个，并把它锁住。
- **托盘操作**：单击锁定 / 解锁，双击打开设置。

## 运行环境

| 项目 | 要求 |
|---|---|
| 操作系统 | Windows 10 1809（build 17763）及以上，推荐 Windows 11 |
| 架构 | x64 |
| 运行时 | WebView2 Runtime（Windows 11 自带） |
| 显示 | 支持 100%~200% 缩放 |

> 暂不支持 macOS / Linux。

## 下载与安装

点页面顶部的按钮即可下载最新版 `LyricsFloat.exe`。
想先看改了什么，打开 [Releases](https://github.com/YuuLuoLuo/LyricsFloat/releases/latest)。

1. 放到任意目录双击运行，绿色免安装。
2. 首次运行会在 `%APPDATA%\com.yuluo.lyricsfloat\` 生成配置、歌词缓存和日志。不用了删掉这个目录即可。

版本号在文件属性里：右键 → 属性 → 详细信息。只有开启「开机自动启动」时才写一条注册表自启项。

提示缺少 WebView2 时，装一下 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。

## 使用说明

- **播放**：用任意播放器放歌，词板自动认出来。没有播放时显示「等待播放器」。
- **显形**：鼠标移到词板附近，歌词和背景出现；移开淡出。锁定后不响应。
- **锁定 / 设置**：托盘单击锁定或解锁，双击打开设置，右键菜单里也能操作。
- **拖动**：解锁后按住词板拖动，位置会记住。
- **调尺寸**：拖宽高滑块时词板临时显形，方便对齐。
- **找不回词板**：设置页「常规 → 词板位置 → 移回屏幕中央」。
- **时间对不上**：设置页「歌词 → 同步 → 时间偏移」，范围 ±3000 ms。
- **本地歌词**：把 `.lrc` 文件拖进设置窗即可使用。

## 支持的播放器

LyricsFloat 读的是 Windows 系统媒体控件（SMTC），也就是按媒体键时任务栏弹出的「正在播放」。

- **直接可用**：网易云音乐、QQ 音乐、酷狗音乐、Spotify、Apple Music（Windows 版）等。
  网易云音乐只有高版本原生支持 SMTC，低版本需要第三方插件才能上报。
- **需要打开媒体键 / 系统集成**：PotPlayer、AIMP、JRiver。
- **网页播放**：Chrome 或 Edge 里播 YouTube、Bilibili、网易云网页版也会上报。
- **读不到**：播放器没有实现 SMTC，或被系统关掉了媒体键支持。
- **多个播放器同时在跑**：默认跟随正在播放的那个；可在「歌词 → 播放来源」里指定，还能锁定。

## 可设置项

设置窗口分四页：**常规 / 歌词 / 外观 / 关于**。样式预览只在外观页。

**常规**：开机自动启动、总在最前、锁定词板、无播放时关闭词板、暂停后自动隐藏（延迟、是否连信息与控件一起隐）、
词板位置、数据文件夹、设置文件导入导出、歌词缓存、日志、退出。

**歌词**：当前播放器、播放来源（自动 / 指定 / 锁定）、当前曲目与进度、歌词源与搜索结果、时间偏移。

**外观**：歌词样式、歌词信息布局（位置、封面、文本、颜色、无歌词时的位置）、媒体控件（部件、进度条位置、
显示时机、锁定时可点击、取色）、词板尺寸、字体、对齐、颜色、描边与阴影、动效、背景样式（不透明度、
背景颜色、背景常显、从封面取色）。

**关于**：版本与检查更新（含更新日志）、仓库地址、作者。

## 从源码构建

需要 **Node.js 18+**、**Rust 1.77+**、**Visual Studio 2022 生成工具（含 C++ 与 Windows SDK）**。

```bash
npm install                 # 装前端依赖

# 开发调试（两个终端）
npm run dev                 # Vite 开发服务器 127.0.0.1:1420
cargo run --manifest-path src-tauri/Cargo.toml

# 发布构建
bash scripts/pack.sh        # 产物：release/LyricsFloat.exe
```

Windows 下 cargo 找不到 MSVC 环境时，先执行 `source scripts/msvc-env.sh`。

## 常见问题

**词板不见了？**
可能锁定了，或者被拖到了屏幕外面。托盘单击解锁，或按上面的方法移回屏幕中央。

**歌词慢半句？**
先用「时间偏移」微调。很多歌都偏的话，多半是播放器上报的进度有延迟。

**封面显示不出来？**
封面来自播放器上报的缩略图。部分播放器不提供，此时显示一格占位。

**杀毒软件报毒？**
未签名的自编译 exe 常见误报。可以自己构建，或加入白名单。

## 许可协议

[MIT](LICENSE)

## 关于

- 作者：**羽洛洛 With DeepSeek**
- 歌词来自网易云音乐、QQ 音乐、[lrclib](https://lrclib.net) 等平台的公开接口，版权归各平台与版权方所有。本工具只在本地展示和缓存。
- 基于 [Tauri](https://tauri.app)、[Vue](https://vuejs.org)、[Vite](https://vitejs.dev) 构建。

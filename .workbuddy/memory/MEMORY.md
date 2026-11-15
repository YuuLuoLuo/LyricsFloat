# 歌词板项目长期备忘

## 打包（用户说「打包」时执行）
1. `npx vite build`（先更新 dist/，嵌入内容是编译期拷的）
2. `source scripts/msvc-env.sh && cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml`
3. `cp src-tauri/target/release/lyric-board.exe release/lyric-board.exe`

**坑**：裸 `cargo build --release` 不带 `--features custom-protocol` 时 tauri 编译成 dev 模式，
窗口去连 devUrl(127.0.0.1:1420)，开发服务器一停就白屏 ERR_CONNECTION_REFUSED。
`tauri build` CLI 会自动开该特性；Cargo.toml 有 `[features] custom-protocol` 转发。

## 验证手段
- 调试端口：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动，
  `scripts/cdp.py` 读真实 DOM（第 3 参数按标题精确筛窗口：'歌词板' / '歌词板设置'）。
- release 验证：同法起 9334，`cdp.PORT = 9334`；URL 应为 `http://tauri.localhost/`。
- 窗口截图用 PrintWindow（scripts/shot.py / print-window.py），桌面截图 API 被安全策略拦。
- 验证完必须 `quit_app` 优雅退出，`taskkill` 会把未落盘设置打回。

## 用户偏好
- 每个功能一次 git 提交。
- 设置页文案只留必要提示，不要写修改说明。
- 用户玩游戏时禁止弹窗/抢焦点测试。
- 设置落盘在 `%APPDATA%\com.yuluo.lyricboard\settings.json`，WebView2 localStorage 旧快照
  启动时会覆盖文件，测试改过的设置要还原：540×260、翻译开、右对齐、锁定开。

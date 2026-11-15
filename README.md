# 歌词板（lyric-board）— Agent 工作手册

> 本文档面向 AI agent / 开发者，记录架构、机制、坑与工作约定。
> 不是 GitHub 面向用户的 README，可以直接写实现内幕。

---

## 1. 项目是什么

Windows 桌面**悬浮歌词板**：常驻桌面、全透明、鼠标穿透，鼠标悬停时磨砂玻璃显形并显示当前播放歌词。从系统媒体传输控件（SMTC，即任务栏/媒体键那一层）读取任意播放器的播放状态，歌词自动从网易云/QQ 音乐/lrclib 匹配拉取，也支持拖入 LRC 文件。

- **技术栈**：Tauri 2 + Vue 3 + Vite + TypeScript（前端），Rust（系统层）
- **进程模型**：单进程双窗口 —— `board`（词板，无边框置顶小窗）与 `settings`（设置窗），各自独立 WebView2 / JS 上下文，通过 localStorage 事件 + 轮询同步设置
- **目标平台**：Windows（大量 Win32 API：DWM、SMTC、DirectWrite、SetWindowRgn）；非 Windows 路径编译通过但功能为 no-op

## 2. 目录结构

```
歌词板/
├── src/                        # 前端
│   ├── App.vue                 # board 窗口入口：设置装载、事件订阅、resize 调度
│   ├── views/SettingsApp.vue   # 设置窗口全部 UI（常规/外观/关于 三标签）
│   ├── components/
│   │   ├── Board.vue           # 词板外壳：玻璃底、卡片信息头、锁定钮、右键菜单、拖动
│   │   ├── LyricStage.vue      # 歌词舞台：行渲染、排布/旋转/逐字高亮、行数自适应
│   │   └── FontSelect.vue      # 字体下拉（Teleport 浮层、搜索、分组）
│   └── core/
│       ├── store.ts            # settings（持久化）+ runtime（运行态）；prune/tame 迁移
│       ├── player.ts           # SMTC tick 消费、歌词加载调度、calibrate
│       ├── clock.ts            # PlayClock：锚点外推 + drift 收敛 + syncPaused
│       ├── lyrics.ts           # 时间轴构建、offset、行/字词数据
│       ├── bridge.ts           # Tauri 命令/事件桥（浏览器环境全部 no-op）
│       ├── persist.ts          # 设置落盘钩子（350ms 防抖）+ 导入导出
│       ├── about.ts            # repo 常量（发布后填）、版本/更新检查前端
│       └── types.ts            # LyricLine/LyricDoc/字词类型
├── src-tauri/src/
│   ├── lib.rs                  # 命令注册、setup、托盘/窗口装配
│   ├── player.rs               # SMTC 轮询线程（250ms）→ emit player-tick / track-changed
│   ├── lyrics.rs               # 打分匹配 + sanitize + 网易云/QQ/lrclib 兜底 + 缓存
│   ├── fonts.rs                # DirectWrite 枚举系统字体家族（约 200+）
│   ├── settings_io.rs          # settings.json 原子读写、导入导出、旧版迁移
│   ├── about.rs                # app_version / open_url / check_update（GitHub Releases）
│   ├── autostart.rs            # 注册表自启
│   ├── tray.rs                 # 托盘：单击切换锁定（220ms 去抖）、双击开设置、右键菜单
│   └── window/
│       ├── mod.rs              # 窗口生命周期、reveal、resize_exact、拖动命令
│       ├── hover.rs            # 16ms 悬停轮询：显隐、穿透位对齐、strip_caption
│       └── effects.rs          # 亚克力、圆角区域、去边框、strip_caption、set_click_through
├── scripts/
│   ├── msvc-env.sh             # 注入 MSVC 构建环境（source 后 cargo 可用）
│   ├── cdp.py                  # WebView2 远程调试：eval/eval-all/pages（端口 9333）
│   ├── print-window.py         # PrintWindow 抓窗口自身渲染
│   └── make-icon.mjs           # 生成图标
├── release/lyric-board.exe     # 发布产物（gitignore）
├── backups/                    # 手动版本备份点（gitignore）
└── .workbuddy/memory/          # 工作日志（YYYY-MM-DD.md）与长期备忘（MEMORY.md）
```

## 3. 核心机制（改代码前必读）

### 3.1 播放状态（src-tauri/src/player.rs）

- 250ms 轮询 `GlobalSystemMediaTransportControlsSessionManager`，挑**正在播放**的会话。
- **坑：SMTC 的 `Position` 单位是秒**（100ns ticks / 1e7），前端时间轴一律毫秒 —— `posMs = position * 1000`。曾经漏乘导致歌词恒差几百毫秒。
- `Position` 是「上次 timeline 更新那一刻」的位置，不是现在。播放器普遍 1~5 秒才刷新一次，直接用会整体滞后。按 `LastUpdatedTime` 外推到现在，外推上限 15s（`MAX_AGE`），2.5s 内算 fresh。
- **暂停冻结**：暂停后播放器不刷新 timeline，Position 会退回旧值。Rust 侧有 `PauseState`：暂停后按外推值冻结位置，只有 timeline 真变了（暂停中拖进度条）才跟进。
- 会话选择：优先 `PlaybackStatus == Playing` 的会话；遍历所有会话兜底。
- 输出事件：`player-tick`（NowPlaying：title/artist/album/duration/position/playing/fresh）、`track-changed`（换歌）。

### 3.2 时钟（src/core/clock.ts + player.ts）

歌词位置 = PlayClock 读数，不是直接用 SMTC 采样：

- 锚点外推：记住 (采样时刻, 采样位置)，读数 = 锚点 + 经过时间，按 1× 前进。
- `sync(pos)`：正向偏差 >400ms 直接 seek，120~400ms 走 drift 平滑（上限 400ms），<120ms 当噪声忽略；反向（我们超前）默认忽略（采样滞后是常态）。
- 反向大修正只在连续 3 次 >500ms 超前或单次 >8s（拖进度条）才回锚 —— 防「启动锚在旧位置后永远对不回来」。
- `syncPaused(pos, honorBackward)`：暂停时只认明显拖动。**重复采样不回拽**：`pausedBase` 记住暂停期间上次报的值，±50ms 内视为同一份采样，绝不往回 seek —— 这是「暂停后歌词滑到下一句再回弹」的最终修复。
- 采样补足：播放器没刷新 timeline 时 position 停住，按「该数值首次出现时刻」补足流逝时间，防「偶发不同步且不自愈」。

### 3.3 悬停显隐与穿透（window/hover.rs + effects.rs）——最容易踩坑的区域

**静止态「隐形 + 穿透」的实现**：`WS_EX_TRANSPARENT | WS_EX_LAYERED` 两个样式位。
layered 且不调 `SetLayeredWindowAttributes` ⇒ 整窗不参与合成 ⇒ 隐形；`WS_EX_TRANSPARENT` ⇒ 点击穿到下层。显形 = 摘掉这两个位 + 开亚克力。

**tao 的三个坑（全部实测踩过）**：

1. **tao 整份重写样式会抹掉我们写的位**。`show` / `set_size` / `set_always_on_top` 等都会 `SetWindowLongW(GWL_STYLE/GWL_EXSTYLE, tao缓存里的值)` + `SWP_FRAMECHANGED`，我们塞进去的 WS_EX_TRANSPARENT|WS_EX_LAYERED 和摘掉的 WS_CAPTION 全部会被写回/抹掉。
   → 对策：悬停轮询每 16ms 对齐一次（`strip_caption` + `set_click_through(!REVEALED)`），与目标一致时只读不写，零开销。
2. **WS_CAPTION 偶现标题栏**。tao 建无边框窗口时 GWL_STYLE 里始终留着 `WS_CAPTION|WS_SYSMENU`，一旦被 SWP_FRAMECHANGED 重算，DWM 就画出标题栏（甚至带最小化/关闭按钮，因为 SYSMENU/MIN/MAX 位也在）。
   → 对策：`effects::strip_caption` 摘 CAPTION/SYSMENU/THICKFRAME/MIN·MAXBOX/WINDOWEDGE 等一族，变了才写 + FRAMECHANGED；每拍轮询兜底。
3. **`drag_window` / `start_dragging` 会画出标题栏帧**。它靠 `SendMessage(WM_NCLBUTTONDOWN, HTCAPTION)` 冒充「按住标题栏拖动」，DWM 收到非客户区交互后画一条标题栏帧，**且样式位是干净的，strip_caption 无变化可清**，帧会一直留到下次 resize（用户原话「点击出现、调整设置消失」）。
   → 对策：已废弃该路径，改为手动拖动（`window_logical_pos` + `move_window_logical`，前端 setPointerCapture + pointermove 跟手挪窗口），源头不碰非客户区。

其他要点：

- `set_click_through` 只改扩展样式位（不碰 GWL_STYLE），不会触发框架重算 —— 悬停进出是最频繁的操作，必须廉价。
- 亚克力用 `SetWindowCompositionAttribute(ACCENT_ENABLE_ACRYLICBLURBEHIND)` 自己调（不用 window-vibrancy，Win11 22H2 后 DWM 接管会取系统主题色，浅色主题下变白底）；GradientColor 要 ABGR 序（`0xAABBGGRR`）。
- 圆角用 **SetWindowRgn**（`CreateRoundRectRgn`，8 逻辑 px，按 `GetDpiForWindow` 换算）裁，不用 `DWMWA_WINDOW_CORNER_PREFERENCE` —— 后者附带投影，静止态全透明时阴影浮在桌面。窗口有 Rgn 后 DWM 不再投影/画框架。
- DWM 1px 边框用 `DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE` 去掉。
- tao 会给窗口留 `WS_CAPTION` ⇒ DWM 按标题画字 ⇒ 建窗时 `set_title("")`。
- 锁定（LOCKED）：暂停一切悬停响应，词板彻底消失；peek（调宽高时临时显形）在锁定态不生效。
- 悬停判定：进入 PAD_IN=4px 外扩，离开 PAD_OUT=36px 内收 + 320ms 延迟，防边缘抖动；光标在设置窗上时不算悬停词板。

### 3.4 歌词（src-tauri/src/lyrics.rs + src/core/lyrics.ts）

- 匹配打分：标题/歌手清洗（去括号注释、feat、大小写）后模糊打分，低分拒绝。
- 兜底顺序：本地缓存 → 网易云 → QQ 音乐 → lrclib。缓存在 `%LOCALAPPDATA%\com.yuluo.lyricboard\lyrics\`，临时文件 + rename 原子写。
- LRC 解析含毫秒时间轴、翻译行（`[xx:xx.xx]` 双语同刻）、逐字（yrc/karaoke）解析为 `words[]`（start/end/text）。
- 前端 `offsetMs` 可整体平移时间轴（改了要 `reapplyOffset` 重建）。
- **前奏/尾奏让空**：`cur` 三态（-1 前奏 / 0..n-1 / n 尾奏）带 150ms 迟滞，当前句不落在首句/尾句上。

### 3.5 歌词排布（src/components/LyricStage.vue）

四个独立开关可任意组合（`settings.view`：`rotate` / `translation` / `karaoke`，`settings.info.show`）：

- 基础排布 = 多行滚动：以当前句为锚，行距/缩放/模糊/透明度按距离衰减；`vAlign` 控制当前句贴顶或居中；行数由窗口高度决定（能塞几行显几行）。
- `rotate`（歌词旋转，原指针排布）：以对齐边（左/右）为圆心，两侧句 `rotate = d*3°*方向`、模糊加深、层叠在当前句下；右锚点旋转方向取负（镜像）。开启时行距 ×0.78；**中间当前句（原文+译文整体）不旋转，两侧才转**。
- `translation`：当前句下方挂译文行；开启时行高 ×1.5 让出译文位。
- `karaoke`（逐字高亮）：当前句按 `words[]` 切 span，past/active/todo 三态着色（active 用 `colorHighlight`）。
- `info.show`（显示歌词信息，原卡片模式）：封面位置（左/右/不显示）× 标题位置（顶/底/不显示），两个都不显示时退化纯歌词。
- `autoFit`：某句超宽时用 canvas measureText 把那一行单独缩字号。
- 描边用**八向 text-shadow**画（`-webkit-text-stroke` 被字形吃掉，白字描边实测无效）；阴影颜色可配。

### 3.6 设置持久化（store.ts + persist.ts + settings_io.rs）

- 位置：`%APPDATA%\com.yuluo.lyricboard\settings.json`（临时文件 + rename 原子写）。
- **localStorage 优先、文件兜底**（两窗口靠 storage 事件 + 900ms 轮询同步）。启动读取后写回两处。⚠️ WebView2 的 localStorage 有时会带着测试期旧快照覆盖文件 —— 测试改过的设置要手动还原。
- `prune()` 按 DEFAULTS 形状重建（补新字段、丢弃废弃字段）；`tame()` 收紧存量超界值。
- 导出到桌面 / 从文件导入（拖入设置窗也行）；旧版 exe 旁的设置文件自动 `migrate_legacy`。
- 版本键 `lyricboard.settings.v3`。

### 3.7 关于页与更新检查（about.rs + core/about.ts）

- `check_update(repo)` 读 `api.github.com/repos/{owner}/{repo}/releases/latest`，比 tag 与 `CARGO_PKG_VERSION`；404 / 网络错误 / 未配置分开提示。
- Release 说明经 `tidy_notes` 清洗（剥 HTML 标签、展开 Markdown 链接、压空行、截 900 字）。
- `open_url` 只放行 http(s)（ShellExecuteW）。
- **仓库地址在 `src/core/about.ts` 的 `repo` 常量，当前留空**，发布后填 `owner/repo`；留空时按钮禁用。

### 3.8 托盘（tray.rs）

- 单击 = 切换锁定（**220ms 去抖**：等一小段确认不是双击，世代号作废机制）。
- 双击 = 打开设置（SUPPRESS_UNTIL 拦掉双击里的第二次 Up）。
- 右键 = 菜单（锁定/解锁、打开设置、退出）；菜单文案跟随状态（`sync_label`）。

## 4. 构建与打包

```bash
# 环境（每次新 shell 都要）
source scripts/msvc-env.sh

# 开发（两终端）
npm run dev                                                    # vite @127.0.0.1:1420
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333 \
  ./src-tauri/target/debug/lyric-board.exe                     # debug 走 devUrl

# 发布打包（完整三步，缺一不可）
npx vite build
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml
cp src-tauri/target/release/lyric-board.exe release/
```

**⚠️ 最大的打包坑**：裸 `cargo build --release` 不带 `--features custom-protocol` 时，tauri crate 编译成 dev 模式，窗口去连 `devUrl`（127.0.0.1:1420），开发服务器一停就整窗白屏 `ERR_CONNECTION_REFUSED`。`tauri build` CLI 会自动开这个特性；我们手动 cargo 构建必须显式带。Cargo.toml 有 `[features] custom-protocol = ["tauri/custom-protocol"]` 转发。

MSVC 环境脚本硬编码 VS Build Tools 14.44.35207 / SDK 10.0.26100.0（升级时改 `scripts/msvc-env.sh`）。

## 5. 调试与验证工具

- **CDP**：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动后用 `scripts/cdp.py` 读真实 DOM / 直接改 store 验证：
  - `python scripts/cdp.py eval "<js>" "歌词板设置"` —— 第 3 参数按**标题精确**筛窗口（`歌词板` / `歌词板设置`；两窗口 title 都可能被 set_title("") 影响，用页面 URL/内容区分兜底）。
  - 也能 `import('/src/core/store.ts')` 拿到真实模块实例，改设置、塞假歌词、量 DOM。
- **窗口截图**：桌面截图 API 被安全策略拦，用 `scripts/print-window.py`（PrintWindow PW_RENDERFULLCONTENT）。注意 BMP 高度为负是 top-down；纯 Python 转 PNG 的小脚本见工作日志。PrintWindow **不包含 DWM 画的非客户区**（标题栏帧在 PrintWindow 里看不到，要用屏幕截图判断）。
- **窗口样式探针**：枚举进程窗口读 `GetWindowLongW(GWL_STYLE/GWL_EXSTYLE)`，验证 caption/layered/transparent 位；`WindowFromPoint` 验证穿透（返回的应是下层窗口）。
- **假 tick**：`plugin:event|emit` 发 `player-tick` 可注入假播放状态（注意会被真实 SMTC 轮询覆盖，只适合短窗口测试）。
- **真机播放/暂停**：`keybd_event(VK_MEDIA_PLAY_PAUSE)` 发媒体键。
- 验证完必须 `quit_app` 优雅退出 —— `taskkill` 会把未落盘设置打回旧值。
- 退出前记得停掉 vite 后台任务；用户玩游戏时**禁止弹窗/抢焦点测试**。

## 6. 已知坑清单（历史学费）

| 坑 | 根因 | 解法 |
|---|---|---|
| 歌词恒慢半句/首句闪烁 | SMTC Position 是秒，前端当毫秒 | `posMs = position*1000` |
| 暂停后滑到下一句再回弹 | 暂停时 Position 退回旧值 + 前端继续外推/回拽 | Rust 冻结位置 + syncPaused 重复采样不回拽 |
| 两句抽搐 | clock 把负差当误差吸收，每 250ms 往回拽 | 反向需连续 3 次才回锚 |
| 偶现标题栏 | tao 留 WS_CAPTION，SWP_FRAMECHANGED 时 DWM 画标题 | strip_caption + 每拍轮询 |
| 点击词板残留标题栏帧 | drag_window 发 WM_NCLBUTTONDOWN(HTCAPTION) | 手动拖动 |
| 启动锁定不穿透/显形 | tao 重写样式抹掉 WS_EX_TRANSPARENT\|LAYERED | 每拍对齐穿透位 |
| 静止态一圈暗影 | DWM 窗口级圆角附带投影 | SetWindowRgn 裁圆角 |
| 磨砂变白底 | window-vibrancy 由 DWM 接管跟系统主题取色 | 自调 SetWindowCompositionAttribute |
| 白字描边不生效 | -webkit-text-stroke 被字形吃掉 | 八向 text-shadow |
| 左右对齐文本溢出 | transform-origin 恒为 center，放大推出锚点侧 | origin 跟随对齐（26px/center） |
| 字体下拉白底白字 | 原生 select 缺 color-scheme | `color-scheme: dark` + option 显式色 |
| 调色板打开致设置页错位 | 隐藏 input[type=color] 聚焦滚动 | 固定 1px + 按钮 click() |
| release 白屏 ERR_CONNECTION_REFUSED | 缺 --features custom-protocol | 见 §4 |
| 后开词板偶发不同步不自愈 | 启动锚在未外推旧值，两边同速差恒定 | 采样补足 + 反向 3 次回锚 |
| 设置被打回旧值 | taskkill 不走 saveSettings | quit_app 优雅退出 |

## 7. 用户约定（必须遵守）

- **一个功能一次 git 提交**，提交信息写清根因与改法（中文）。
- 设置页文案只留必要提示（如「配置信息跟随软件本体」），**不要写修改说明/更新日志**。
- 用户玩游戏时禁止以弹窗/抢焦点方式测试。
- 视觉默认值：涨红跌绿（金融场景）；本项目颜色模板 light/dark 一键切换。
- 测试改过的设置必须还原（用户原值：540×260、翻译开、右对齐、锁定开、fontSize 23 —— 以 settings.json 当前值为准）。
- `.workbuddy/` 目录不可删；工作日志 append-only。

## 8. 当前状态与待办

- 版本 0.2.0；`git log` 为主线（每功能一提交）。
- `src/core/about.ts` 的 `repo` 留空，等 GitHub 仓库建好填入并把英文名（首选 **Lyriva**，备选 Lyricue / Lyrix）写进软件。
- 设置窗三标签：常规（含当前播放信息）/ 外观 / 关于。
- 暂停/标题栏/穿透修复待用户真机长测反馈。

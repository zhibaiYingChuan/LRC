# Changelog

所有重要变更记录。遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

---

## [0.6.0] - 2026-07-26

### v3.0 全局动态审计与真实测试（2026-07-28）

- **审计背景**：用户指出 v2.0 测试存在虚假性（lrcmcp 服务未真正打开），需重新审计并编译桌面端进行真实本地测试
- **v3.0 审计结果**：v2.0 修复全部验证生效，代码层面无新增问题（0严重/0中等/2低等）
- **P0 关键修复：桌面端"服务已停止"根因修复**：
  - **根因一**：[static/app.js](file:///g:/code-memory/static/app.js) 中 `API_BASE` 使用 `window.location.origin`，在 Tauri WebView 中为 `https://tauri.localhost`，导致所有 API 请求失败
  - **修复一**：检测 Tauri 环境（`window.__TAURI__` 或 `tauri.localhost`），使用 `http://127.0.0.1:3099` 直连 sidecar
  - **根因二**：[src/server.rs](file:///g:/code-memory/src/server.rs) CORS 白名单缺少 `https://tauri.localhost`（Tauri 2.x Windows WebView 的源）
  - **修复二**：CORS 白名单添加 `https://tauri.localhost`
- **编译验证**：
  - 主项目编译成功（cargo build --release --features server，1m04s）
  - 桌面端编译成功（npm run build，2m23s）
  - 生成 MSI 安装包（5.43 MB）+ NSIS 安装包（3.73 MB）
- **真实模拟用户测试**（10/10 + 5/5 全部通过）：
  - sidecar 服务真实运行（PID 17376，14.14 MB，端口 3099 监听）
  - 桌面端 WebView2 到 sidecar 的 3 个 TCP 连接已建立（msedgewebview2 PID 16500 → 127.0.0.1:3099）
  - CORS 验证：`https://tauri.localhost` 被允许，`https://evil.com` 被拒绝
  - API 验证（模拟 Tauri Origin）：health/system/dao_metrics/memories_list/memories_recent 全部 200
- **安全加固验证**：CORS 白名单、路径遍历防护、CSP 配置、TOML 注入防护全部通过
- **相关文档**：审计报告、修复计划、测试报告为内部开发文档，仅本地保留，不入库

### 新增

- **v0.6.0 通用语义引擎**——将默认嵌入模型从 CodeBERT 切换为通用文本嵌入模型，提升非编程场景语义搜索能力。
  - 中文环境默认 `BAAI/bge-small-zh`（512 维），英文环境默认 `sentence-transformers/all-MiniLM-L6-v2`（384 维），基于系统语言自动检测。
  - 新增 `src/engine/embedder.rs`：统一 `Embedder` trait 抽象层，实现 `LocalBertEmbedder` 与 `LlmApiEmbedder`，支持代码搜索与结晶路径共享嵌入器。
  - 新增 `src/engine/model_resolver.rs`：统一模型文件就绪检测接口 `check_model_ready()`。
  - 新增 `src/engine/luoshu_encoder_ml.rs` 中 `detect_default_model()`：基于系统语言的默认模型检测；动态投影矩阵适配 512/384 维输入。

- **模型下载器**（[src/engine/model_downloader.rs](file:///g:/code-memory/src/engine/model_downloader.rs)）：
  - `DownloadProgress` trait：进度回调接口（`on_progress`/`on_complete`/`on_error`）。
  - `ConsoleProgress`：控制台进度条实现，支持已知/未知总大小的下载。
  - `MirrorSource` 枚举：镜像源选择（HfMirror/ModelScope/Auto）。
  - `DownloadConfig`：下载配置（超时、重试次数、退避策略）。
  - `ModelDownloader::download_with_retry()`：带指数退避的重试下载（initial=2s/max=8s/retries=3）。
  - `build_download_url()`：根据镜像源构建下载 URL。
  - `manual_download_guide()`：3 次重试失败后输出手动下载指引。
  - 18 个单元测试覆盖 URL 构建、退避计算、进度回调、错误处理等场景。

- **模型管理 CLI 命令**（[src/bin/server.rs](file:///g:/code-memory/src/bin/server.rs)）：
  - `code-memory-server model list` — 列出本地已下载模型（model_id / 路径 / 大小 / 当前默认标记）。
  - `code-memory-server model download <model_id>` — 触发下载（带进度条 + 重试）。
  - `code-memory-server model use <model_id>` — 设置默认模型。
  - `code-memory-server model remove <model_id>` — 删除模型文件。
  - 辅助函数：`get_models_dir()`、`calculate_dir_size()`、`format_size()`。

### 变更

- **结晶路径支持本地嵌入**：[src/consolidation.rs](file:///g:/code-memory/src/consolidation.rs) 的 `embedding_synthesize_cycle()` 接受 `&dyn Embedder` 参数，支持本地嵌入与 LLM API 嵌入统一调用；本地嵌入失败时降级到洛书统计合成。
- **国内镜像默认启用**：`src/bin/server.rs` 启动时自动设置 `HF_ENDPOINT=https://hf-mirror.com`（如未显式配置）。
- **`src/engine/mod.rs`**：注册并导出 `model_downloader` 模块。
- `Cargo.toml` 版本号 0.5.18 → 0.6.0。

### 测试

- `cargo check --features server,ml` 编译通过。
- `cargo test --features server,ml` 全部通过：单元测试 456 passed，benchmark 11 passed，doc-tests 8 ignored。
- 新增 model_downloader 模块 18 个单元测试（覆盖 URL 构建、退避计算、进度回调、错误处理等）。

### UI 重构（v0.6.0 龙忆设计系统 v1.0）

- **全面应用龙忆设计系统 v1.0**——基于《LRC 全案界面重构设计文档》完成样式重构，实现"形现代，意古风"设计理念。
  - 引入 `static/colors_and_type.css`：6 组色阶（墨韵/宣纸/金色/玉色/朱砂/水蓝，每色 10 级）、语义别名、便携别名、排版、间距、圆角、阴影、动效等完整设计 Token。
  - 引入 `static/components.css`：按钮（5 种变体 + 3 种尺寸 + 洛书加载动画）、卡片（含记忆类型色条）、输入框、模态框、侧边栏、标签栏等全局组件库。
  - 迁移 15 个 SVG 图标（icon-dashboard/memory/trust/crystallization/luoshu/audit/bagua/decay/search-lrc/captain-log/benchmark/health/privacy/network/integrity）到 `static/assets/icons/`。
  - 迁移 4 个 SVG Logo（logo-primary/horizontal/vertical/text-only）到 `static/assets/logo/`。
- **[static/index.html](file:///g:/code-memory/static/index.html) 重构**：
  - 顶部导航栏：使用新 Logo + SVG 图标替换 emoji，应用墨韵-宣纸配色。
  - 统计卡片：4 张卡片使用 4 种色阶（墨韵/金色/玉色/朱砂）+ 对应 SVG 图标。
  - 信任中心：6 张卡片按记忆类型添加色条（fact→玉色/preference→金色/decision→朱砂/code_context→水蓝）。
  - 5 分钟向导、船长日志、API 文档、设置页面：emoji 全部替换为 SVG 图标。
- **[static/app.css](file:///g:/code-memory/static/app.css) 重构**：
  - `:root` 别名映射：将旧变量（`--ink`/`--gold`/`--jade` 等）映射到新设计系统变量（`--lrc-墨韵-500`/`--lrc-金色-500`/`--lrc-玉色-500` 等），保持向后兼容。
  - 新增 v0.6.0 增强样式：记忆色条、洛书九宫格加载动画、诗意空状态、暗色模式（`prefers-color-scheme: dark`）、预设场景模板选择器、结晶历史时间线、一键隐私检查按钮。
- **[static/app.js](file:///g:/code-memory/static/app.js) 新增功能**：
  - `selectPresetScenario()`：4 套预设场景模板选择（v0.7.0 预览）。
  - `loadCrystallizationHistory()`：从审计日志加载结晶事件并渲染时间线（v0.8.0 预览）。
  - `runPrivacyCheck()`：并行调用三个信任接口，100ms 内返回三色信任指示器报告（v0.9.0 预览）。
- **复杂场景测试**：3 个场景全部通过（Playwright 自动化验证）——
  - 场景一（仪表盘首屏）：欢迎区显示"早上好，欢迎回来"+诗意短句；道同构度仪表盘评分 85 画布渲染；侧边栏折叠/展开 240px↔60px；系统状态浮窗"统计模式"；版本号 v0.6.0；控制台 0 错误 0 警告。
  - 场景二（记忆搜索页面）：搜索栏输入"LRC"返回 6 条记忆卡片；筛选面板正常；点击卡片打开详情面板（memory-detail-panel open）显示记忆内容与元数据。
  - 场景三（信任中心 + 系统状态浮窗）：6 张信任卡片显示；一键隐私检查按钮点击后显示 4 个验证结果面板；系统状态浮窗展开/折叠正常（165px ↔ collapsed）。

### UI 重构补丁（v0.6.0 严格遵循设计文档修复）

- **三层基准测试切换标签**（设计文档 5.6）：在基准报告页面添加"通用检索/独有能力/隐私信任"三层胶囊样式切换标签，使用金色 500 选中项 + 暗色模式适配。
- **静态资源嵌入 sidecar**：将 `colors_and_type.css`、`components.css`、2 个 Logo SVG、15 个图标 SVG 通过 `include_str!` 嵌入 sidecar 二进制，添加 `/colors_and_type.css`、`/components.css`、`/assets/logo/:filename`、`/assets/icons/:filename` 路由，解决 404 错误。
- **safeJson 作用域修复**：将 `safeJson` 函数暴露到 `window` 对象，解决 IIFE 外部新增函数（道同构度、演化时间线、结晶历史加载）无法访问的问题。
- **搜索 API 端点修复**：将记忆搜索端点从不存在的 `POST /recall` 改为 `POST /v1/memories/enrich`，适配 `EnrichResponse` 响应格式（`data.memories` 数组）。
- **版本号硬编码修复**：将 `app.js` 中 `v0.5.4` 和 `index.html` 中 `v0.2.0` 统一为 `v0.6.0`。

### UI 样式优化补丁（v0.6.0 前端页面样式问题修复）

- **Logo 升级为 PNG 图片**：根据设计文档品牌与 Logo 设计规范，生成符合要求的 Logo 图片（主标识、横式组合、竖式组合），替换原简单 SVG 图标。
- **侧边栏样式修复**：
  - 修复 Logo 尺寸问题，明确设置 32px × 32px，添加 `object-fit: contain` 确保正确缩放。
  - 修复导航图标尺寸问题，明确设置 20px × 20px，添加透明度和 hover 状态。
  - 修复侧边栏固定高度问题，从 480px 改为 100% 自适应。
- **顶部导航栏优化**：桌面端（≥1024px）隐藏顶部导航栏，仅保留左侧侧边栏导航，避免双重导航。
- **道同构度主题优化**：
  - 环形进度条颜色主题调整为金色（≥80 分金色 / 60-79 玉色 / <60 朱砂），符合品牌主色定位。
  - 子指标文字颜色加深，标签从墨韵 400 改为墨韵 500，描述从墨韵 200 改为墨韵 300，提升可读性。
- **欢迎区样式优化**：渐变背景从玉色调整为金色调，与整体品牌主题保持一致。
- **快速操作区域修复**：修复标题颜色使用旧 CSS 变量的问题，改为玉色主题，替换正确的八卦图标。
- **底部状态栏样式修复**：全面更新状态栏样式，使用宣纸 400 背景 + 墨韵 400 文字，添加顶部边框，统一使用新设计系统变量。
- **页面验证**：验证仪表盘、记忆搜索、信任中心、船长日志、基准报告等主要页面样式均符合设计文档规范。

---

## [0.5.12] - 2026-06-24

### 新增

- **SpaceSniffer 式项目索引**：
  - `scan_roots()` 扫描所有可用驱动器根目录（C:\, D:\, G:\ 等），不再仅扫描 C:\Users\*
  - `scan_marker_projects()` 使用 walkdir 递归扫描，最大深度 5 层
  - `is_scan_ignored_dir()` 跳过系统目录（Windows、Program Files）和依赖目录（node_modules、target、.git）
  - `MAX_SCAN_ENTRIES` 从 200 增加到 5000，支持全盘扫描
  - 新增 `walkdir = "2"` 依赖

- **快捷方式扫描检测 AI 工具**（用户建议）：
  - 新增 `scan_shortcuts()` 函数，扫描桌面和开始菜单 .lnk 文件
  - 通过解析 .lnk 文件二进制内容匹配 exe_names（UTF-16LE + ASCII 编码）
  - 新增 `collect_shortcut_dirs()` 收集快捷方式目录（用户桌面、公共桌面、用户开始菜单、系统开始菜单）
  - 新增 `search_exe_in_lnk()` 和 `contains_subsequence()` 辅助函数
  - 解决问题：用户将 AI 工具安装在非标准目录（如 D:\Trae CN\、H:\CodeBuddy CN\）时无法检测

- **exe 文件扫描检测**：
  - `KnownTool` 结构体新增 `exe_names` 字段，存储每个工具的可执行文件名列表
  - 新增 `scan_exe_in_install_dirs()` 扫描常见安装目录中的可执行文件
  - 新增 `collect_install_dirs()` 收集跨平台安装目录
  - `check_known_tool()` 检测策略：binary_paths → exe_names 扫描 → 快捷方式扫描

### 修复

- **AI 工具数量显示错误**：`showReadyPanel` 过滤 `configured_agents`，只保留 `installed=true` 且 `supports_mcp=true` 的工具
- **CodeBuddy CN 全局规则未配置**：检测方式从 dot 目录改为 exe 文件扫描 + 快捷方式扫描
- **lrc-sidecar.exe 内存占用大**：`index_project()` 添加目录过滤（node_modules、target、.git）和文件大小限制（>1MB 跳过）
- **索引失败**：项目扫描从仅扫描 C:\Users\* 改为扫描所有驱动器根目录

### 变更

- 移除非 AI 工具（cloudbase-mcp、playwright-mcp）的 KNOWN_TOOLS 条目
- `Cargo.toml` 版本号 0.5.11 → 0.5.12
- `desktop/src-tauri/Cargo.toml` 版本号 0.5.11 → 0.5.12，新增 walkdir 依赖
- README.md 精简重写，突出基准测试评分和核心功能，使用说明书通过链接提供
- README.md 基准测试表格增加每个测试报告的超链接

### 测试

- 桌面端 44 个单元测试全部通过
- `test_print_installed_agents` 验证：检测到 2 个已安装 AI 工具（Trae CN + CodeBuddy）

---

## [0.5.11] - 2026-06-24

### 修复

- **全局规则路径修正**：`agent_detector.rs` 中规则文件写入路径从 `~/.trae-cn/rules/` 改为 `~/.trae-cn/user_rules/`
- **AI 工具数量显示错误**：`wizard.js` 的 `updateReadyPanelStatus` 添加 `supports_mcp` 过滤
- **仪表盘配色问题**：`app.css` 中 7 处硬编码颜色值替换为 CSS 变量或 rgba 值
- **缺少主题切换按钮**：`index.html` 添加主题切换按钮（☀️/🌙），`wizard.js` 添加 `initTheme`/`applyTheme`/`toggleTheme` 函数，localStorage 持久化

---

## [0.5.10] - 2026-06-24

### 变更

- `app.css` 中 7 处硬编码颜色值替换为 CSS 变量（commit 25ec8b3）
- 版本号更新至 0.5.10，触发 CI 构建

---

## [0.5.9] - 2026-06-24

### 修复

- **全局规则未安装**：路径修正 + 旧文件清理 + 手动写入正确规则
- **AI 工具数量错误**：`wizard.js` 的 `updateReadyPanelStatus` 添加 `supports_mcp` 过滤
- **仪表盘配色问题**：`app.css` 中硬编码颜色值替换为 CSS 变量
- **缺少主题切换按钮**：添加 ☀️/🌙 切换按钮，localStorage 持久化

---

## [0.5.8] - 2026-06-24

### 变更

- 前端文案审计与修复：完成页面引导、快速启动示例、Agent 配置描述、端口号提示、30秒体验测试内容
- LLM 配置字符串分隔符统一使用 `||`
- 复选框渲染逻辑统一使用 `installed && supports_mcp`
- Key 链接显示逻辑：ollama/custom 隐藏（`keyUrl: null`）
- 模型占位符动态更新
- LLM 提供商列表一致性：wizard、设置面板、常量均为 11 个提供商
- 术语统一：使用"AI 工具"而非"Agent"

---

## [0.5.7] - 2026-06-23

### 新增

- **桌面端 UIUX 设计规范完整应用**：
  - 引入 Catppuccin Latte 浅色主题（宣纸底色 `#F5F3EF` + 中国古典色系），护眼优先
  - 8px 网格间距系统（xs=4px / sm=8px / md=16px / lg=24px / xl=32px）
  - 字体规范：Inter（UI）+ JetBrains Mono（代码）+ Noto Sans SC（中文）
  - 组件圆角规范：按钮 6px / 卡片 12px / 模态框 16px
  - 三档阴影系统（low / medium / high）
  - 引入 Google Fonts（CSP 策略同步更新允许 fonts.googleapis.com 和 fonts.gstatic.com）

- **二次审计修复**：
  - `stop_sidecar` 锁顺序修复：缩小 sidecar 锁持有范围，避免 L1→L2 锁嵌套
  - `save_llm_config` / `clear_llm_config` 锁嵌套修复：释放 wizard 锁后再获取 sidecar_port 锁
  - `wizard.js` fallback 值修复：`var(--jade, #2ecc71)` → `var(--jade, #5B7C63)`（深色主题遗留色值）
  - `wizard.js` 残留硬编码颜色清理：`#555` / `#888` / `#f0f7ff` / `#0066cc` 全部替换为 CSS 变量

### 修复

- **全局规则路径修正**：根据 AI 工具官方文档规范 `get_rules_file_template` 全局规则写入路径，确保各 IDE 的规则文件路径符合官方规范
- **审计中危问题 M-3**：`start_sidecar` / `start_sidecar_for_project` 缩小 sidecar 锁持有范围，sidecar_port 更新移到锁释放后
- **审计中危问题 M-4**：`stop_sidecar` 锁顺序违反 L1→L2 约束，拆分为两个独立作用域
- **审计中危问题 M-15**：消除 `start_sidecar` / `start_sidecar_for_project` / `switch_project` 三处重复的 sidecar 启动后处理逻辑，提取为 `post_sidecar_start` 公共函数
- **审计低危问题 L-1**：`tracing_appender::rolling` 原子日志轮转，避免日志文件轮转时丢失
- **审计低危问题 L-5**：心跳协程 panic 恢复机制（`tokio::task::spawn` + `JoinError` 捕获）
- **审计低危问题 L-11**：`v1_api.rs` 缓存机制优化，减少重复计算

### 变更

- `Cargo.toml` 版本号 0.5.6 → 0.5.7
- `desktop/src-tauri/Cargo.toml` 版本号 0.5.6 → 0.5.7
- `desktop/src-tauri/tauri.conf.json` 版本号 0.5.6 → 0.5.7
- `desktop/package.json` 版本号 0.5.6 → 0.5.7
- `desktop/src/index.html` 版本号 v0.5.4 → v0.5.7（2 处）
- `desktop/src/styles.css` 从深色主题完全切换为 Latte 浅色主题
- `desktop/src-tauri/tauri.conf.json` CSP 策略更新：添加 `https://fonts.googleapis.com` 和 `https://fonts.gstatic.com`

### 测试

- 主项目 406 个单元测试全部通过
- 桌面端 44 个单元测试全部通过
- 基准测试 11 个测试全部通过
- Pre-commit hook 全绿（含算法泄露检测）

---

## [0.5.6] - 2026-06-23

### 修复

#### 修复一：写回性能瓶颈（O(N²) → O(N)）

- **问题**：每次 `recall` 后全量重写所有记忆（`clear_memories()` + 循环 `save_memory()`），3633 条记忆时单次 recall 写回耗时 ~105s，严重阻碍大规模记忆检索
- **修复**：
  - 在 `Persistence` trait 增加 `update_memories` 批量更新方法（默认实现为循环 `save_memory`，推荐具体后端重写）
  - `JsonPersistence` 重写 `update_memories` 为单次序列化 + 单次磁盘写入，仅更新被检索到的记忆（通常 ≤ top_k=10 条）
  - `recall` 函数写回逻辑从全量重写改为增量批量更新
- **效果**：大规模记忆场景下 recall 写回从 ~105s 降至毫秒级，3633 条记忆场景性能提升 10000 倍+
- **涉及文件**：`src/persistence/mod.rs`、`src/persistence/json.rs`、`src/memory_store.rs`

#### 修复二：TF-IDF 词边界检测

- **问题**：TF-IDF 检索使用 `contains()` 子串匹配，导致 "cat" 错误匹配 "category"、"rust" 匹配 "frustrated" 等英文单词误匹配
- **修复**：
  - 新增 `contains_word` 和 `count_word_occurrences` 辅助函数
  - 对长度 ≥ 3 的英文单词做词边界检测（检查匹配位置前后字符是否为非字母字符）
  - CJK bigram 和 2 字符 ASCII bigram 保留 `contains()` 子串匹配（适配中文检索和短词匹配）
- **效果**：英文检索精度提升，避免子串误匹配，同时保持中文检索能力
- **涉及文件**：`src/memory_store.rs`

### 新增

- **公平性改革 — 基准测试从"测架构"转变为"测能力"**：
  - 改革核心：将"验证架构"（测有没有洛书编码/LLM翻译器）转变为"验证效果"（测能不能做到知识更新/模糊查询/双关词区分）
  - 公平原则：不利用 ground truth，所有文档 importance=5（统一），蓄水池抽样随机文档
  - LRC 原生基准公平版：TF-IDF 模式 11/11 PASS（总评分 0.94），LLM 模式 9/11 PASS（总评分 0.79）
  - LongMemEval 公平版 v3：Session Recall@10=85.74%（不利用 has_answer 差异化）

- **6 次基准测试完整报告**：
  - MS MARCO BEIR 测试：TF-IDF MRR@10=0.7749，LLM MRR@10=0.8895（LLM 增益 +14.8%）
  - Natural Questions BEIR 测试：TF-IDF MRR@10=0.5389，LLM MRR@10=0.8016（LLM 增益 +48.7%）
  - HotpotQA BEIR 测试：TF-IDF MRR@10=0.7964，LLM MRR@10=0.9383（LLM 增益 +17.8%）
  - FiQA BEIR 测试：TF-IDF MRR@10=0.2729，LLM MRR@10=0.4453（LLM 增益 +63.2%）
  - LRC 原生基准测试（公平版）：TF-IDF 11/11 PASS，总评分 0.94
  - LongMemEval 基准测试（公平版 v3）：Session Recall@10=85.74%，Turn Recall@10=61.70%

- **BEIR 基准测试评估脚本**：
  - MS MARCO 评估脚本（`lrc_msmarco_eval.py`）：500 文档，100 查询，支持 TF-IDF 和 LLM 两种模式
  - Natural Questions 评估脚本（`lrc_nq_eval.py`）：500 文档，100 查询，适配 NQ 数据集特征（title + text 文档内容）
  - HotpotQA 评估脚本（`lrc_hotpotqa_eval.py`）：500 文档，100 查询，适配多跳推理场景
  - FiQA 评估脚本（`lrc_fiqa_eval.py`）：500 文档，100 查询，含多字节字符处理（避免 panic）
  - 蓄水池抽样随机文档，跳过合成记忆，BEIR 标准指标（MRR@10, Recall@10, Hit Rate@10）

- **LRC 原生基准测试公平版脚本**（`lrc_native_benchmark.py`）：
  - 11 项测试覆盖三层模型：通用检索、高级记忆能力、综合能力与信任
  - 公平性改革：L2 和 L3 的 6 个测试函数全部重构，从"测架构"变为"测能力"
  - 支持 TF-IDF 和 LLM 两种模式

- **LongMemEval 公平版评估脚本**：
  - v1（`lrc_real_retrieval_eval.py`）：仅会话级注入，importance=5（公平）
  - v2（`lrc_real_retrieval_eval_v2.py`）：Turn 级注入 + has_answer=8（不公平，对比用）
  - v3（`lrc_fair_eval_v3.py`）：Turn 级注入 + 统一 importance=5（公平，推荐）

- **基准测试报告目录**（`benchmarks/reports/`）：
  - 7 份分项报告 + 1 份汇总对比报告
  - 完整的评估方法、结果、分析和使用建议

### 性能优化

- **大规模记忆检索性能释放**：v0.5.6 修复一使 LRC 能够高效处理 500+ 文档的检索场景
  - 500 文档场景下 TF-IDF 平均检索仅 13ms（MS MARCO）/ 18ms（NQ）/ 21ms（HotpotQA）/ 19ms（FiQA）
  - P99 检索耗时仅 27ms（MS MARCO）/ 32ms（NQ）/ 39ms（HotpotQA）/ 44ms（FiQA）
  - LongMemEval 470 实例评估 < 60 秒，平均检索 2.6ms/查询

### 变更

- `Cargo.toml` 版本号 0.5.4 → 0.5.6
- `desktop/src-tauri/Cargo.toml` 版本号 0.5.4 → 0.5.6
- `desktop/src-tauri/tauri.conf.json` 版本号 0.5.5 → 0.5.6
- `desktop/package.json` 版本号 0.5.4 → 0.5.6

### 测试

- 新增 6 个单元测试：
  - `test_update_memories_partial_update`：验证增量批量更新
  - `test_update_memories_empty`：验证空输入处理
  - `test_contains_word_english_boundary`：验证英文词边界检测
  - `test_contains_word_cjk_bigram`：验证 CJK bigram 子串匹配
  - `test_contains_word_short_ascii_bigram`：验证短 ASCII bigram 子串匹配
  - `test_count_word_occurrences_boundary`：验证词频统计的词边界检测
- 全项目 406 个单元测试全部通过，clippy 无警告

---

## [0.5.5] - 2026-06-21

### 新增
- **MCP 配置自动升级**：Sidecar 启动时自动检测并升级旧版本 MCP 配置（stdio `loong-recall` → HTTP `lrc-memory`）
- **`auto_upgrade_configs()` 方法**：在 `agent_detector.rs` 中新增，sidecar 启动后自动调用
- **`config_needs_upgrade()` 方法**：检查配置是否包含旧的 stdio 模式配置项
- 用户升级 LRC Desktop 后无需重新运行配置向导，旧配置自动迁移

### 修复
- **MCP 工具不显示（"no tools yet"）**：根因是配置文件中是 stdio 模式 `loong-recall`，但 LRC Desktop 运行的是 HTTP sidecar。修复 `generate_config()` 始终生成 HTTP 模式配置；修复 `write_or_merge_config()` 清理旧配置名称
- **AI 主动调用 recall 未生效**：根因是 Trae 规则文件路径错误（`.trae/rules.md` → `.trae/rules/lrc-memory.md`）且缺少 `alwaysApply: true` frontmatter。修复 `get_rules_file_template()` 路径；添加 YAML frontmatter
- **AI 工具检测不准确**（检测出 9 个实际只有 2 个）：改进 `check_known_tool()` 检测策略，无 `binary_paths` 且无 `mcp_config_template` 的工具不自动检测
- **仪表盘"修改配置"按钮无反应**：移除只读卡片逻辑，统一使用完整 LLM 配置表单（多提供商选择）

### 变更
- `generate_config()` 始终生成 HTTP 模式配置（`type: "http"`, `url: "http://127.0.0.1:{port}/mcp"`）
- `write_or_merge_config()` 合并时自动清理旧配置名称（`loong-recall`, `lrc`, `lrc-memory-stdio`, `lrc-stdio`）
- `get_rules_file_template()` 路径更新为各 IDE 的标准规则文件路径
- `generate_ai_rules_content()` 添加 YAML frontmatter（Trae/Cursor）
- `write_ai_rules()` 清理旧路径规则文件，提取用户自定义内容并迁移
- `commands.rs` 在 `start_sidecar` 和 `start_sidecar_for_project` 中调用 `auto_upgrade_configs`

### 性能优化
- **关闭 `ml` feature 默认启用**：`default = ["server"]`，减少 sidecar 基线内存占用（candle 等重型依赖不再编译进二进制）
- **关闭后台结晶流水线 `run_on_start`**：延迟首次合成，避免启动内存峰值

### 安全
- 编译产物保密性确认：`Cargo.toml` 配置 `strip = true` + `lto = true` + `opt-level = "z"` + `codegen-units = 1` + `panic = "abort"`，符号信息已剥离

---

## [0.5.4] - 2026-06-20

### 新增
- 全项目静态代码审计报告
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 编译时与运行时反逆向工程保护增强（具体实现受 DaoTi Research License 保护）
- DPAPI 密钥损坏自动恢复机制

### 修复
- 修复所有 Clippy 警告（doc_lazy_continuation 等）
- 消除 tray.rs 中的 unwrap() 调用
- PostgresPersistence 新增 `block_on_async` 封装 tokio 运行时处理
- encoder_codebert::encode 返回 Result 类型，正确传播错误

---

## [0.5.1] - 2026-06-18

### 修复
- **P1-1**: server.rs 巨型函数拆分（964行 → 5个函数）
- **P1-2**: 模型加载逻辑重复（提取共享 PoolingStrategy 到 `src/engine/pooling.rs`）
- **P1-3**: RRF 融合逻辑重复（提取共享 `src/engine/rrf.rs` 模块）
- **P1-4**: synthesis_engine 循环内重复构建 HashSet
- **P1-5**: synthesis_engine 测试覆盖（14 个测试用例）
- **P1-6**: 速率限制器集成（AppStore 集成 + 关键命令保护）
- **P1-7**: SidecarManager Drop 等待退出（进程泄漏修复）
- **P1-8**: Agent 检测器扫描深度限制（MAX_SCAN_ENTRIES=200）
- **P2-1**: Dockerfile 缺少 static/ 复制
- **P2-2**: JSON 全量读写 O(n) 瓶颈（RwLock 内存缓存）
- **P2-3**: CI 仅 Windows runner（三平台矩阵策略）
- **P2-4**: 前端 CSS 内联 1260 行（提取到 `static/app.css`）
- **P2-5**: 前端 app.js 全局变量污染（IIFE 隔离）

### 变更
- 前端版本号一致性（统一从 Cargo.toml 读取）
- 新增 `src/engine/pooling.rs`、`src/engine/rrf.rs`、`static/app.css`

---

## [0.5.0] - 2026-06-17

### 新增
- 一键安装脚本（`scripts/install.ps1` / `scripts/install.sh`）
- v0.5.0 用户使用手册（`docs/v0.5.0_用户使用手册.md`）
- v0.5.0 开发者指南（`docs/v0.5.0_开发者指南.md`）
- v0.5.0 安全架构白皮书（`docs/v0.5.0_安全架构白皮书.md`）
- v0.5.0 综合修复与发布方案（`docs/v0.5.0_综合修复与发布方案.md`）
- 控制流平坦化反逆向工程支持
- 反内存 dump 保护（敏感数据使用后清零）
- DPAPI 密钥损坏自动恢复机制

### 修复
- **P0-1**: 多项目/多窗口/多IDE 隔离（Sidecar 进程管理 + 项目指纹）
- **P0-2**: wizard.js XSS 风险（HTML 转义 + CSP 头）
- **P0-3**: 密钥与密文同目录存储（AES-256-GCM + DPAPI 加密）
- **P0-4**: SHA-256 完整性校验（build.rs 编译时生成 + 运行时校验）
- **P0-5**: Qdrant 数据持久化（添加 collection 存在性检查）
- **P0-6**: 系统托盘面板（动态 tooltip + 项目切换菜单）
- **P0-7**: 前端版本号一致性（统一从 Cargo.toml 读取）
- **P1-1**: server.rs 巨型函数拆分（964行 → 5个函数）
- **P1-2**: 模型加载逻辑重复（提取共享 PoolingStrategy）
- **P1-3**: RRF 融合逻辑重复（提取共享 rrf.rs 模块）
- **P1-4**: synthesis_engine 循环内重复构建 HashSet
- **P1-5**: synthesis_engine 测试覆盖（14 个测试用例）
- **P1-6**: 速率限制器集成（AppStore 集成 + 关键命令保护）
- **P1-7**: SidecarManager Drop 等待退出（进程泄漏修复）
- **P1-8**: Agent 检测器扫描深度限制（MAX_SCAN_ENTRIES=200）
- **P2-1**: Dockerfile 缺少 static/ 复制
- **P2-2**: JSON 全量读写 O(n) 瓶颈（RwLock 内存缓存）
- **P2-3**: CI 仅 Windows runner（三平台矩阵策略）
- **P2-4**: 前端 CSS 内联 1260 行（提取到 app.css）
- **P2-5**: 前端 app.js 全局变量污染（IIFE 隔离）
- **T-01**: Neo4j subgraph 真正使用 Cypher 查询（可变长度路径 + 本地兜底）
- **T-11**: wizard.js 空 catch 块（添加错误日志和用户提示）
- **T-12**: desktop/commands.rs eval 安全加固（URL 白名单验证 + 单引号转义）
- DPAPI 密钥损坏自动恢复（解密失败时删除损坏文件并重新生成）

### 安全
- 桌面端 URL 导航白名单验证（仅允许 127.0.0.1）
- 敏感数据使用后内存清零（SecureString 模式）
- 字符串编译时混淆（obfstr）
- 代码签名文档（自签名 + EV 证书方案）

### 文档
- 新增 4 份 v0.5.0 文档（用户手册、开发者指南、安全白皮书、综合方案）
- 更新 README.md 版本号和安装方式
- 新增安装脚本（Windows + Linux/macOS）

---

## [0.4.0] - 2026-06-15

### 新增
- 洛书 9 维坐标编码器
- 镜像梯形递归算子
- 八卦分类投影
- 双重衰减模型（时间+拓扑双因子）
- 合成引擎（并查集聚类 + 洛书递归）
- Dao 自适应调节器（自愈系统）
- RRF 双路检索融合
- MCP 协议接口（13 个工具）
- REST v1 API（11 个端点）
- Web 仪表盘（Tauri 2 桌面端）
- 系统托盘集成
- JSON/PostgreSQL/Qdrant/Neo4j 多后端支持
- 多语言代码切分器（chunker）
- 审计追踪（audit_trail）
- 复杂度预算与红线检查
- 道枢演化协议
- 用户反馈回路
- 系统健康报告
- A/B 测试框架
- 基准测试框架

### 安全
- 反逆向防护（IsDebuggerPresent + CheckRemoteDebuggerPresent）
- 进程守护（process_guard）
- 数据加密（AES-256-GCM）
- 配置持久化

---

## 许可证说明

- **公开层** (L1): Apache 2.0 — `src/bin/`, `src/persistence/`, `src/chunker.rs`, `static/`, `desktop/`
- **引擎层** (L2): DaoTi Research License v1.0 — `src/engine/`
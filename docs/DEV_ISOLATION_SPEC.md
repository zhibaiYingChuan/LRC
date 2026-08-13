# 开发版本与稳定版本隔离规范

> 版本: v0.9.0  
> 最后更新: 2026-08-12  
> 核心原则：**开发版和稳定版在所有维度上完全隔离，零交集。**

---

## 一、隔离维度总览

| 维度 | 稳定版 | 开发版 | 隔离方式 |
|------|--------|--------|----------|
| Sidecar 端口 | 3099 | 3111 | `--dev` CLI 标志 / `dev_mode_port()` |
| CDP 调试端口 | 9230 | 9231 | `TAURI_DEV` 环境变量 |
| 数据目录 | `~/.loong-recall/global/data/` | `~/.loong-recall/dev/data/` | `--dev` CLI 标志 / `dev_mode_data_dir()` |
| 全局锁文件 | `~/.loong-recall/.lrc.lock` | `~/.loong-recall/.lrc-dev.lock` | `LRC_DEV_MODE` 环境变量 |
| 向导配置 (wizard.json) | `%APPDATA%\LoongRecall\wizard.json` | `%APPDATA%\LoongRecall\dev\wizard.json` | `TAURI_DEV` / `LRC_DEV_MODE` 环境变量 |
| 运行配置 (config.json) | `%APPDATA%\LoongRecall\config.json` | `%APPDATA%\LoongRecall\dev\config.json` | `LRC_DEV_MODE` 环境变量 |
| 前端默认端口 | 3099 | 3111 (meta 注入) | `<meta name="lrc-sidecar-port">` 注入 |
| MCP 自动升级 | 执行 | **禁止** | `is_dev_mode()` 守卫 |
| IDE 规则写入 | 执行 | **禁止** | `is_dev_mode()` 守卫 |
| Agent 配置 (configure_agents) | 允许 | **禁止** | `is_dev_mode()` 守卫 |
| IDE 安装 (--install-ide) | 允许 | **禁止** | `dev_mode` 变量守卫 |
| 桌面端启动探测 | 取第一个 (probed[0]) | 只取 3111（找不到则跳过） | `is_dev_mode()` 分支 |

---

## 二、涉及文件清单

### 后端 Sidecar

| 文件 | 修改内容 | 恢复方法 |
|------|----------|----------|
| `src/bin/server.rs` | `--dev` 标志: 端口=3111, 设 `LRC_DEV_MODE=1`, 数据目录=`dev/data/` | 移除 `--dev` 分支即可 |
| `src/bin/server.rs` | `--install-ide`: dev 模式下拒绝执行 | 移除 `dev_mode` 检查 |
| `src/bin/server.rs` | `load_llm_from_wizard_json()`: dev 模式读 `dev/wizard.json` | 移除 `is_dev` 判断 |
| `src/server.rs` | `wizard_json_path()`: dev 模式返回 `dev/wizard.json` | 移除 `is_dev` 判断 |
| `src/server.rs` | `save_llm_to_wizard_json()`: 使用 `wizard_json_path()` 辅助函数 | 自动跟随 |
| `src/server.rs` | `AppState` 新增 `dev_mode: bool` 字段 | 移除字段和所有引用 |
| `src/v1_api.rs` | `/health/system` 返回 `dev_mode` 和 `consolidation` 字段 | 移除新增字段 |
| `src/data_dir.rs` | `global_lock_path()`: dev 模式返回 `.lrc-dev.lock` | 移除 `is_dev` 判断 |
| `src/config.rs` | `get_config_path()`: dev 模式返回 `dev/config.json` | 移除 `is_dev` 判断 |

### 桌面端 Tauri

| 文件 | 修改内容 | 恢复方法 |
|------|----------|----------|
| `desktop/src-tauri/src/commands.rs` | `dev_mode_port()`: 返回 3111 | 改回 3100 或删除函数 |
| `desktop/src-tauri/src/commands.rs` | `open_dashboard_window()`: dev 默认 3111 | 改回 `unwrap_or(3099)` |
| `desktop/src-tauri/src/commands.rs` | `navigate_main_to_dashboard()`: dev 默认 3111 | 改回 `unwrap_or(3099)` |
| `desktop/src-tauri/src/commands.rs` | `post_sidecar_start()`: dev 模式跳过 MCP 升级和规则写入 | 移除 `is_dev_mode()` 守卫 |
| `desktop/src-tauri/src/commands.rs` | `configure_agents()`: dev 模式拒绝执行 | 移除 `is_dev_mode()` 守卫 |
| `desktop/src-tauri/src/commands.rs` | `switch_project()`: dev 模式跳过规则写入 | 移除 `is_dev_mode()` 守卫 |
| `desktop/src-tauri/src/main.rs` | 启动探测: dev 只取 3111（找不到则跳过，不退回到 3099） | 恢复为 `probed[0]` 逻辑 |
| `desktop/src-tauri/src/main.rs` | CDP 端口: dev=9231, stable=9230 | 改回硬编码 9230 |
| `desktop/src-tauri/src/tray.rs` | `open_dashboard()`: dev 默认 3111 | 改回 `unwrap_or(3099)` |
| `desktop/src-tauri/src/config_wizard.rs` | `config_path()`: dev 模式用 `dev/wizard.json` | 移除 `is_dev_mode()` 调用 |
| `desktop/package.json` | `dev` 脚本: `set LRC_DEV_MODE=1 && tauri dev` | 改回 `tauri dev` |
| `desktop/src-tauri/tauri.conf.json` | (无变更，dev 环境变量通过 package.json 设置) | - |

### 前端

| 文件 | 修改内容 | 恢复方法 |
|------|----------|----------|
| `static/app.js` | `DEV_DEFAULT_PORT` / `STABLE_DEFAULT_PORT` 常量 | 删除常量，改回硬编码 3099 |
| `static/app.js` | `<meta name="lrc-sidecar-port">` 端口同步读取 | **保留**（稳定版也需此功能） |

---

## 三、触发机制

### 开发模式激活条件

以下任一条件满足即进入开发模式：

```
环境变量 LRC_DEV_MODE=1      # npm run dev 脚本设置 / --dev CLI 标志设置
CLI 参数 --dev               # sidecar 启动时传入
```

**注意**：`TAURI_DEV` 环境变量仅在 Tauri CLI 内部使用，**不会**传递给 Rust 进程。因此桌面端 `is_dev_mode()` 依赖 `LRC_DEV_MODE` 环境变量（由 `package.json` 的 `dev` 脚本设置）。

### 稳定模式

上述条件均不满足时为稳定模式。

---

## 四、打包前恢复清单

> 发布正式版本前，必须逐项检查并恢复：

- [ ] `package.json` — `dev` 脚本改回 `tauri dev`（移除 `set LRC_DEV_MODE=1 &&`）
- [ ] `commands.rs` — `dev_mode_port()` 恢复为 3100 或删除
- [ ] `commands.rs` — `open_dashboard_window()` 改回 `unwrap_or(3099)`
- [ ] `commands.rs` — `navigate_main_to_dashboard()` 改回 `unwrap_or(3099)`
- [ ] `commands.rs` — `post_sidecar_start()` 移除 `is_dev_mode()` 守卫
- [ ] `commands.rs` — `configure_agents()` 移除 `is_dev_mode()` 守卫
- [ ] `commands.rs` — `switch_project()` 移除 `is_dev_mode()` 守卫
- [ ] `main.rs` — 启动探测恢复为直接取 `probed[0]`
- [ ] `main.rs` — CDP 端口改回硬编码 `9230`
- [ ] `tray.rs` — `open_dashboard()` 改回 `unwrap_or(3099)`
- [ ] `config_wizard.rs` — `config_path()` 移除 `is_dev_mode()` 分支
- [ ] `server.rs` — `wizard_json_path()` 移除 `is_dev` 判断（或删除函数）
- [ ] `bin/server.rs` — `--dev` 标志中移除 `std::env::set_var("LRC_DEV_MODE", "1")`
- [ ] `bin/server.rs` — `--install-ide` 移除 `dev_mode` 检查
- [ ] `bin/server.rs` — `load_llm_from_wizard_json()` 移除 `is_dev` 判断
- [ ] `config.rs` — `get_config_path()` 移除 `is_dev` 判断
- [ ] `data_dir.rs` — `global_lock_path()` 移除 `is_dev` 判断
- [ ] `app.js` — 删除 `DEV_DEFAULT_PORT` 常量
- [ ] 版本号回写（10 处同步）
- [ ] `preflight_check.ps1` 全通过
- [ ] 引擎泄露检测通过
- [ ] `hcse-release-compliance` 智能体审计通过

---

## 五、关键隔离逻辑详解

### 5.1 端口隔离

```
稳定版 sidecar:  3099
开发版 sidecar:  3111

开发版 sidecar 启动命令（必须加 --daemon）:
  G:\rust-target\release\code-memory-server.exe --dev --daemon --global

桌面端探测:
  - 扫描端口 3099-3198
  - 稳定版: 取第一个 (probed[0])
  - 开发版: 只取 3111，找不到则跳过（不设置 sidecar_port，禁止回退到 3099）
```

### 5.2 数据目录隔离

```
稳定版: ~/.loong-recall/global/data/
开发版: ~/.loong-recall/dev/data/

sidecar 启动: --dev → 端口 3111 + 数据目录 ~/.loong-recall/dev/data/
桌面端启动: dev_mode_data_dir() → ~/.loong-recall/dev/data/
```

### 5.3 配置文件隔离

```
wizard.json:
  稳定版: %APPDATA%\LoongRecall\wizard.json
  开发版: %APPDATA%\LoongRecall\dev\wizard.json

config.json:
  稳定版: %APPDATA%\LoongRecall\config.json
  开发版: %APPDATA%\LoongRecall\dev\config.json

桌面端: config_wizard.rs::config_path() 判断 is_dev_mode()
sidecar: wizard_json_path() / get_config_path() 判断 LRC_DEV_MODE 环境变量
```

### 5.4 锁文件隔离

```
稳定版: ~/.loong-recall/.lrc.lock
开发版: ~/.loong-recall/.lrc-dev.lock

data_dir.rs::global_lock_path() 判断 LRC_DEV_MODE 环境变量
```

### 5.5 CDP 调试端口隔离

```
稳定版: 9230
开发版: 9231

main.rs 启动时根据 LRC_DEV_MODE 选择端口，在 Tauri::Builder 之前执行
```

### 5.6 前端 API 端点隔离

```
前端通过 <meta name="lrc-sidecar-port"> 注入正确端口
兜底值:
  - 稳定版: STABLE_DEFAULT_PORT = 3099
  - 开发版: 依赖 meta 注入 + META_SIDECAR_PORT 变量

【重要】index.html 中必须包含 <meta name="lrc-sidecar-port" content="3111">
标签，否则前端 app.js 的 _readSidecarPortFromMeta() 返回 null，
DEFAULT_API_BASE 回退到 STABLE_DEFAULT_PORT (3099)，导致所有 API 
请求指向稳定版。此标签是前端隔离的唯一入口点，删除或遗漏会导致
"桌面端明明是 v0.9.0 但却显示 v0.8.48 数据"的现象。
```

### 5.7 MCP 自动升级 / IDE 规则写入 / Agent 配置隔离

```
以下操作在开发模式下**一律禁止**，通过 is_dev_mode() 守卫:

1. post_sidecar_start() — MCP 自动升级 + 全局 IDE 规则写入
   文件: commands.rs:42-48

2. configure_agents() — 用户手动配置 Agent（写 MCP 配置 + 规则文件）
   文件: commands.rs:1553-1558

3. switch_project() — 切换项目后的规则文件写入
   文件: commands.rs:2116

4. --install-ide CLI — sidecar 直接安装 IDE 配置
   文件: bin/server.rs:468-470

违规后果：开发版端口 (3111) 被写入用户的全局 IDE 配置，导致 IDE 连接失败。
```

---

## 六、禁止事项

1. **禁止在开发环境修改稳定版文件**：包括 `C:\Users\Administrator\AppData\Local\LRC Desktop\` 下的任何文件
2. **禁止开发版 sidecar 绑定 3099 端口**
3. **禁止开发版使用稳定版数据目录** (`~/.loong-recall/global/data/`)
4. **禁止开发版读写稳定版 `wizard.json`** 或 `config.json`
5. **禁止开发版获取稳定版锁文件** (`.lrc.lock`)
6. **禁止代码中硬编码 3099 端口**（应使用常量 + dev_mode 判断）
7. **【严重】禁止开发版桌面端自动升级用户的 MCP 配置**（v0.9.0 教训）
8. **【严重】禁止开发版桌面端写入全局 IDE 规则文件**（v0.9.0 教训）
9. **【严重】禁止开发版桌面端执行 `configure_agents` 命令**（v0.9.0 教训）
10. **【严重】禁止开发版 sidecar 执行 `--install-ide` 命令**（v0.9.0 教训）
11. **禁止开发版探测时回退到稳定版端口** — 找不到 3111 就跳过，不选 3099

---

## 七、测试验证方法

```powershell
# 1. 确认稳定版 sidecar 正常运行（端口 3099）
netstat -ano | Select-String "3099.*LISTENING"

# 2. 启动开发版 sidecar（端口 3111，**必须加 --daemon**）
$env:LRC_DEV_MODE="1"
Start-Process -FilePath "G:\rust-target\release\code-memory-server.exe" `
  -ArgumentList "--dev","--daemon","--global" -WindowStyle Hidden -PassThru

# 3. 验证两个 sidecar 互不干扰
netstat -ano | Select-String "3111.*LISTENING|3099.*LISTENING"
# 应显示两条记录

# 4. 验证开发版锁文件独立
ls ~/.loong-recall/.lrc-dev.lock

# 5. 验证开发版配置文件独立
ls $env:APPDATA\LoongRecall\dev\wizard.json
ls $env:APPDATA\LoongRecall\dev\config.json

# 6. 验证 MCP 配置未被修改（端口仍为 3099）
Select-String "3099" "$env:APPDATA\Trae CN\User\mcp.json"

# 7. 编译桌面开发版
cd desktop; npm run dev

# 8. 检查日志确认端口选择正确
# 应显示: 启动时探测：检测到外部 sidecar，端口 3111 [开发模式]
# 应显示: [开发模式] 跳过 MCP 自动升级和 IDE 规则写入

# 9. 通过 CDP 连接桌面端 WebView2 进行交互测试
# 开发版 CDP 端口: http://127.0.0.1:9231/json
```

---

## 八、审计记录

> 以下记录了每次全局审计发现的问题和修复。

| 日期 | 审计发现 | 修复状态 |
|------|----------|----------|
| 2026-08-12 | **H1**: `configure_agents` 命令无 dev 保护 | 已修复 |
| 2026-08-12 | **H2**: `switch_project` 规则写入无 dev 保护 | 已修复 |
| 2026-08-12 | **H3**: `--install-ide` CLI 无 dev 保护 | 已修复 |
| 2026-08-12 | **M1**: `config.json` 路径无 dev 隔离 | 已修复 |
| 2026-08-12 | **L1**: 探测兜底 `or_else(probed.first())` 可能回退稳定版 | 已修复（dev 找不到 3111 直接跳过） |
| 2026-08-12 | **教训**: 开发版桌面端自动升级了用户的 Trae CN 和 Gemini MCP 配置（3099→3111） | 已加 `is_dev_mode()` 守卫并恢复用户配置 |

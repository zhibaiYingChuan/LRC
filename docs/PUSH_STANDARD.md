# LRC 仓库推送规范

> 本文档规范 LRC（Loong Recall）项目推送到 GitHub 仓库的全部流程，确保推送内容干净、可复现、可追溯。
> 适用于所有版本发布与日常提交，后续推送必须遵循此标准。

---

## 一、文件分类原则

### 1.1 必须推送（生产交付物）

| 类别 | 路径示例 | 说明 |
|------|---------|------|
| Rust 源码 | `src/**/*.rs`、`build.rs`、`Cargo.toml`、`Cargo.lock` | 后端核心 |
| 桌面端源码 | `desktop/src-tauri/src/**/*.rs` | Tauri 桌面应用后端（Rust） |
| 静态资源 | `static/index.html`、`static/app.js`、`static/app.css`、`static/colors_and_type.css`、`static/components.css`、`static/assets/**` | 仪表盘前端，被 sidecar `include_str!` 内嵌；`tauri.conf.json` 的 `frontendDist` 指向 `../../static` |
| 构建配置 | `desktop/src-tauri/tauri.conf.json`、`desktop/src-tauri/Cargo.toml`、`desktop/package.json` | 构建系统配置 |
| CI/CD | `.github/workflows/*.yml` | GitHub Actions 工作流 |
| 用户文档 | `README.md`、`CHANGELOG.md`、`docs/USER_GUIDE.md`、`docs/PRODUCT_ROADMAP_v1.0.md`、`LICENSE`、`LICENSE_CODE` | 面向用户的文档 |
| 基准测试 | `benchmarks/**` | 基准测试报告与脚本 |
| 预设模板 | `templates/**` | 用户可用的预设模板 |

> **注意**：`desktop/src/` 下的旧前端文件（index.html、styles.css、wizard.js）已于 v0.6.0 删除，被 `static/` 下的新仪表盘完全取代。**禁止**重新引入 `desktop/src/` 目录作为前端，所有前端资源统一放在 `static/`。

### 1.2 禁止推送（本地开发产物）

| 类别 | 路径模式 | .gitignore 规则 |
|------|---------|----------------|
| 本地审计报告 | `docs/AUDIT_REPORT_*.md`、`docs/CROSS_PLATFORM_AUDIT_*.md` | 已忽略 |
| 本地测试报告 | `docs/TEST_REPORT_*.md`、`desktop/TEST_REPORT_*.md` | 已忽略 |
| 本地修复计划 | `docs/FIX_PLAN_*.md` | 已忽略 |
| 本地设计文档 | `docs/LRC*全案*.md`、`docs/PRD_*.md` | 已忽略 |
| 本地测试脚本 | `desktop/test-*.mjs`、`test-cdp.ps1`、`tests/e2e_*.ps1` | 已忽略 |
| 本地 UI Kit | `loong-recall/` | 已忽略 |
| 本地预览文件 | `static/logo-preview.html` | 已忽略 |
| 构建产物 | `target/`、`desktop/src-tauri/target/`、`desktop/node_modules/`、`*.exe`、`*.msi`、`*.dmg` | 已忽略 |
| 二进制目录 | `desktop/src-tauri/binaries/` | 已忽略 |
| 模型文件 | `*.safetensors`、`*.onnx`、`*.pt`、`models/` | 已忽略 |
| 运行时数据 | `.loong-recall/`、`data/`、`*.log` | 已忽略 |
| IDE 配置 | `.idea/`、`.vscode/`、`.trae/`、`.cursor/` | 已忽略 |
| 签名密钥 | `*.p12`、`*.pem`、`*.key`、`*.crt` | 已忽略 |

### 1.3 判定原则

- **生产必需判定**：被 `include_str!`/`include_bytes!` 引用的文件必须推送
- **本地开发判定**：仅用于开发期调试、测试、审计的文件不推送
- **文档判定**：面向用户使用的文档推送，内部开发文档不推送
- **安全判定**：含密钥、证书、个人配置的文件绝不推送

---

## 二、提交前检查清单

### 2.1 必做检查（每次提交前）

```powershell
# 1. 确认当前分支
git branch --show-current

# 2. 查看待提交文件
git status --short

# 3. 确认无本地测试/开发文档泄露
git ls-files --others --exclude-standard --directory
# 期望结果：仅显示生产必需的新增文件

# 4. 确认无敏感信息
git diff --cached --name-only | Select-String -Pattern '\.env|\.key|\.pem|\.p12|secret|credential'
# 期望结果：无匹配

# 5. 核心算法泄露检测（强制，0 错误才允许提交）
python scripts/check_algorithm_leak.py --verbose
# 期望输出：通过: 公开层文件无核心算法泄露

# 6. 编译验证（主项目）
cargo build --release --features server

# 7. 编译验证（桌面端，发布前才需要）
cd desktop; npm run build; cd ..
```

### 2.2 发布前检查（版本发布时额外执行）

```powershell
# 1. 单元测试
cargo test --features server,ml

# 2. 桌面端单元测试
cd desktop/src-tauri; cargo test; cd ../..

# 3. 版本号一致性检查
# 确认以下文件版本号一致：
# - Cargo.toml (version 字段)
# - desktop/src-tauri/Cargo.toml (version 字段)
# - desktop/src-tauri/tauri.conf.json (version 字段)
# - desktop/package.json (version 字段)
# - CHANGELOG.md (最新版本标题)

# 4. CHANGELOG.md 已更新
# 5. README.md 内容实事求是（无虚假数据）
# 6. 用户文档已同步更新
```

---

## 三、提交信息规范

### 3.1 格式

```
<type>(<scope>): <subject>

<body>

<footer>
```

### 3.2 Type 取值

| Type | 说明 | 示例 |
|------|------|------|
| `feat` | 新功能 | feat(engine): 新增通用语义引擎支持 BGE 模型 |
| `fix` | Bug 修复 | fix(desktop): 修复 Tauri WebView API_BASE 解析错误 |
| `docs` | 文档更新 | docs: 更新用户指南移除 Ollama 内容 |
| `style` | 代码格式 | style: 统一使用 CSS 变量替代硬编码颜色 |
| `refactor` | 重构 | refactor(server): 提取静态资源路由处理函数 |
| `test` | 测试 | test: 新增 model_downloader 18 个单元测试 |
| `chore` | 构建/工具 | chore(ci): 更新 release workflow 三平台矩阵 |
| `perf` | 性能优化 | perf(recall): recall 写回从 O(N²) 优化到 O(N) |

### 3.3 Scope 取值

`engine`、`server`、`desktop`、`ui`、`docs`、`ci`、`security`、`config`、`memory`、`consolidation`、`benchmark`

### 3.4 规则

- subject 不超过 50 字符，使用祈使句（"添加"而非"添加了"）
- body 解释"为什么"而非"做了什么"（代码差异已说明做了什么）
- footer 引用 issue：`Closes #123`、`Refs #456`
- **中文提交信息**：项目面向中文用户，提交信息使用简体中文

### 3.5 示例

```
feat(desktop): 新增用户友好功能三件套

- 仪表盘状态栏点击"已停止/不可达"弹出启动服务弹窗
- 右下角数据目录点击直接打开文件夹
- API 文档页改造为综合用户文档模块

Closes #v0.6.0
```

---

## 四、版本号规范

遵循 [语义化版本](https://semver.org/lang/zh-CN/)：

```
MAJOR.MINOR.PATCH
```

| 版本类型 | 触发条件 | 示例 |
|---------|---------|------|
| MAJOR | 不兼容的 API 修改 | 1.0.0 → 2.0.0 |
| MINOR | 向下兼容的功能新增 | 0.5.18 → 0.6.0 |
| PATCH | 向下兼容的 Bug 修复 | 0.6.0 → 0.6.1 |

### 4.1 版本号同步要求

发布新版本时，以下文件版本号必须一致：

1. `Cargo.toml` → `version` 字段
2. `desktop/src-tauri/Cargo.toml` → `version` 字段
3. `desktop/src-tauri/tauri.conf.json` → `version` 字段
4. `desktop/package.json` → `version` 字段
5. `static/app.js` → `APP_VERSION` 常量（前端版本号唯一来源，status-version 和日志前缀均引用）
6. `static/index.html` → 显示给用户的版本号（`id="sys-version"` 和 `id="status-version"` 初始值 + `<meta name="version">`）
7. `CHANGELOG.md` → 最新版本标题

### 4.2 当前版本

- **当前版本**：0.8.7
- **下一版本**：0.8.8（Bug 修复）或 0.9.0（功能新增）

---

## 五、三平台构建规范

### 5.1 构建矩阵

| 平台 | Runner | Target | 产物 |
|------|--------|--------|------|
| Windows | `windows-latest` | `x86_64-pc-windows-msvc` | sidecar.exe + MSI + NSIS |
| macOS | `macos-latest` | `aarch64-apple-darwin` | sidecar + DMG |
| Linux | `ubuntu-latest` | `x86_64-unknown-linux-gnu` | sidecar + deb + AppImage |

### 5.2 触发方式

#### 方式一：标签触发（推荐）

```powershell
# 1. 确认所有修改已提交并推送
git status

# 2. 创建标签
git tag v0.6.0

# 3. 推送标签（触发构建）
git push origin v0.6.0
```

#### 方式二：手动触发（workflow_dispatch）

在 GitHub Actions 页面选择 `Release` workflow，输入版本号（如 `v0.6.0`），点击 `Run workflow`。

### 5.3 构建流程（自动）

1. **Job 1: build-sidecar** — 编译 sidecar 二进制（三平台并行）
   - `cargo build --release --features server --target <target>`
   - macOS Ad-Hoc 签名
   - 生成 SHA256 校验文件
2. **Job 2: build-desktop** — 编译桌面端安装包（三平台并行）
   - 先编译 sidecar，复制到 `desktop/src-tauri/lrc-sidecar[.exe]`
   - `cargo tauri build --target <target>`
   - macOS Ad-Hoc 签名
   - 收集 MSI/NSIS/DMG/deb/AppImage
3. **Job 3: release** — 创建 GitHub Release
   - 下载所有构建产物
   - 生成 `install.sh` 一键安装脚本
   - 生成 `REPRODUCIBLE_BUILDS.md` 可复现构建验证指南
   - 创建 Release 并上传所有文件

### 5.4 产物命名规范

```
# Sidecar 二进制
lrc-<version>-linux-x86_64
lrc-<version>-macos-arm64
lrc-<version>-windows-x86_64.exe

# 桌面端安装包
lrc-desktop-<version>-windows-x86_64.msi
lrc-desktop-<version>-windows-x86_64-setup.exe
lrc-desktop-<version>-macos-arm64.dmg
lrc-desktop-<version>-linux-amd64.deb
lrc-desktop-<version>-linux-x86_64.AppImage

# 校验文件
<artifact>.sha256
```

### 5.5 可复现构建

所有发布产物支持可复现构建验证：

```bash
# 用户验证步骤
git clone https://github.com/zhibaiYingChuan/LRC.git
cd LRC
git checkout v<VERSION>
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
export RUSTFLAGS="-C link-arg=-Wl,--build-id=sha1"
cargo build --release --features server --target <YOUR_TARGET>
sha256sum target/<YOUR_TARGET>/release/code-memory-server
# 与 Release 中的 .sha256 对比
```

---

## 六、推送流程

### 6.1 日常提交推送

```powershell
# 1. 暂存指定文件（避免 git add . 误添加）
git add <file1> <file2> ...

# 2. 提交（使用规范提交信息）
git commit -m "feat(scope): 简短描述" -m "详细说明"

# 3. 推送
git push origin <branch>
```

### 6.2 版本发布推送

```powershell
# 1. 完成所有开发与测试
# 2. 更新版本号（同步所有文件）
# 3. 更新 CHANGELOG.md
# 4. 更新用户文档
# 5. 提交版本发布
git add .
git commit -m "chore(release): 发布 v0.6.0"

# 6. 推送主分支
git push origin main

# 7. 创建并推送标签（触发 CI 构建）
git tag v0.6.0
git push origin v0.6.0

# 8. 监控 GitHub Actions 构建状态
# https://github.com/zhibaiYingChuan/LRC/actions

# 9. 构建成功后，编辑 GitHub Release 描述
# https://github.com/zhibaiYingChuan/LRC/releases
```

### 6.3 紧急回滚

```powershell
# 仅回滚最近的提交（保留更改）
git reset --soft HEAD~1

# 回滚并丢弃更改（谨慎使用）
git reset --hard HEAD~1

# 已推送的回滚（创建回滚提交，推荐）
git revert <commit-hash>
git push origin <branch>
```

---

## 七、文档规范

### 7.1 实事求是原则

- **基准测试数据**：必须来自真实运行结果，禁止编造
- **功能描述**：仅描述已实现的功能，禁止描述未实现的功能
- **测试结果**：必须基于真实测试，禁止虚假测试报告
- **截图与示例**：必须来自实际运行，禁止 PS 修改

### 7.2 用户文档维护

| 文档 | 维护时机 | 负责内容 |
|------|---------|---------|
| `README.md` | 每次版本发布 | 项目简介、基准测试、快速开始 |
| `CHANGELOG.md` | 每次提交 | 变更记录 |
| `docs/USER_GUIDE.md` | 功能变更时 | 用户使用指南 |
| `docs/PRODUCT_ROADMAP_v1.0.md` | 路线图调整时 | 产品路线图 |

### 7.3 CHANGELOG.md 格式

```markdown
## [版本号] - 日期

### 新增
- 功能描述（文件路径引用）

### 变更
- 变更描述

### 修复
- 修复描述

### 测试
- 测试结果
```

---

## 八、安全规范

### 8.1 禁止推送的敏感信息

- API 密钥、Token、密码
- 私钥文件（`.pem`、`.key`、`.p12`）
- 数据库连接字符串
- 用户个人数据
- `.env` 环境变量文件

### 8.2 安全检查

```powershell
# 提交前扫描敏感信息
git diff --cached --name-only | ForEach-Object {
    Select-String -Path $_ -Pattern 'api_key|secret|password|token|private_key' -SimpleMatch
}
```

### 8.3 核心算法泄露检测（强制）

本项目采用双许可证策略：
- **Apache 2.0**（`LICENSE_CODE`）：覆盖公开层源码
- **DaoTi Research License v1.0**（`LICENSE`）：保护核心算法文件

#### 受保护的核心算法文件

以下文件受 DaoTi Research License 保护，**禁止在公开层文件中泄露其算法内容**：

| 文件 | 保护内容 |
|------|---------|
| `src/engine/encoder.rs` | 语义编码算法 |
| `src/engine/retriever.rs` | 向量检索算法 |
| `src/engine/manager.rs` | 编排引擎 |
| `src/engine/encoder_codebert.rs` | 外部编码器适配 |
| `src/engine/mod.rs` | 模块入口 |

#### 公开层文件（必须通过泄露检测）

| 文件 | 许可证 |
|------|--------|
| `src/chunker.rs` | Apache 2.0 |
| `src/server.rs` | Apache 2.0 |
| `src/bin/server.rs` | Apache 2.0 |
| `src/lib.rs` | Apache 2.0 |

#### 泄露检测规则

`scripts/check_algorithm_leak.py` 检测以下内容（error 级别，必须为 0）：

- **道枢映射**：道枢、道体、道同构、Dao pivot/ti/isomorphism、dao_evolution
- **八卦编码**：乾卦、坤卦、震卦、巽卦、坎卦、离卦、艮卦、兑卦、八卦、Bagua、trigram
- **几何坐标空间**：几何坐标、geometric coordinate、memory topology、拓扑演化
- **洛书算法**：luoshu、洛书、mirror trapezoid、镜像梯形
- **剪枝算法**：ROI prun、剪枝、可逆组合、reversible composit
- **底层架构变造**：底层架构、underlying architecture、gauge field、规范场、退化基态

#### 合规引用（白名单，不算泄露）

以下引用是合规的，检测脚本会自动跳过：

1. **资源文件名引用**：`include_str!("../static/assets/icons/icon-luoshu.svg")`
2. **资源文件名匹配**：`"icon-bagua.svg"`、`"icon-luoshu.svg"`
3. **engine 模块文件名引用**：`luoshu_encoder_ml.rs`
4. **环境变量名**：`LRC_LUOSHU_MODEL_ID`
5. **UI 设计风格命名**：`洛书九宫格加载动画`
6. **结构体字段名**：`.bagua_entropy`、`.dao_pivot`
7. **模块重导出**：`pub use engine::luoshu_encoder`

#### 提交前强制检测

```powershell
# 提交前必须运行算法泄露检测，0 错误才允许推送
python scripts/check_algorithm_leak.py --verbose

# 期望输出：
#   通过: 公开层文件无核心算法泄露
#   退出码: 0
```

> **重要**：Pre-commit hook 已包含算法泄露检测。如检测失败，提交将被拒绝。
> 如检测到真正的算法泄露，必须将相关内容移至 `src/engine/` 目录下的受保护文件中。

### 8.4 代码签名

- macOS：使用 Ad-Hoc 签名（`codesign --sign - --force`）
- Windows：当前未签名，用户首次运行需在 SmartScreen 警告中选择"仍要运行"
- Linux：deb 包通过包管理器安装，无需签名

---

## 九、Git 操作红线

### 9.1 禁止操作

- **禁止** `git push --force` 到 main/master 分支
- **禁止** `git reset --hard` 已推送的提交
- **禁止** `git add .` 或 `git add -A`（可能添加敏感文件）
- **禁止**提交 `.env`、密钥、证书文件
- **禁止**在提交信息中暴露敏感信息
- **禁止**在 `feat` 提交上打版本 tag（tag 必须指向 `chore(release)` 提交，详见第十章）

### 9.2 推荐操作

- 使用 `git add <具体文件>` 明确添加文件
- 使用 `git stash` 临时保存未完成的工作
- 使用分支开发：`git checkout -b feature/xxx`
- 使用 Pull Request 进行代码审查

---

## 十、旧版本清理规范

> **强制要求**：每次版本发布前，必须执行旧版本清理，防止旧文件残留导致版本混淆（v0.6.0 曾因旧前端残留导致编译出 0.5.12 版本）。

### 10.1 清理范围

| 类别 | 清理对象 | 判定标准 |
|------|---------|---------|
| 旧前端文件 | `desktop/src/` 目录 | 已被 `static/` 取代的配置向导、旧样式表、旧脚本 |
| 旧构建脚本 | `build_release.ps1`、`sign-binary.ps1` | 含过期 token、硬编码旧版本号的脚本 |
| 旧测试报告 | `desktop/TEST_REPORT_v*.md` | 历史版本测试报告，不影响当前版本 |
| 旧安装包 | `*.msi`、`*setup*.exe`、`*.dmg`、`MicrosoftEdgeWebview2Setup.exe` | 旧版本安装包，CI/CD 会重新构建 |
| 空残留文件 | `build-log.txt`（0 字节）| 空文件残留 |
| 旧编译缓存 | `target/`、`desktop/src-tauri/target/` | 本地编译缓存（.gitignore 已忽略） |
| 旧版本 sidecar | `desktop/src-tauri/lrc-sidecar.exe`、`G:\rust-target\release\lrc-sidecar.exe` | 旧版本 sidecar 二进制（v0.7.x debug 构建约 17MB，release 约 4MB） |
| 旧版本桌面端 | `G:\rust-target\release\lrc-desktop.exe` | 旧版本桌面端二进制 |
| 旧版本清理脚本 | `scripts/cleanup_old_builds_v*.ps1` | 历史版本清理脚本（保留最新版本） |

### 10.2 清理流程

```powershell
# 1. 检查旧版本残留文件
Get-ChildItem -Path . -Filter "TEST_REPORT_v*" -Recurse
Get-ChildItem -Path . -Filter "build_release.ps1" -Recurse
Get-ChildItem -Path . -Filter "*.msi" -Recurse

# 2. 删除旧前端（已被 static/ 取代）
git rm desktop/src/index.html desktop/src/styles.css desktop/src/wizard.js 2>$null

# 3. 删除旧脚本（含过期 token）
Remove-Item build_release.ps1 -Force -ErrorAction SilentlyContinue

# 4. 删除旧测试报告
Remove-Item desktop/TEST_REPORT_v*.md -Force -ErrorAction SilentlyContinue

# 5. 删除旧安装包
Remove-Item MicrosoftEdgeWebview2Setup.exe -Force -ErrorAction SilentlyContinue

# 6. 提交清理
git commit -m "chore: 清理旧版本残留文件"
```

### 10.3 清理验证

清理完成后，验证以下事项：

1. `desktop/src/` 目录不存在或为空（前端统一在 `static/`）
2. `tauri.conf.json` 的 `frontendDist` 指向 `../../static`
3. 无含过期 token 的脚本文件
4. 无旧版本测试报告
5. `git status` 干净，无非预期的旧文件

---

## 十一、Tag 构建规范

> **强制要求**：版本 tag 必须指向最新的 `chore(release)` 提交，不能指向 `feat` 或 `fix` 提交。

### 11.1 Tag 指向规则

| 提交类型 | 是否可以打 tag | 原因 |
|---------|--------------|------|
| `chore(release): 发布 vX.Y.Z` | ✅ **必须** | 包含所有前端+后端+配置的完整更新 |
| `feat: 新功能` | ❌ **禁止** | 可能不包含前端更新或配置修复 |
| `fix: Bug 修复` | ❌ **禁止** | 可能不包含完整的版本发布内容 |

### 11.2 Tag 创建流程

```powershell
# 1. 确认所有修改已提交（包括前端、后端、配置、文档）
git status
# 期望：nothing to commit, working tree clean

# 2. 确认 HEAD 是 chore(release) 提交
git log --oneline -1
# 期望：chore(release): 发布 vX.Y.Z

# 3. 确认版本号一致性（见 4.1 节）

# 4. 确认旧版本已清理（见第十章）

# 5. 创建 tag（在 chore(release) 提交上）
git tag -a vX.Y.Z -m "vX.Y.Z: 版本说明"

# 6. 验证 tag 指向
git rev-list -n 1 vX.Y.Z
# 必须等于 git rev-parse HEAD

# 7. 推送 tag（触发 CI/CD）
git push origin vX.Y.Z
```

### 11.3 Tag 修复流程（当 tag 指向错误提交时）

```powershell
# 1. 删除本地旧 tag
git tag -d vX.Y.Z

# 2. 在正确的提交上创建新 tag
git tag -a vX.Y.Z -m "vX.Y.Z: 版本说明"

# 3. 强制推送 tag（覆盖远程旧 tag）
git push origin vX.Y.Z --force

# 4. 验证远程 tag 指向
git ls-remote --tags origin vX.Y.Z
```

> **注意**：强制推送 tag 后，需要手动删除旧的 GitHub Release（如果已创建），或让 CI/CD 自动更新 Release（`softprops/action-gh-release` 会更新已存在的 Release）。

### 11.4 二进制编译包处理

| 场景 | 处理方式 | 说明 |
|------|---------|------|
| 仓库代码 | **禁止**推送二进制 | `.gitignore` 已忽略 `*.exe`、`*.msi`、`*.dmg`、`target/` |
| CI/CD 构建 | 自动编译 | `release.yml` 步骤 1 编译 sidecar，步骤 2 复制到 `desktop/src-tauri/`，步骤 4 构建桌面端 |
| GitHub Release | 自动上传 | CI/CD 编译后自动上传到 GitHub Release，用户从 Release 下载 |
| 本地构建 | 手动编译 | `cargo build --release --features server` 编译 sidecar，复制到 `desktop/src-tauri/lrc-sidecar.exe` |

> **重要**：仓库中**不需要**也不**应该**包含预编译的二进制文件。CI/CD 会从源码编译所有产物。`build.rs` 会检测 `desktop/src-tauri/lrc-sidecar.exe` 是否存在，本地构建时需手动编译并复制。

---

## 十二、附录

### 12.1 当前仓库状态（v0.8.7）

- **仓库地址**：https://github.com/zhibaiYingChuan/LRC
- **主分支**：main
- **当前版本**：0.8.7
- **构建工作流**：`.github/workflows/release.yml`
- **覆盖平台**：Windows、macOS、Linux

### 12.2 相关文档

- [CHANGELOG.md](../CHANGELOG.md) — 变更记录
- [README.md](../README.md) — 项目说明
- [docs/USER_GUIDE.md](USER_GUIDE.md) — 用户指南
- [.github/workflows/release.yml](../.github/workflows/release.yml) — 构建工作流
- [.gitignore](../.gitignore) — Git 忽略规则

### 12.3 修订记录

| 日期 | 版本 | 修订内容 |
|------|------|---------|
| 2026-07-28 | v1.0 | 初始版本，基于 v0.6.0 迭代经验形成 |
| 2026-07-28 | v1.1 | 新增第十章旧版本清理规范、第十一章 Tag 构建规范；删除 desktop/src/ 相关要求；明确二进制编译包处理方式 |
| 2026-07-30 | v1.2 | HCSE 安全评估 + 发布规范专家评审后更新：第 4.1 节版本号同步清单 6→7 处（新增 static/app.js APP_VERSION）；第 4.2 节当前版本 0.6.0→0.8.7；第 10.1 节清理范围补充 v0.8.x 编译产物；第 12.1 节仓库状态更新到 v0.8.7 |

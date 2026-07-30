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

# 6. 代码格式检查（与 CI Rustfmt job 对齐，v0.8.8 新增）
cargo fmt --all -- --check
# 期望结果：无输出（退出码 0）

# 7. Clippy 静态检查（与 CI Clippy job 对齐，v0.8.8 新增）
cargo clippy --features server -- -D warnings
cargo clippy --all-targets -- -D warnings
# 期望结果：无 warning，无 error

# 8. 编译验证（主项目）
cargo build --release --features server

# 9. 编译验证（桌面端，发布前才需要）
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
# 5. README.md 审查（7 项，见 7.4 节，0 错误才允许推送）
#    - 链接有效性：无死链、无 file:// 链接
#    - 徽章准确性：Rust 版本徽章 = Cargo.toml rust-version
#    - 性能数据出处：所有性能数据有测试报告支撑
#    - 版本一致性：徽章/数据版本/功能标记与当前版本一致
#    - 未发布版本禁令：无未发布版本功能描述
#    - 实事求是：无虚假数据、无未实现功能描述
#    - 过时内容清理：无"（新）"等过时标记、无版本号小节标题
# 6. 用户文档已同步更新
```

### 2.3 跨平台预检（v0.8.8 新增，防止 macOS/Linux CI 失败）

> **强制要求**：打 tag 前必须执行本节检查。v0.8.7 因跳过此检查导致 macOS/Linux 桌面端 CI 失败。

```powershell
# 1. Tauri 配置验证（确保 targets 不是单一平台）
# 错误示例："targets": ["nsis"]  ← 仅 Windows，macOS/Linux 无 bundle
# 正确示例："targets": "all"     ← 三平台全量打包
Select-String -Path desktop/src-tauri/tauri.conf.json -Pattern '"targets"'

# 2. MSRV 一致性检查（主项目与桌面端必须一致）
$mainMsrv = (Select-String -Path Cargo.toml -Pattern '^rust-version').Line
$desktopMsrv = (Select-String -Path desktop/src-tauri/Cargo.toml -Pattern '^rust-version').Line
Write-Host "主项目 MSRV: $mainMsrv"
Write-Host "桌面端 MSRV: $desktopMsrv"
# 期望：两者版本号一致（当前均为 1.80）

# 3. 桌面端编译验证（本地 Windows 可验证编译通过）
cd desktop/src-tauri; cargo check; cd ../..

# 4. CI 工作流矩阵验证（确认三平台均覆盖）
Select-String -Path .github/workflows/release.yml -Pattern 'matrix:'
# 期望：build-sidecar 和 build-desktop 均包含 windows/macos/linux 三平台
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

### 5.6 CI 预检门禁（v0.8.8 已实现）

> **已落地**：`release.yml` 已实现 preflight job（v0.8.8），在三平台构建前执行 7 项检查，任一失败则阻止构建。
> **强制要求**：`release.yml` 在触发三平台构建前，必须先跑 preflight job 确保基础编译通过。v0.8.7 因无 preflight 导致三平台并行失败浪费时间。

#### Preflight Job 已实现内容（release.yml Job 0）

```yaml
# release.yml 新增 preflight job（在 build-sidecar 和 build-desktop 之前）
preflight:
  name: Preflight Check
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v5
    - uses: dtolnay/rust-toolchain@stable
    - name: Format check
      run: cargo fmt --all -- --check
    - name: Clippy check
      run: cargo clippy --features server -- -D warnings
    - name: Compile check
      run: cargo check --features server
    - name: Desktop config lint
      run: |
        # 验证 tauri.conf.json targets 不是单一平台
        grep -q '"targets": "all"' desktop/src-tauri/tauri.conf.json || \
        grep -E '"targets".*\[.*"app".*"dmg".*\]' desktop/src-tauri/tauri.conf.json
    - name: MSRV consistency check
      run: |
        MAIN_MSRV=$(grep '^rust-version' Cargo.toml | cut -d'"' -f2)
        DESKTOP_MSRV=$(grep '^rust-version' desktop/src-tauri/Cargo.toml | cut -d'"' -f2)
        echo "Main MSRV: $MAIN_MSRV, Desktop MSRV: $DESKTOP_MSRV"
        [ "$MAIN_MSRV" = "$DESKTOP_MSRV" ] || { echo "ERROR: MSRV 不一致"; exit 1; }

build-sidecar:
  needs: preflight    # 必须等 preflight 通过
build-desktop:
  needs: preflight    # 必须等 preflight 通过
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

### 7.4 README.md 审查规范（v0.8.8 新增）

> **强制要求**：每次版本发布前必须对 README.md 执行以下 7 项审查，0 错误才允许推送。
> 基于 v0.8.8 HCSE 评估新增，防止死链、虚假数据、过时内容流入用户文档。

#### 7.4.1 链接有效性验证（P0）

README.md 中所有文档链接必须满足：

1. 相对路径链接指向的文件必须已被 git 跟踪（`git ls-files <path>` 返回非空）
2. **禁止**链接到 `.gitignore` 忽略的文件（如 `docs/LRC*全案*.md`、`docs/PRD_*.md`）
3. **禁止**使用 `file:///` 本地绝对路径，必须使用相对路径
4. 外部 URL 链接必须可访问

```powershell
# 检测 file:// 本地路径链接（必须为空）
Select-String -Path README.md -Pattern 'file:///'
# 期望结果：无匹配

# 验证相对路径链接的文件已被 git 跟踪
Select-String -Path README.md -Pattern '\]\(([^h)][^)]+)\)' -AllMatches |
  ForEach-Object { $_.Matches.Groups[1].Value } |
  Where-Object { $_ -notmatch '^http' } |
  ForEach-Object {
    $f = ($_ -replace '#.*$','' -replace '%20',' ')
    if (-not (git ls-files --error-unmatch $f 2>$null)) {
      Write-Host "死链: $f" -ForegroundColor Red
    }
  }
# 期望结果：无输出
```

#### 7.4.2 徽章准确性验证（P1）

README.md 中所有徽章必须与项目实际配置一致：

| 徽章 | 验证来源 | 规则 |
|------|---------|------|
| Rust 版本 | `Cargo.toml` 的 `rust-version` | 徽章版本号 = Cargo.toml rust-version |
| License | `LICENSE_CODE` / `LICENSE` 文件 | 文件必须存在 |

```powershell
# 验证 Rust 徽章版本与 Cargo.toml 一致
$badgeVersion = (Select-String -Path README.md -Pattern 'Rust-(\d+\.\d+)').Matches.Groups[1].Value
$cargoMsrv = (Select-String -Path Cargo.toml -Pattern '^rust-version.*"(\d+\.\d+)"').Matches.Groups[1].Value
if ($badgeVersion -ne $cargoMsrv) {
  Write-Host "徽章版本($badgeVersion) != MSRV($cargoMsrv)" -ForegroundColor Red
}
# 期望结果：无输出
```

#### 7.4.3 性能数据出处验证（P0）

README.md 中所有性能数据必须满足：

1. **必须有测试报告**：`benchmarks/reports/` 目录下存在对应报告
2. **数据规模必须匹配**：README 声称的规模必须在测试报告中实际测试过
3. **禁止逻辑矛盾**：大规模延迟不得反常低于小规模
4. **禁止无测试支撑的规模声明**（如"百万条"必须有百万级测试报告）

#### 7.4.4 版本一致性验证（P1）

1. **徽章版本号**：与 `Cargo.toml` 的 `rust-version` 一致
2. **基准测试数据版本**：引用 `benchmarks/reports/` 数据时必须标注来源版本
3. **禁止时效性标记**：已发布多个版本的功能不应标"（新）"
4. **禁止版本号小节标题**：如"### v0.6.0 xxx"应改为"### xxx"，版本信息移至 CHANGELOG

#### 7.4.5 未发布版本功能禁令（P1）

1. **禁止**描述未发布版本的功能（当前 v0.8.8，则禁止描述 v0.9.0 功能）
2. **禁止**对已发布版本使用"预览"标记
3. 未实现功能移至 `docs/PRODUCT_ROADMAP_v1.0.md`

#### 7.4.6 README 审查检查清单（并入 2.2 节第 5 项）

发布前检查清单第 5 项扩展为：

```powershell
# 5. README.md 审查（7 项，0 错误才允许推送）
#   5.1 链接有效性：无死链、无 file:// 链接（见 7.4.1）
#   5.2 徽章准确性：Rust 版本徽章 = Cargo.toml rust-version（见 7.4.2）
#   5.3 性能数据出处：所有性能数据有测试报告支撑（见 7.4.3）
#   5.4 版本一致性：徽章/数据版本/功能标记与当前版本一致（见 7.4.4）
#   5.5 未发布版本禁令：无未发布版本功能描述（见 7.4.5）
#   5.6 实事求是：无虚假数据、无未实现功能描述（见 7.1）
#   5.7 过时内容清理：无"（新）"等过时标记、无版本号小节标题（见 7.4.4）
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

### 11.5 CI 失败后的 Tag 处置规则（v0.8.8 新增）

> **强制要求**：Tag 触发 CI 失败时，禁止立即删除 Tag 重打。应按以下决策树处理。

#### CI 失败处置决策树

```
Tag 触发 CI 失败
    │
    ├─ 仅 1-2 个平台失败（如 macOS/Linux 桌面端）？
    │   ├─ 是 → 保留 Tag，发布 PATCH 版本修复（推荐）
    │   │        原因：保留 HCSE 可追溯性基线，sidecar + 成功平台产物仍可用
    │   │        操作：修复 → chore(release): 发布 vX.Y.Z+1 → 打新 Tag
    │   │
    │   └─ 否（全部平台失败 / sidecar 编译失败）
    │       → 删除 Tag，修复后重新打 Tag
    │         操作：git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z
    │         修复 → 重新 chore(release) → 重新打 Tag
    │
    └─ 是否已创建 GitHub Release？
        ├─ 是且全平台失败 → 删除 Release + 删除 Tag
        └─ 是且部分平台失败 → 保留 Release，发布 PATCH 版本
```

#### v0.8.7 案例应用

- v0.8.7 CI 失败：仅 macOS/Linux 桌面端失败，sidecar + Windows 桌面端成功
- 处置决策：**保留 v0.8.7 Tag**，发布 v0.8.8 PATCH 修复版
- 修复内容：tauri.conf.json targets 改 "all" + MSRV 统一 1.80 + clippy 修复

---

## 十二、MSRV 一致性规范（v0.8.8 新增）

> **强制要求**：主项目与桌面端的 `rust-version` 必须一致，防止跨平台编译失败。

### 12.1 MSRV 统一规则

| 项目 | 文件 | 当前 MSRV | 依据 |
|------|------|----------|------|
| 主项目 | `Cargo.toml` | 1.80 | `std::sync::LazyLock` 需要 1.80+ |
| 桌面端 | `desktop/src-tauri/Cargo.toml` | 1.80 | 与主项目一致（Tauri 2.x 要求 1.77.2+，取更高值） |

### 12.2 MSRV 提升流程

当引入新依赖或使用新 std 特性时：

1. 确认所需最低 Rust 版本（如 `LazyLock` → 1.80）
2. **同步更新** `Cargo.toml` 和 `desktop/src-tauri/Cargo.toml` 的 `rust-version`
3. 在 CI `preflight` job 中增加 MSRV 一致性校验（见 5.6 节）
4. 更新本节表格

### 12.3 禁止事项

- **禁止**主项目与桌面端 MSRV 不一致
- **禁止**降低 MSRV 而不验证依赖兼容性
- **禁止**使用 nightly-only 特性（CI 使用 stable 工具链）

---

## 十三、CI 失败处理与防复发（v0.8.8 新增）

### 13.1 CI 失败分类

| 失败类型 | 根因模式 | 防复发措施 |
|---------|---------|-----------|
| 编译错误（全平台） | MSRV 不兼容 / 语法错误 | preflight job 编译检查 |
| 编译错误（单平台） | 平台特定代码 / 配置缺失 | 跨平台预检（2.3 节） |
| 格式化失败 | cargo fmt 未执行 | preflight job fmt 检查 |
| Clippy 失败 | lint 未修复 | preflight job clippy 检查 |
| Bundle 缺失 | tauri.conf.json targets 配置错误 | preflight job config lint |
| 系统依赖缺失 | Linux webkit2gtk 等未安装 | release.yml 已含 apt-get install |

### 13.2 v0.8.7 失败复盘（防复发档案）

| 项目 | 详情 |
|------|------|
| 失败现象 | macOS/Linux 桌面端 CI exit 1，Windows 成功 |
| 根本原因 | `tauri.conf.json` 的 `"targets": ["nsis"]` 仅产出 Windows NSIS 包，macOS/Linux 无 bundle 产物，`find ... -exec cp` 找不到文件 exit 1 |
| 次要原因 | 主项目 MSRV 1.70 与 LazyLock（1.80+）不兼容；Clippy question_mark lint 未修复 |
| 修复方案 | targets 改 "all" + MSRV 统一 1.80 + clippy --fix + cargo fmt |
| 防复发规则 | 新增 2.3 跨平台预检 + 5.6 CI preflight 门禁 + 第十二章 MSRV 规范 |

### 13.3 CI 失败应急流程

```powershell
# 1. 查看 CI 失败详情（浏览器打开）
# https://github.com/zhibaiYingChuan/LRC/actions

# 2. 本地复现（按失败平台分类处理）
#    - 格式/Clippy 错误：cargo fmt && cargo clippy --fix
#    - 编译错误：cargo check --features server
#    - 配置错误：检查 tauri.conf.json targets

# 3. 修复后本地验证（执行 2.1 + 2.2 + 2.3 全部检查）

# 4. 按 11.5 节决策树决定是否删除 Tag

# 5. 提交修复并推送
git add <具体文件>
git commit -m "fix(ci): 修复跨平台编译失败"
git push origin main

# 6. 重新打 Tag（如需）
git tag -a vX.Y.Z -m "vX.Y.Z: 修复跨平台编译"
git push origin vX.Y.Z
```

---

## 十四、发布前全面审计规范（v0.8.8 新增）

> **强制要求**：任何版本发布前，必须对**所有面向用户的内容**执行全面审计，禁止仅审计单一领域。
> 本规范基于 v0.8.8 复盘：前三次专家评估因"隧道视野"（每次只审一个领域），导致 README 死链、虚假性能数据等问题在发布后才被发现。

### 14.1 审计范围（8 大领域，缺一不可）

| 序号 | 审计领域 | 审计对象 | 对应规则 | P0 条件 |
|:---:|---------|---------|---------|---------|
| 1 | **代码编译** | 主项目 + 桌面端编译通过 | 2.1 节第 8-9 项 | 编译失败 |
| 2 | **代码质量** | cargo fmt + clippy + 算法泄露 | 2.1 节第 5-7 项 | 泄露/格式/lint 错误 |
| 3 | **跨平台配置** | tauri.conf.json + MSRV 一致性 | 2.3 节 + 第十二章 | macOS/Linux 构建失败 |
| 4 | **README.md** | 链接/徽章/性能数据/版本/功能描述 | 7.4 节（7 项） | 死链/虚假数据 |
| 5 | **版本号一致性** | 7 处版本号同步 | 4.1 节 | 任意一处不一致 |
| 6 | **CHANGELOG.md** | 最新版本已记录 | 7.3 节 | 缺少当前版本记录 |
| 7 | **用户文档** | USER_GUIDE.md 等已同步 | 7.2 节 | 功能描述与代码不符 |
| 8 | **CI/CD 配置** | release.yml + ci.yml 预检门禁 | 5.6 节 + 第十一章 | 无 preflight job |

### 14.2 审计流程

```powershell
# 步骤 1：执行全面审计（按 8 大领域逐一检查）
# 不允许跳过任何领域，不允许只审计"出问题的领域"

# 1. 代码编译
cargo check --features server
cd desktop/src-tauri; cargo check; cd ../..

# 2. 代码质量
cargo fmt --all -- --check
cargo clippy --features server -- -D warnings
python scripts/check_algorithm_leak.py --verbose

# 3. 跨平台配置（见 2.3 节）
# 4. README.md 审查（见 7.4 节）
# 5. 版本号一致性（见 4.1 节）
# 6. CHANGELOG.md 已更新
# 7. 用户文档已同步
# 8. CI/CD 配置（见 5.6 节）

# 步骤 2：审计报告
# 记录 8 大领域的审计结果，0 个 P0 才允许进入发布流程
```

### 14.3 "隧道视野"防范规则

> **核心教训**：v0.8.7 发布前，专家评估了 PUSH_STANDARD.md 规则（领域 5/6），但未评估 README（领域 4），导致死链和虚假数据流入用户文档。

1. **禁止局部审计替代全面审计**：即使只需修复一个领域的问题，也必须确认其他 7 个领域无 P0 问题
2. **专家调用必须指定全面范围**：调用高可信发布规范专家时，必须要求"对所有面向用户的内容做全面审计"，而非仅评估单一领域
3. **审计报告必须覆盖 8 大领域**：即使某领域"无变化"，也必须明确标注"已检查，无问题"
4. **用户提出的特定问题只是入口**：修复特定问题后，必须扩展到全面审计，不能只修完就推送

### 14.4 审计检查清单模板

```markdown
## 发布前全面审计报告 v0.X.Y

| 领域 | 状态 | P0 | P1 | 备注 |
|------|------|:--:|:--:|------|
| 1. 代码编译 | ✅/❌ | 0 | 0 | cargo check 通过 |
| 2. 代码质量 | ✅/❌ | 0 | 0 | fmt/clippy/泄露检测通过 |
| 3. 跨平台配置 | ✅/❌ | 0 | 0 | tauri.conf.json targets=all, MSRV 一致 |
| 4. README.md | ✅/❌ | 0 | 0 | 无死链/虚假数据/过时标记 |
| 5. 版本号一致性 | ✅/❌ | 0 | 0 | 7 处版本号一致 |
| 6. CHANGELOG.md | ✅/❌ | 0 | 0 | 当前版本已记录 |
| 7. 用户文档 | ✅/❌ | 0 | 0 | 功能描述与代码一致 |
| 8. CI/CD 配置 | ✅/❌ | 0 | 0 | preflight 门禁就绪 |

**结论**：[ ] 全部通过，允许发布 / [ ] 存在 P0，禁止发布
```

---

## 十五、附录

### 15.1 当前仓库状态（v0.8.8）

- **仓库地址**：https://github.com/zhibaiYingChuan/LRC
- **主分支**：main
- **当前版本**：0.8.8
- **构建工作流**：`.github/workflows/release.yml`
- **覆盖平台**：Windows、macOS、Linux

### 14.2 相关文档

- [CHANGELOG.md](../CHANGELOG.md) — 变更记录
- [README.md](../README.md) — 项目说明
- [docs/USER_GUIDE.md](USER_GUIDE.md) — 用户指南
- [.github/workflows/release.yml](../.github/workflows/release.yml) — 构建工作流
- [.gitignore](../.gitignore) — Git 忽略规则

### 15.3 修订记录

| 日期 | 版本 | 修订内容 |
|------|------|---------|
| 2026-07-28 | v1.0 | 初始版本，基于 v0.6.0 迭代经验形成 |
| 2026-07-28 | v1.1 | 新增第十章旧版本清理规范、第十一章 Tag 构建规范；删除 desktop/src/ 相关要求；明确二进制编译包处理方式 |
| 2026-07-30 | v1.2 | HCSE 安全评估 + 发布规范专家评审后更新：第 4.1 节版本号同步清单 6→7 处（新增 static/app.js APP_VERSION）；第 4.2 节当前版本 0.6.0→0.8.7；第 10.1 节清理范围补充 v0.8.x 编译产物；第 12.1 节仓库状态更新到 v0.8.7 |
| 2026-07-30 | v1.3 | 新增第 2.1 节 cargo fmt + clippy 预检规则（与 CI 对齐） |
| 2026-07-30 | v1.4 | v0.8.7 CI 失败复盘后新增：第 2.3 节跨平台预检、第 5.6 节 CI preflight 门禁、第 11.5 节 CI 失败 Tag 处置决策树、第十二章 MSRV 一致性规范、第十三章 CI 失败处理与防复发；桌面端 MSRV 1.77→1.80 统一 |
| 2026-07-30 | v1.5 | HCSE README 审核后新增：第 7.4 节 README.md 审查规范（7 项审查）；2.2 节检查清单第 5 项扩展为 7 项；修复 README 死链、虚假性能数据、file:// 链接、未发布版本功能描述、过时标记 |
| 2026-07-30 | v1.6 | 新增第十四章发布前全面审计规范（8 大领域审计范围 + 隧道视野防范规则 + 审计报告模板）；根因分析：前三次专家评估因"隧道视野"导致 README 问题遗漏 |
| 2026-07-30 | v1.7 | 工作流程标准化改造：pre-commit hook v2.0（新增 fmt+clippy+PATH 修复）；release.yml preflight job 已实现（7 项检查）；ci.yml build-matrix 新增桌面端 cargo check + tauri 配置校验；新增 scripts/preflight_check.ps1 一键 8 大领域审计脚本；新增第十六章标准化工作流程门禁链 |

---

## 十六、标准化工作流程门禁链（v0.8.8 工具化）

> **核心改进**：将文档规范升级为自动化工具链，构建三层门禁防线，确保问题在最早阶段被发现和拦截。
> **根因分析**：v0.8.7 三平台两失败的根因不是规范缺失（PUSH_STANDARD.md 已有 977 行规范），而是**规范与工具脱节**——规范要求了 fmt/clippy/跨平台预检，但 pre-commit hook 和 release.yml 都没有实现这些检查。

### 16.1 三层门禁架构

```
开发提交 → [门禁 1] pre-commit hook（本地，5 项检查）
               ↓ 通过
           git push main → [门禁 2] CI（ci.yml，5 个 job）
               ↓ 全绿
           git tag vX.Y.Z → [门禁 3] Release preflight（release.yml Job 0，7 项检查）
               ↓ 全绿
           build-sidecar + build-desktop（三平台并行构建）
               ↓ 全部成功
           GitHub Release（自动发布）
```

### 16.2 门禁 1：pre-commit hook v2.0（本地防线）

> **已实现**：`.git/hooks/pre-commit`（v2.0，2026-07-30）

| 检查项 | 工具 | 失败动作 | 对应 CI job |
|--------|------|---------|------------|
| 代码格式 | `cargo fmt --all -- --check` | 阻止提交 | Rustfmt |
| Clippy（server） | `cargo clippy --features server -- -D warnings` | 阻止提交 | Clippy |
| Clippy（all-targets） | `cargo clippy --all-targets -- -D warnings` | 阻止提交 | Clippy |
| 编译检查 | `cargo check --features server` | 阻止提交 | Build Check |
| 单元测试 | `cargo test` | 阻止提交 | Unit & Integration Tests |
| 算法泄露检测 | `python scripts/check_algorithm_leak.py` | 阻止提交 | — |

**v2.0 变更**：
- 新增 `cargo fmt --check`（v0.8.7 遗漏，CI fmt job 失败）
- 新增 `cargo clippy -D warnings`（v0.8.7 遗漏，CI Clippy job 失败）
- 修复 Windows Git Bash cargo PATH 问题（`export PATH="$HOME/.cargo/bin:$PATH"`）

### 16.3 门禁 2：CI（ci.yml，push 到 main 触发）

> **已增强**：`.github/workflows/ci.yml`（v0.8.8）

| Job | 检查内容 | v0.8.8 新增 |
|-----|---------|------------|
| Rustfmt | `cargo fmt --all -- --check` | — |
| Clippy | `cargo clippy --features server -- -D warnings` + `--all-targets` | — |
| Unit & Integration Tests | `cargo test --features server` | — |
| E2E Smoke Test | 启动 sidecar，curl `/dashboard`（v0.8.8 修复 `/`→302 问题） | ✓ 端点修复 |
| Build Check（三平台） | `cargo check --features server` + **桌面端 `cargo check`** | ✓ 桌面端检查 |
| — | **Tauri 配置校验**（targets 禁止单平台） | ✓ 新增 |

### 16.4 门禁 3：Release Preflight（release.yml Job 0，tag 触发）

> **已实现**：`.github/workflows/release.yml` preflight job（v0.8.8）

| 检查项 | 检查内容 | 防止的故障 |
|--------|---------|-----------|
| Format check | `cargo fmt --all -- --check` | CI Rustfmt job 失败 |
| Clippy check (server) | `cargo clippy --features server -- -D warnings` | CI Clippy job 失败 |
| Clippy check (all-targets) | `cargo clippy --all-targets -- -D warnings` | CI Clippy job 失败 |
| Compile check | `cargo check --features server` | 编译失败浪费三平台构建 |
| Unit tests | `cargo test --features server` | 测试失败 |
| Tauri config lint | targets 禁止 `["nsis"]` 单平台 | macOS/Linux 无 bundle（v0.8.7 根因） |
| MSRV consistency | 主项目 = 桌面端 | MSRV 不一致导致编译失败 |
| Version consistency | Cargo.toml = desktop Cargo.toml = tauri.conf.json | 版本号不一致 |

**关键设计**：`build-sidecar` 和 `build-desktop` 均设置 `needs: preflight`，preflight 失败则不进入构建阶段。

### 16.5 一键验证脚本：scripts/preflight_check.ps1

> **已创建**：`scripts/preflight_check.ps1`（v0.8.8）

一键执行 PUSH_STANDARD.md 第十四章 8 大领域审计：

```powershell
# 发布前一键验证（8 大领域全覆盖）
powershell -File scripts/preflight_check.ps1
```

| 领域 | 自动化检查项 | 退出码 |
|------|------------|--------|
| 1. 代码编译 | 主项目 + 桌面端 cargo check | 0=通过, 1=失败 |
| 2. 代码质量 | fmt + clippy + 泄露检测 | — |
| 3. 跨平台配置 | tauri targets + MSRV 一致性 | — |
| 4. README.md | file:// 链接 + 徽章版本 | — |
| 5. 版本号一致性 | 3 处核心版本号 | — |
| 6. CHANGELOG.md | 当前版本已记录 | — |
| 7. 用户文档 | 人工检查提示 | WARN |
| 8. CI/CD 配置 | preflight job + desktop check + tauri lint | — |

### 16.6 v0.8.7 故障与门禁对照表

| v0.8.7 故障 | 根因 | 门禁 1 拦截 | 门禁 2 拦截 | 门禁 3 拦截 |
|------------|------|:----------:|:----------:|:----------:|
| Clippy exit 101 | unnecessary_sort_by 等 4 个 lint | ✓ v2.0 新增 | ✓ CI Clippy | ✓ Preflight |
| E2E exit 1 | GET / 返回 302 | — | ✓ v0.8.8 修复端点 | — |
| macOS/Linux 无 bundle | tauri targets=["nsis"] | — | ✓ Tauri lint | ✓ Tauri lint |
| MSRV 不一致 | 主项目 1.70 vs 桌面端 1.77 | — | — | ✓ MSRV check |
| Node.js 20 废弃 | download-artifact@v5 | — | — | — (非构建失败) |

> **结论**：三层门禁中任意一层即可拦截 v0.8.7 的全部故障。v0.8.7 之所以失败，是因为三层门禁全部缺失。

### 16.7 CI 步骤自验证规则（v0.8.9 新增）

> **核心教训**：新增 CI 步骤时，不仅要验证"检查什么"，还要验证"步骤本身能否在 CI 环境中执行"。v0.8.8 新增 `cargo check (desktop)` 步骤，但步骤依赖 sidecar 文件存在，CI 环境没有准备，导致三平台全挂。

#### 规则

1. **新增 CI 步骤前，必须列出前置依赖**：步骤是否依赖编译产物、系统库、环境变量等
2. **新增 CI 步骤后，必须在 CI 环境验证步骤本身可执行**：不能只验证"本地能跑"，CI 环境与本地环境不同
3. **CI 步骤的检查项必须在三层门禁中一致**：如果门禁2（CI）检查了桌面端编译，门禁1（pre-commit）和门禁3（release preflight）要么也检查，要么明确标注"仅 CI 检查"及原因

#### CI 步骤前置依赖清单模板

每次新增 CI 步骤时，填写以下清单：

| 步骤名 | 前置依赖 | 准备方式 | 已验证 |
|--------|---------|---------|:------:|
| cargo check (desktop) | lrc-sidecar 文件（tauri.conf.json resources glob） | 创建占位文件 | ✓ v0.8.9 |
| Tauri config lint | 无 | — | ✓ |
| E2E Smoke Test | sidecar 二进制 | cargo build --release | ✓ |

### 16.8 v0.8.9 CI 步骤依赖盲区故障

> **故障**：v0.8.8 新增 `cargo check (desktop)` 步骤，三平台 CI 全部失败（exit 101/1/101）。
> **根因**：tauri.conf.json 的 `resources: ["lrc-sidecar*"]` 要求 sidecar 文件存在，`tauri_build::build()` 在编译时检查 resources glob，CI 环境没有 sidecar 文件 → panic。
> **为什么本地通过**：本地 `desktop/src-tauri/` 有缓存的 `lrc-sidecar.exe`。
> **为什么 Release 通过**：release.yml `build-desktop` 先编译 sidecar 并复制到 `desktop/src-tauri/`。
> **修复**：ci.yml 新增 `Create placeholder sidecar` 步骤，在 `cargo check (desktop)` 前创建占位文件。
> **教训**：新增 CI 步骤时，必须考虑步骤的前置依赖，并在 CI 环境验证步骤本身可执行。

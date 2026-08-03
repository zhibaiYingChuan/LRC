# LRC 发布流程规范

> v0.8.42 建立 — 标准化发布流程，确保每次发布可靠、可追溯。

---

## 一、发布前准备（本地执行）

### 1.1 版本号同步

检查以下 10 处版本号是否一致（参考 `release.yml` 中 `Version consistency check` 步骤）：

| # | 文件 | 检查方式 |
|---|------|---------|
| 1 | `Cargo.toml` | `grep '^version'` |
| 2 | `desktop/src-tauri/Cargo.toml` | `grep '^version'` |
| 3 | `desktop/src-tauri/tauri.conf.json` | `grep '"version"'` |
| 4 | `Cargo.lock` | `cargo check --features server` 自动更新 |
| 5 | `desktop/src-tauri/Cargo.lock` | `cd desktop/src-tauri && cargo check` 自动更新 |
| 6 | `desktop/package.json` | `grep '"version"'` |
| 7 | `static/app.js` | `const APP_VERSION` |
| 8 | `static/index.html` | `meta name="version"` |
| 9 | `CHANGELOG.md` | 顶部 `## [x.y.z]` |
| 10 | `static/index.html` | `id="status-version"` |

命令：
```bash
cargo check --features server                    # 更新根目录 Cargo.lock
cd desktop/src-tauri && cargo check              # 更新 desktop Cargo.lock
```

### 1.2 本地预检

```bash
cargo fmt --all -- --check                        # 代码格式
cargo clippy --features server -- -D warnings     # 静态检查
cargo clippy --all-targets -- -D warnings         # 全目标检查
cargo check --features server                     # 编译检查
cargo test --features server                      # 单元测试
```

### 1.3 泄露检测

提交前确保泄露检测脚本通过（见 `pre-commit` 钩子配置）。

### 1.4 更新 CHANGELOG

格式：
```markdown
## [x.y.z] - YYYY-MM-DD

### 修复：xxx

**问题根因：** ...

**修复：** ...

**涉及文件：**
- `path/to/file.rs`：具体修改说明
```

---

## 二、发布执行

### 2.1 提交与推送

```bash
# 暂存变更文件
git add <file1> <file2> ...

# 提交（遵循规范格式）
git commit -m "chore: 发布 vx.y.z

- 变更说明
- 变更说明"

# 创建 tag
git tag vx.y.z

# 推送
git push
git push origin vx.y.z
```

### 2.2 提交信息规范

- **功能发布**：`chore: 发布 vx.y.z`
- **修复发布**：`fix: 发布 vx.y.z — 问题简述`
- **紧急修复**：`hotfix: 发布 vx.y.z — 问题简述`

---

## 三、推送后监控

### 3.1 CI 状态检查

推送后前往 [Actions 页面](https://github.com/zhibaiYingChuan/LRC/actions) 监控：

1. **CI 工作流**（`ci.yml`）：约 5-10 分钟完成
2. **Release 工作流**（`release.yml`）：约 25-30 分钟完成

### 3.2 已知失败模式及处理

#### 模式 A：Windows DNS 解析失败

**现象：** `Build Sidecar x86_64-pc-windows-msvc` 或 `Build Desktop x86_64-pc-windows-msvc` 的
Checkout 步骤报错 `unable to access '...': Could not resolve host: github.com`。

**处理流程：**
1. ✅ 检查 Linux 和 macOS 构建是否通过（已在 `release.yml` 中设置 `continue-on-error`）
2. ✅ 如果 Linux + macOS 通过，Release 自动创建，Windows 产物后续补充
3. 🔄 手动重试 Windows 构建：进入 Actions → 失败的工作流 → `Re-run jobs` → 选择失败的 Windows job
4. 📝 如果重试后仍失败，在 GitHub Issues 中记录（附 Actions 运行链接）

**自动兜底机制（v0.8.42）：**
1. 系统 DNS 解析 `github.com`（`[System.Net.Dns]::GetHostAddresses`）
2. 备用 DNS 解析（`nslookup github.com 8.8.8.8`）
3. 将解析结果写入 `%windir%\System32\drivers\etc\hosts`，使 `git clone` 绕过系统 DNS

#### 模式 B：Preflight Check 失败

**现象：** `Preflight Check` job 退出码 101。

**处理流程：**
1. 展开失败步骤，查看具体错误
2. 常见原因：
   - **版本号不一致**：某处版本号未同步，对照 1.1 检查
   - **MSRV 不一致**：主项目与 desktop 的 `rust-version` 不同
   - **测试失败**：本地 `cargo test` 应复现，修复后重推
3. 修复后重新推送（需删除旧 tag 重新打，或增加版本号）

#### 模式 C：Security Audit 失败

**现象：** `Security Audit` 工作流 `Cargo Audit` 失败。

**处理流程：**
1. 查看 `cargo audit` 报告，确认是否有新漏洞
2. 如果漏洞在依赖中且无已知修复，记录到 CHANGELOG 的 "Known Issues" 章节
3. 如果漏洞影响严重，升级依赖版本后重新发布

---

## 四、发布后检查清单

- [ ] Release 工作流全部通过（Linux + macOS 必须，Windows 可选）
- [ ] 所有 5 个二进制产物已上传到 Release
  - `lrc-linux-x86_64`
  - `lrc-macos-arm64`
  - `lrc-windows-x86_64.exe`（可能因 DNS 失败缺失）
  - `lrc-desktop-windows-x86_64.msi`（可能因 DNS 失败缺失）
  - `lrc-desktop-macos-arm64.dmg`
  - `lrc-desktop-linux-x86_64.AppImage`
- [ ] 编译产物版本号与 tag 一致
- [ ] CHANGELOG 已更新到当前版本
- [ ] 如果 Windows 构建失败，已记录 Issue

---

## 五、Windows 构建失败后续处理

如果 Windows 构建因 DNS 失败，需要手动处理：

### 5.1 手动重试

1. 打开 GitHub Actions 页面
2. 找到失败的 Release 工作流
3. 点击 `Re-run jobs` → `Re-run failed jobs`
4. 等待 Windows 构建完成

### 5.2 手动上传产物

如果重试后仍失败，可以在本地编译 Windows 产物后手动上传：

```bash
# 本地编译
cargo build --release --features server
# 产物路径：target/release/code-memory-server.exe

# 桌面端
cd desktop/src-tauri
cargo tauri build
# 产物路径：desktop/src-tauri/target/release/bundle/msi/*.msi
```

### 5.3 手动追加到 Release

1. 打开已创建的 Release 页面
2. 点击 `Edit release`
3. 拖拽本地编译的产物到 `Attach binaries` 区域
4. 保存

---

## 六、版本号策略

| 变更类型 | 版本号示例 | 触发条件 |
|---------|-----------|---------|
| 紧急修复 | 0.8.41 → 0.8.42 | CI 修复、小 bug |
| 功能发布 | 0.8.x → 0.9.0 | 新功能、API 变更 |
| 重大发布 | 0.x → 1.0.0 | 稳定版、API 稳定 |

---

## 七、参考

- [GitHub Actions 工作流](https://github.com/zhibaiYingChuan/LRC/actions)
- [Release 工作流配置](../.github/workflows/release.yml)
- [CI 工作流配置](../.github/workflows/ci.yml)
- [Security Audit 配置](../.github/workflows/security.yml)
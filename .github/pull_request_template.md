# LRC 拉取请求模板

## 描述

请简要描述此 PR 的变更内容：

- [ ] 新功能
- [ ] Bug 修复
- [ ] 代码重构
- [ ] 文档更新
- [ ] 依赖更新
- [ ] CI/CD 配置变更

## 变更摘要

请列出主要变更点：

1. 
2. 
3. 

## 自检清单

### 代码质量
- [ ] 代码已通过 `cargo fmt --all -- --check`
- [ ] 代码已通过 `cargo clippy --features server -- -D warnings`
- [ ] 代码已通过 `cargo check --features server`
- [ ] 单元测试已通过 `cargo test --features server`

### 版本号一致性（如适用）
- [ ] 已在所有 9 处位置更新版本号（见 PRE_PUSH_CHECKLIST.md）
- [ ] 已更新 CHANGELOG.md

### 安全性
- [ ] 无敏感文件泄露（.env, credentials.json 等）
- [ ] 无大文件提交（>1MB）
- [ ] 无硬编码密钥

### 桌面端
- [ ] 桌面端编译通过 `cd desktop/src-tauri && cargo check`

## 相关 Issue

Closes #

## 测试说明

请描述如何验证此变更：

1. 
2. 
3.
# 历史文档归档（docs/archive/）

> 建立于 v0.9.11（GLOBAL_CODE_REVIEW_REPORT_v0.9.11 §4.4 **CR-58**：`docs/` 存量文档缺乏归档约定，
> 历史审计与修复计划与现行文档混放，读者无法判断哪份权威）。

## 一、归档判据（两条同时满足才移入本目录）

1. **历史快照**：内容已被后续版本取代（对应版本已发布且 CHANGELOG 有记录）；
2. **零引用**：全仓 Markdown / JS / PowerShell / workflow 中**没有任何链接指向它**
   （判据可复核：`Grep "文件名"` 应 0 命中，除本文件与 CHANGELOG 的历史叙述）。

> ⚠️ **被现行文档引用的历史计划仍留在 `docs/`**（例如 `FIX_PLAN_v0.8.0..v0.8.3.md`、
> `AUDIT_REPORT_v0.7.1.md` —— CHANGELOG 与 `static/app.js` 注释会链到它们）。
> 把有引用的文件也搬走会**制造死链**，得不偿失；故归档只针对"零引用"的历史快照。

## 二、本目录内容（14 份，2026-10 归档）

| 文件 | 说明 |
|---|---|
| `AUDIT_REPORT_v1.0.md` ~ `v3.0.md`、`v5.0.md`、`v6.0.md` | 早期审计报告（v1.0~v6.0 编号体系，后被「版本号对齐」体系取代） |
| `CROSS_PLATFORM_AUDIT_v4.0.md` | 跨平台专项审计（同一编号体系） |
| `FIX_PLAN_v1.0.md`、`FIX_PLAN_v2.0.md` | 早期修复计划（同上编号体系） |
| `FIX_PLAN_v0.6.1_v0.7.0.md` | v0.6.1→v0.7.0 合并修复计划 |
| `FIX_PLAN_v0.8.4.md` ~ `v0.8.7.md` | 已发布版本的历史修复计划（零引用） |
| `audit-report-v0.5.4.md` | 最早的审计报告（命名体系与后续不一致） |

## 三、读者指路（现行权威文档）

| 想了解 | 看这里 |
|---|---|
| 产品用法（面向使用者） | [../USER_GUIDE.md](../USER_GUIDE.md) |
| 项目总览 / 下载 / 边界声明 | [../../README.md](../../README.md) |
| 结构与目录级索引（传统架构文档） | [../ARCHITECTURE.md](../ARCHITECTURE.md) |
| 当前工作区状态的一手核实结论 | [../ARCHITECTURE_OVERVIEW.md](../ARCHITECTURE_OVERVIEW.md) |
| 版本变更历史 | [../../CHANGELOG.md](../../CHANGELOG.md) |
| **当前**的代码审查与修复跟踪 | [../GLOBAL_CODE_REVIEW_REPORT_v0.9.11.md](../GLOBAL_CODE_REVIEW_REPORT_v0.9.11.md) |

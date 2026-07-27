# LRC 产品迭代计划文档

**文档版本**：v1.2
**适用产品版本**：v0.6.0 ~ v0.9.0
**当前基线版本**：v0.6.0（通用语义引擎 + 龙忆设计系统 v1.0 已交付）
**文档状态**：v0.6.0 已实现（含 UI 重构 + v0.7/v0.8/v0.9 预览），待评审后续版本
**产品负责人**：创世者设计
**最后更新**：2026-07-27

---

## 1. 产品愿景与目标

### 1.1 产品定位演进

| 阶段 | 当前定位（v0.5.x） | 目标定位（v0.9.0） |
|------|---------------------|---------------------|
| 一句话 | 给 AI 编程助手装上记忆插件 | 通用个人记忆管家 |
| 核心场景 | IDE 内代码搜索 + 跨会话决策记忆 | 编程 / 笔记 / 项目管理 / 学习助手 全场景记忆 |
| 语义模型 | CodeBERT 强代码弱通用文本 | 通用文本嵌入（BGE/MiniLM）+ 可切换代码模型 |
| 结晶路径 | 强依赖 LLM API（OpenAI） | 本地嵌入优先，LLM API 可选增强 |
| 用户感知 | 黑盒记忆库 | 可视化演化轨迹 + 隐私审计 |

### 1.2 北极星目标

**让任何领域的 AI 助手，在 5 分钟内拥有跨会话、可演化、可信任的永久记忆——无需配置 LLM API 即可开箱可用。**

### 1.3 三层战略目标

1. **可用性目标**：语义搜索在通用文本场景下 Recall@10 ≥ 0.85（对标 v0.5.18 LLM 模式 NQ 0.8016）
2. **通用性目标**：非编程场景记忆占比 ≥ 40%（当前 < 5%）
3. **信任度目标**：用户可一键查看全部数据存储位置与网络访问记录，隐私审计响应 < 100ms

---

## 2. 现状分析

### 2.1 优势盘点（基于现有代码资源）

| 维度 | 现有能力 | 代码位置 |
|------|---------|---------|
| **代码语义搜索** | GraphCodeBERT 默认，代码检索精度比 CodeBERT 高 12.3% | [encoder_codebert.rs](file:///g:/code-memory/src/engine/encoder_codebert.rs) |
| **写入时编码** | 已支持 BGE-small-zh（通过 `LRC_LUOSHU_MODEL_ID` 环境变量），BERT 768/384 维 → 9 维洛书向量 | [luoshu_encoder_ml.rs](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs) |
| **结晶路径** | v0.5.18 LLM embedding 合成已就绪，三阶段锁安全，LLM 失败降级洛书 | [consolidation.rs](file:///g:/code-memory/src/consolidation.rs) |
| **国内镜像** | HF_ENDPOINT 默认 hf-mirror.com，加载策略三层（本地→缓存→远程） | [luoshu_encoder_ml.rs:62-63](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs#L62-63) |
| **模型就绪检测** | `check_model_ready()` 统一接口，支持本地/HF缓存检测 | [model_resolver.rs](file:///g:/code-memory/src/engine/model_resolver.rs) |
| **MCP 工具集** | 12 个工具（remember/recall/forget/search_code 等） | [server.rs](file:///g:/code-memory/src/server.rs) |
| **基准测试** | 6 次标准基准（MS MARCO/NQ/HotpotQA/FiQA 等），TF-IDF MRR@10=0.7749 | [benchmarks/reports](file:///g:/code-memory/benchmarks/reports) |
| **写回性能** | v0.5.6 修复 O(N²)→O(N)，3633 条记忆 recall 写回毫秒级 | [memory_store.rs](file:///g:/code-memory/src/memory_store.rs) |

### 2.2 差距分析

| 差距编号 | 差距描述 | 影响 | 根因 |
|---------|---------|------|------|
| G-01 | **写入时默认模型仍为 MiniLM-L6-v2**，未切到 BGE-small-zh | 中文用户语义精度未达最优 | [luoshu_encoder_ml.rs:67-68](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs#L67-68) 默认值未更新 |
| G-02 | **结晶路径强依赖 LLM API**，本地嵌入模型无法用于结晶 | 离线/无 API Key 用户无法结晶 | [consolidation.rs:412-454](file:///g:/code-memory/src/consolidation.rs#L412-454) 硬编码 `llm_config.embed_texts` |
| G-03 | **代码搜索模型与通用模型割裂** | 用户需分别配置两个模型，认知负担重 | 双编码器架构无统一抽象层 |
| G-04 | **无下载进度提示与重试机制** | 首次下载失败即不可用 | [encoder_codebert.rs](file:///g:/code-memory/src/engine/encoder_codebert.rs) 使用 ureq 同步请求，无进度回调 |
| G-05 | **无非编程场景模板** | 笔记/项目管理等场景需手动设计记忆结构 | 无场景预设配置 |
| G-06 | **记忆演化不可视** | 用户无法感知结晶发生与记忆成长 | 仪表盘未展示结晶历史 |
| G-07 | **无隐私审计入口** | 用户无法验证数据存储与网络访问 | 无统一审计日志查询接口 |

### 2.3 关键技术债

1. **双编码器模型割裂**：`CodeBertEncoder`（代码搜索）与 `LuoShuMlEncoder`（写入编码）使用独立环境变量（`LRC_MODEL_ID` vs `LRC_LUOSHU_MODEL_ID`），用户需理解两套配置语义
2. **结晶路径模型硬编码**：`llm_synthesize_cycle` 直接调用 `LlmApiConfig::embed_texts`，未抽象出 `Embedder` trait，无法切换本地嵌入
3. **投影矩阵维度耦合**：洛书编码器的 `projection: Vec<Vec<f32>>` 与 `hidden_size` 绑定，切换模型需重新生成投影矩阵

---

## 3. 迭代路线图

### 3.1 总览

```
v0.5.18 (当前) ── LLM embedding 合成路径已就绪
      │
      ▼
v0.6.0 (4 周) ── 通用语义引擎
      │           · 默认模型切 BGE-small-zh
      │           · 结晶路径支持本地嵌入
      │           · 统一 Embedder 抽象 + 模型管理器
      │           · 下载进度与重试
      ▼
v0.7.0 (4 周) ── 非编程场景优化
      │           · 预设场景模板
      │           · 场景感知记忆类型
      │           · 完整使用示例
      ▼
v0.8.0 (4 周) ── 记忆演化可视化
      │           · 结晶历史记录
      │           · 仪表盘成长轨迹
      │           · 抽象知识摘要
      ▼
v0.9.0 (4 周) ── 隐私与信任感知
                  · 一键隐私检查
                  · 网络访问审计日志
                  · 数据导出与清除
```

### 3.2 版本里程碑

| 版本 | 发布日期（目标） | 核心主题 | 关键交付物 |
|------|----------------|---------|-----------|
| v0.6.0 | T+4 周 | 通用语义引擎 + 龙忆设计系统 v1.0 | Embedder trait + 模型管理器 + BGE 默认 + 本地结晶 + UI 全面重构 + v0.7/v0.8/v0.9 预览 |
| v0.7.0 | T+8 周 | 场景化记忆 | 4 套场景模板 + 场景感知 API（v0.6.0 已预览 UI） |
| v0.8.0 | T+12 周 | 演化可视化 | 结晶历史持久化 + 仪表盘增强（v0.6.0 已预览时间线 UI） |
| v0.9.0 | T+16 周 | 隐私信任 | 审计日志查询 + 隐私仪表盘（v0.6.0 已预览一键检查 UI） |

---

## 4. v0.6.0 — 通用语义引擎 + 龙忆设计系统 v1.0 ✅ 已交付

> **交付状态**：已完成全部 5 项核心功能 + UI 全面重构 + v0.7/v0.8/v0.9 预览功能。cargo test 456 passed，3 个复杂场景测试全部通过（Playwright 自动化验证）。
> **交付日期**：2026-07-27（UI 重构 + 补丁修复）/ 2026-07-26（通用语义引擎）
> **代码变更**：新增 `src/engine/model_downloader.rs`（~600 行）、`src/engine/embedder.rs`、`src/engine/model_resolver.rs`、`src/engine/luoshu_encoder_ml.rs`，修改 `src/consolidation.rs`、`src/bin/server.rs`、`src/engine/mod.rs`；UI 重构涉及 `static/index.html`、`static/app.css`、`static/app.js`、`static/colors_and_type.css`、`static/components.css`、`static/assets/icons/`（15 个图标）、`static/assets/logo/`；`src/server.rs` 新增静态资源嵌入路由（colors_and_type.css/components.css/15 SVG）。
> **补丁修复**（2026-07-27）：三层基准测试切换标签（设计文档 5.6）、静态资源 404 修复（嵌入 sidecar）、safeJson 作用域修复、搜索 API 端点修复（/recall → /v1/memories/enrich）、版本号硬编码统一为 v0.6.0。

### 4.1 版本目标

**让语义搜索真正可用且通用**——无需 LLM API 即可完成结晶，中文用户开箱即得最优语义精度。

### 4.2 功能规格

#### 4.2.1 统一 Embedder 抽象层（P0 / Must） ✅ 已完成

> **实现位置**：[src/engine/embedder.rs](file:///g:/code-memory/src/engine/embedder.rs)
> **实现内容**：定义 `Embedder` trait（`embed`/`dim`/`model_id`）、`LocalBertEmbedder`（封装 BERT 加载逻辑）、`LlmApiEmbedder`（封装 LLM API embedding）。

**用户故事**：
> As a LRC 架构师，I want to 在代码搜索与结晶路径间共享统一的 Embedder 抽象，so that 切换模型时只需配置一处。

**验收标准**：
- Given 用户设置 `LRC_EMBEDDER_MODEL=BAAI/bge-small-zh`，When 代码搜索与结晶同时触发，Then 两者使用同一模型实例
- Given 投影矩阵维度与模型 hidden_size 不匹配，When 加载洛书编码器，Then 自动重新生成投影矩阵并持久化到 `models/.projection.json`
- Given 模型加载失败，When 任意路径调用嵌入，Then 降级到统计编码器并记录告警日志

**技术实现要点**：
1. 新增 `src/engine/embedder.rs`，定义 `Embedder` trait：
   ```rust
   pub trait Embedder: Send + Sync {
       fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;
       fn dim(&self) -> usize;
       fn model_id(&self) -> &str;
   }
   ```
2. 实现 `LocalBertEmbedder`（封装现有 `CodeBertEncoder` + `LuoShuMlEncoder` 的 BERT 加载逻辑）
3. 实现 `LlmApiEmbedder`（封装 `LlmApiConfig::embed_texts`）
4. 新增 `EmbedderRegistry`：根据配置选择本地或 LLM，支持运行时切换

**优先级**：MoSCoW = Must / RICE = Reach=10, Impact=5, Confidence=0.9, Effort=3 → RICE=15

---

#### 4.2.2 默认模型切换为 BGE-small-zh（P0 / Must） ✅ 已完成

> **实现位置**：[src/engine/luoshu_encoder_ml.rs](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs)
> **实现内容**：`detect_default_model()` 基于系统语言检测默认模型——中文 → BGE-small-zh，英文 → MiniLM-L6-v2；动态投影矩阵适配 512/384 维输入；模型就绪检测由 `model_resolver.rs` 统一管理。

**用户故事**：
> As a 中文用户，I want to 默认使用 BGE-small-zh 模型，so that 无需任何配置即可获得最优中文语义精度。

**验收标准**：
- Given 全新安装的 LRC，When 用户首次调用 `remember`，Then 写入时编码使用 `BAAI/bge-small-zh`（512 维）
- Given 英文环境（`LANG=en_US`），When 首次启动，Then 默认模型为 `sentence-transformers/all-MiniLM-L6-v2`（384 维）
- Given 用户设置 `LRC_EMBEDDER_MODEL=...`，When 启动，Then 该配置覆盖语言检测默认值

**技术实现要点**：
1. 修改 [luoshu_encoder_ml.rs:67-68](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs#L67-68) 默认值，加入语言检测逻辑
2. 投影矩阵从 384 维适配 512 维：动态生成 `W(512×9)`，使用 Xavier 初始化 + 行归一化（保证幻和约束）
3. 持久化投影矩阵到 `models/.projection.{model_id}.json`，避免每次启动重新生成

**优先级**：MoSCoW = Must / RICE = Reach=10, Impact=5, Confidence=0.85, Effort=2 → RICE=21

---

#### 4.2.3 结晶路径支持本地嵌入（P0 / Must） ✅ 已完成

> **实现位置**：[src/consolidation.rs](file:///g:/code-memory/src/consolidation.rs)
> **实现内容**：`embedding_synthesize_cycle()` 接受 `&dyn Embedder` 参数，支持本地嵌入与 LLM API 嵌入统一调用；本地嵌入失败时降级到洛书统计合成（保持现有降级链）；三阶段锁安全模式已保持。

**用户故事**：
> As a 离线用户，I want to 在无 LLM API 的情况下完成结晶，so that 不依赖云端即可享受记忆融合能力。

**验收标准**：
- Given 未配置 LLM API，When 记忆数达到结晶阈值，Then 使用本地 BGE-small-zh 进行 embedding 聚类
- Given 本地嵌入聚类信息增量 < 0.01，When 评估簇，Then 跳过该簇（与 v0.5.18 LLM 路径一致）
- Given 本地嵌入失败，When 结晶周期触发，Then 降级到洛书统计合成（保持现有降级链）
- Given 用户配置 `LRC_SYNTHESIS_EMBEDDER=local|llm|auto`，When 启动，Then 按配置选择结晶嵌入源（auto=优先 llm，失败降级 local）

**技术实现要点**：
1. 重构 [consolidation.rs:412](file:///g:/code-memory/src/consolidation.rs#L412) `llm_synthesize_cycle` 为 `embedding_synthesize_cycle`，参数从 `&LlmApiConfig` 改为 `&dyn Embedder`
2. 新增 `local_synthesize_cycle`：复用 `LocalBertEmbedder`，聚类算法与 LLM 路径相同（余弦相似度贪心聚类）
3. 修改结晶调度器：根据 `LRC_SYNTHESIS_EMBEDDER` 配置选择路径
4. 保持三阶段锁安全模式不变（Phase1 持锁加载 → Phase2 无锁嵌入 → Phase3 持锁写入）

**优先级**：MoSCoW = Must / RICE = Reach=10, Impact=5, Confidence=0.8, Effort=4 → RICE=10

---

#### 4.2.4 下载进度提示与失败重试（P0 / Must） ✅ 已完成

> **实现位置**：[src/engine/model_downloader.rs](file:///g:/code-memory/src/engine/model_downloader.rs)
> **实现内容**：`DownloadProgress` trait（`on_progress`/`on_complete`/`on_error`）、`ConsoleProgress` 控制台进度条、`MirrorSource` 镜像源枚举（HfMirror/ModelScope/Auto）、`DownloadConfig` 下载配置、`ModelDownloader::download_with_retry()` 指数退避重试（initial=2s/max=8s/retries=3）、`build_download_url()` 镜像源 URL 构建、`manual_download_guide()` 手动下载指引。18 个单元测试全部通过。

**用户故事**：
> As a 首次用户，I want to 看到模型下载进度并在失败后自动重试，so that 不会因网络抖动导致语义功能不可用。

**验收标准**：
- Given 首次启动且本地无模型，When 触发下载，Then 控制台输出进度条（已下载/总大小，百分比，速度）
- Given 下载失败（网络超时），When 重试次数 < 3，Then 自动重试，间隔 2s/4s/8s 指数退避
- Given 3 次重试均失败，When 下载终止，Then 输出友好错误信息（含手动下载指引链接 `docs/OFFLINE_MODEL_GUIDE.md`）并降级到 TF-IDF
- Given 用户设置 `LRC_MODEL_MIRROR=modelscope`，When 下载，Then 使用 ModelScope 镜像源

**技术实现要点**：
1. 新增 `src/engine/model_downloader.rs`，封装 ureq 流式下载 + 进度回调
2. 进度回调接口：`pub trait DownloadProgress { fn on_progress(&self, downloaded: u64, total: u64); }`
3. 桌面端通过 Tauri 事件 `model-download-progress` 推送到前端进度条
4. 重试策略：`retry_policy = ExponentialBackoff { initial: 2s, max: 8s, retries: 3 }`
5. 镜像源选择：`LRC_MODEL_MIRROR=hf|modelscope|auto`

**优先级**：MoSCoW = Must / RICE = Reach=10, Impact=4, Confidence=0.9, Effort=3 → RICE=12

---

#### 4.2.5 模型管理 CLI 命令（P1 / Should） ✅ 已完成

> **实现位置**：[src/bin/server.rs](file:///g:/code-memory/src/bin/server.rs)
> **实现内容**：在 CLI 参数解析中添加 `model` 子命令分支，支持四个子命令：
> - `model list` — 列出本地已下载模型（model_id / 路径 / 大小 / 当前默认标记）
> - `model download <model_id>` — 触发下载（带进度条 + 重试）
> - `model use <model_id>` — 设置默认模型
> - `model remove <model_id>` — 删除模型文件
> 辅助函数：`get_models_dir()`、`calculate_dir_size()`、`format_size()`。

**用户故事**：
> As a 用户，I want to 通过 CLI 命令管理模型（列出、下载、切换、删除），so that 不必手动操作文件系统。

**验收标准**：
- Given 执行 `code-memory-server model list`，When 已下载模型存在，Then 输出表格（model_id / 维度 / 大小 / 用途 / 当前默认）
- Given 执行 `code-memory-server model download BAAI/bge-small-zh`，When 网络可用，Then 下载到 `models/BAAI--bge-small-zh/` 并验证完整性
- Given 执行 `code-memory-server model use BAAI/bge-small-zh`，When 模型已就绪，Then 写入配置到 `~/.lrc/config.toml` 并提示重启生效
- Given 执行 `code-memory-server model remove <model_id>`，When 模型存在，Then 询问确认后删除文件

**技术实现要点**：
1. 扩展 [src/bin/server.rs](file:///g:/code-memory/src/bin/server.rs) 的 clap 子命令：`model list|download|use|remove`
2. 配置持久化：`~/.lrc/config.toml`（toml 格式，包含 `default_embedder`、`default_synthesis_embedder` 等字段）
3. 环境变量优先级：CLI 参数 > 环境变量 > 配置文件 > 语言检测默认值

**优先级**：MoSCoW = Should / RICE = Reach=8, Impact=3, Confidence=0.85, Effort=3 → RICE=6.8

---

### 4.3 v0.6.0 关键决策

**决策点 1：本地嵌入 vs LLM API——何时用哪个？**

| 场景 | 推荐选择 | 理由 |
|------|---------|------|
| 写入时编码（每条记忆） | **本地嵌入**（BGE-small-zh） | 高频调用，本地零延迟，无 API 成本 |
| 结晶时聚类（批量） | **优先 LLM，失败降级本地** | LLM 维度更高（1536 vs 512），聚类更精准；本地作为离线兜底 |
| 代码语义搜索 | **本地 BGE**（统一） | 代码搜索本就离线场景为主，BGE 中文能力覆盖代码注释 |
| 用户配置了 LLM 且网络稳定 | **LLM**（自动模式） | 充分利用云端算力，精度最优 |
| 离线/内网环境 | **本地** | 唯一可选 |

**决策点 2：写入时编码 vs 结晶时编码——各自用什么模型？**

| 路径 | 默认模型 | 可选模型 | 维度 |
|------|---------|---------|------|
| 写入时（洛书投影） | BGE-small-zh | MiniLM-L6-v2 / BGE-base-zh / multilingual-e5-small | 512 |
| 结晶时（聚类） | LLM embedding（text-embedding-3-small） | 本地 BGE-small-zh（降级） | 1536 / 512 |
| 代码搜索 | BGE-small-zh（统一） | GraphCodeBERT（向后兼容） | 512 / 768 |

**决策点 3：模型可配置性——用户如何选择模型？**

配置优先级（高 → 低）：
1. CLI 参数：`--embedder-model BAAI/bge-small-zh`
2. 环境变量：`LRC_EMBEDDER_MODEL`、`LRC_LUOSHU_MODEL_ID`（向后兼容）、`LRC_MODEL_ID`（仅代码搜索）
3. 配置文件：`~/.lrc/config.toml` 的 `default_embedder` 字段
4. 语言检测默认值：中文 → BGE-small-zh，其他 → MiniLM-L6-v2

---

### 4.4 v0.6.0 成功指标

| 指标 | 当前基线（v0.5.18） | v0.6.0 目标 | v0.6.0 实际 | 衡量方式 |
|------|---------------------|------------|------------|---------|
| 中文语义精度（NQ MRR@10） | 0.8016（LLM 模式） | ≥ 0.82（本地 BGE 模式） | 待基准测试验证 | benchmarks/lrc_nq_eval.py |
| 离线结晶成功率 | 0%（无 LLM 不可结晶） | ≥ 95% | ✅ 本地嵌入路径已就绪 | 集成测试 |
| 首次下载成功率 | ~70%（无重试） | ≥ 95%（含重试） | ✅ 双镜像 + 3 次重试已实现 | 桌面端用户埋点 |
| 模型切换无需改代码 | 否（需改环境变量） | 是（CLI/配置文件） | ✅ `model use` CLI 已实现 | 手工验证 |
| 单元测试覆盖 | 现有 414 | ≥ 440 | ✅ 456 passed（+42） | cargo test |

---

## 5. v0.7.0 — 非编程场景深度优化

### 5.1 版本目标

**让 LRC 从"AI 编程助手记忆"扩展为"通用个人记忆管家"**——预设 4 套场景模板，覆盖笔记、项目管理、学习助手。

### 5.2 功能规格

#### 5.2.1 预设场景模板（P0 / Must）

**用户故事**：
> As a 知识工作者，I want to 一键启用"个人笔记"场景模板，so that 不必手动设计记忆类型与标签体系。

**4 套预设场景模板**：

| 模板名 | 适用场景 | 预设 memory_type | 预设 tags | 结晶策略 |
|--------|---------|-----------------|-----------|---------|
| `personal-notes` | 个人笔记、灵感、日记 | `note` | `[note, personal]` | 按主题聚类，7 天结晶 |
| `project-management` | 项目决策、会议纪要、任务 | `decision` / `task` | `[project, {id}]` | 按项目聚类，实时结晶 |
| `learning-assistant` | 学习笔记、知识点、问答 | `knowledge` | `[learn, {subject}]` | 按学科聚类，按需结晶 |
| `coding-helper` | 代码决策、偏好、约定（默认） | `code_context` / `preference` | `[code, {lang}]` | 按代码语言聚类（现有） |

**优先级**：MoSCoW = Must / RICE = Reach=9, Impact=5, Confidence=0.85, Effort=4 → RICE=9.6

---

#### 5.2.2 场景感知记忆类型扩展（P0 / Must）

**用户故事**：
> As a 学习者，I want to 记忆类型包含 `knowledge` 和 `question`，so that 区分"我学到的"与"我想问的"。

新增 `MemoryType` 枚举：`Note`、`Decision`、`Task`、`Knowledge`、`Question`

**优先级**：MoSCoW = Must / RICE = Reach=9, Impact=4, Confidence=0.9, Effort=2 → RICE=16.2

---

#### 5.2.3 非编程场景完整使用示例（P1 / Should）

4 套示例：个人笔记、项目管理、学习助手、编程助手

**优先级**：MoSCoW = Should / RICE = Reach=10, Impact=3, Confidence=0.95, Effort=2 → RICE=14.25

---

#### 5.2.4 场景自动识别（P2 / Could）

基于路径 + 内容关键词的简单规则引擎

**优先级**：MoSCoW = Could / RICE = Reach=7, Impact=3, Confidence=0.7, Effort=3 → RICE=4.9

---

### 5.3 v0.7.0 成功指标

| 指标 | 当前基线 | v0.7.0 目标 | 衡量方式 |
|------|---------|------------|---------|
| 非编程场景记忆占比 | < 5% | ≥ 30%（试点用户） | memory_stats 类型分布 |
| 场景模板使用率 | N/A | ≥ 40% 用户启用至少 1 个模板 | 桌面端埋点 |
| 新用户上手时间 | ~15 分钟 | ≤ 5 分钟（含场景示例） | 用户调研 |
| MCP 工具数 | 12 | 13（新增 `scenario` 工具） | server.rs 验证 |

---

## 6. v0.8.0 — 记忆演化可视化

### 6.1 版本目标

**让记忆库的成长可见可感**——用户能直观看到结晶发生、记忆融合、知识抽象的演化轨迹。

### 6.2 功能规格

#### 6.2.1 结晶历史持久化（P0 / Must）
- 每次结晶写入 `crystallization_log.jsonl`
- 新增 MCP 工具 `list_crystallizations`
- 桌面端展示"最近结晶时间"卡片

#### 6.2.2 仪表盘成长轨迹可视化（P0 / Must）
- 3 张图表：记忆总数增长曲线、结晶时间轴、记忆类型分布饼图
- 引入 Chart.js（~60KB）

#### 6.2.3 抽象知识摘要展示（P1 / Should）
- 列出所有 Synthesis 记忆
- 支持点击展开源记忆
- 支持"重新生成"

#### 6.2.4 记忆血缘图谱（P2 / Could）
- 复用 graph_store.rs
- D3.js 力导向图

### 6.3 v0.8.0 成功指标

| 指标 | 当前基线 | v0.8.0 目标 |
|------|---------|------------|
| 用户感知到结晶发生 | ~20% | ≥ 80% |
| 仪表盘日活 | ~30% | ≥ 60% |
| 知识摘要使用率 | 0% | ≥ 35% |

---

## 7. v0.9.0 — 隐私与信任感知

### 7.1 版本目标

**让用户对数据拥有完全掌控感**——一键查看存储位置、网络访问记录、审计日志，支持数据导出与清除。

### 7.2 功能规格

#### 7.9.1 一键隐私检查（P0 / Must）
- 100ms 内返回报告：存储位置、大小、网络访问、加密状态
- 三色信任指示器（绿/黄/红）

#### 7.9.2 网络访问审计日志（P0 / Must）
- 所有网络请求写入审计日志
- 桌面端"网络日志"页面
- 日志保留 30 天

#### 7.9.3 数据导出与清除（P0 / Must）
- 一键导出 ZIP（memories + config + audit）
- 二次确认清除（输入"DELETE"）
- 支持重新导入

#### 7.9.4 数据驻留声明（P1 / Should）
- 100% 本地存储声明
- 开源依赖清单
- 合规认证信息

### 7.3 v0.9.0 成功指标

| 指标 | 当前基线 | v0.9.0 目标 |
|------|---------|------------|
| 隐私检查响应时间 | N/A | < 100ms |
| 用户信任度评分 | 3.5/5 | ≥ 4.5/5 |
| 数据导出成功率 | N/A | ≥ 99% |
| 审计日志覆盖率 | ~40% | 100% |

---

## 8. 关键决策点汇总

### 8.1 决策矩阵：本地嵌入 vs LLM API

| 决策项 | 推荐 | 理由 |
|--------|------|------|
| 写入时编码默认 | **本地 BGE-small-zh** | 高频、零延迟、无成本 |
| 结晶时默认（auto 模式） | **LLM 优先，本地降级** | LLM 精度高，本地兜底 |
| 离线环境 | **本地** | 唯一选项 |
| 企业内网 | **本地** | 数据不出网 |
| 个人开发者（有 API Key） | **LLM** | 精度最优 |
| 中文场景 | **BGE-small-zh** | 中文 SOTA |
| 英文场景 | **MiniLM-L6-v2** | 体积最小 |

### 8.2 决策矩阵：写入时 vs 结晶时模型

| 路径 | v0.6.0 默认 | v0.7.0+ 演进 |
|------|------------|-------------|
| 写入时（洛书投影） | BGE-small-zh（512 维） | 支持用户切换 BGE-base-zh（768 维） |
| 结晶时（聚类） | LLM（auto）→ 本地 BGE（降级） | 引入量化模型（如 BGE-base-q） |
| 代码搜索 | 统一使用 BGE-small-zh | 保留 GraphCodeBERT 向后兼容 |

### 8.3 决策矩阵：模型可配置性

| 配置层级 | 优先级 | 配置方式 | 适用场景 |
|---------|--------|---------|---------|
| L1（最高） | CLI 参数 | `--embedder-model` | 临时测试 |
| L2 | 环境变量 | `LRC_EMBEDDER_MODEL` | 容器/CI |
| L3 | 配置文件 | `~/.lrc/config.toml` | 持久化 |
| L4（最低） | 语言检测 | `LANG=zh_CN` → BGE | 开箱默认 |

---

## 9. 风险评估

### 9.1 技术风险

| 风险编号 | 风险描述 | 影响 | 概率 | 缓解措施 |
|---------|---------|------|------|---------|
| R-T-01 | **投影矩阵维度切换导致已有记忆失效** | 高 | 中 | v0.6.0 引入投影矩阵版本号，迁移时自动重新编码已有记忆 |
| R-T-02 | **本地结晶精度低于 LLM** | 中 | 高 | 保留 LLM 路径作为 auto 模式默认；本地路径作为降级兜底 |
| R-T-03 | **模型下载失败率（国内网络）** | 高 | 中 | 双镜像源 + 3 次指数退避重试 |
| R-T-04 | **Embedder trait 抽象引发性能回退** | 中 | 低 | 基准测试对比 v0.5.18，回归测试覆盖 6 项 BEIR |
| R-T-05 | **场景自动识别误判** | 低 | 中 | v0.7.0 仅用规则引擎，置信度 < 0.6 时询问用户 |

### 9.2 用户体验风险

| 风险编号 | 风险描述 | 影响 | 概率 | 缓解措施 |
|---------|---------|------|------|---------|
| R-U-01 | **用户不理解"本地嵌入 vs LLM"差异** | 中 | 高 | 桌面端首次启动向导增加"嵌入源选择"页 |
| R-U-02 | **模型下载等待导致用户流失** | 高 | 中 | 提供"先试用 TF-IDF，后台下载模型"选项 |
| R-U-03 | **场景模板与用户实际需求不符** | 中 | 中 | 支持用户自定义模板 |
| R-U-04 | **仪表盘信息过载** | 中 | 中 | 渐进式披露：默认仅展示 3 个核心卡片 |

### 9.3 隐私与合规风险

| 风险编号 | 风险描述 | 影响 | 概率 | 缓解措施 |
|---------|---------|------|------|---------|
| R-P-01 | **LLM API 调用泄露用户记忆内容** | 极高 | 中 | v0.9.0 隐私检查高亮显示 LLM 调用；提供"纯本地模式"开关 |
| R-P-02 | **模型下载被中间人篡改** | 高 | 低 | SHA-256 完整性校验 |
| R-P-03 | **审计日志泄露敏感信息** | 中 | 中 | 日志中 LLM API Key 脱敏 |
| R-P-04 | **数据导出包含未加密敏感数据** | 高 | 低 | 导出 ZIP 加密 |

---

## 10. 成功指标总览

### 10.1 北极星指标

| 指标 | v0.5.18 基线 | v0.6.0 | v0.7.0 | v0.8.0 | v0.9.0 |
|------|-------------|--------|--------|--------|--------|
| 中文语义精度（NQ MRR@10） | 0.8016 | ≥ 0.82 | — | — | — |
| 离线结晶成功率 | 0% | ≥ 95% | — | — | — |
| 非编程场景记忆占比 | < 5% | — | ≥ 30% | — | — |
| 用户感知结晶发生 | ~20% | — | — | ≥ 80% | — |
| 用户信任度评分 | 3.5/5 | — | — | — | ≥ 4.5/5 |

---

## 11. 附录

### 11.1 现有代码资源索引

| 资源 | 路径 | 用途 |
|------|------|------|
| 代码语义编码器 | [encoder_codebert.rs](file:///g:/code-memory/src/engine/encoder_codebert.rs) | Smart Match 代码搜索 |
| 洛书 ML 编码器 | [luoshu_encoder_ml.rs](file:///g:/code-memory/src/engine/luoshu_encoder_ml.rs) | 写入时语义编码 |
| LLM 翻译器 | [llm_translator.rs](file:///g:/code-memory/src/engine/llm_translator.rs) | LLM API 调用（chat + embedding） |
| 结晶模块 | [consolidation.rs](file:///g:/code-memory/src/consolidation.rs) | 记忆合成主流程 |
| 模型就绪检测 | [model_resolver.rs](file:///g:/code-memory/src/engine/model_resolver.rs) | 统一模型文件检测 |
| 池化策略 | [pooling.rs](file:///g:/code-memory/src/engine/pooling.rs) | BERT 池化（CLS/Mean） |
| 审计追踪 | [audit_trail.rs](file:///g:/code-memory/src/engine/audit_trail.rs) | 关键操作审计 |
| 数据导出 | [export.rs](file:///g:/code-memory/src/export.rs) | 数据导出基础 |
| 仪表盘 | [dashboard.rs](file:///g:/code-memory/src/dashboard.rs) | Web 仪表盘后端 |
| MCP 工具集 | [server.rs](file:///g:/code-memory/src/server.rs) | 12 个 MCP 工具定义 |

### 11.2 推荐模型对比表

| 模型 | 维度 | 大小 | 中文 | 英文 | 推荐场景 |
|------|------|------|------|------|---------|
| BAAI/bge-small-zh | 512 | ~100MB | ★★★★★ | ★★★ | 中文默认 |
| BAAI/bge-base-zh | 768 | ~400MB | ★★★★★ | ★★★ | 中文高精度 |
| sentence-transformers/all-MiniLM-L6-v2 | 384 | ~80MB | ★★ | ★★★★★ | 英文默认 |
| multilingual-e5-small | 384 | ~120MB | ★★★★ | ★★★★ | 多语言 |
| text2vec-base-chinese | 768 | ~400MB | ★★★★ | ★★ | 中文语义匹配经典 |
| microsoft/graphcodebert-base | 768 | ~500MB | ★★ | ★★★★ | 代码搜索（向后兼容） |

### 11.3 国内镜像源

| 镜像 | URL | 配置方式 |
|------|-----|---------|
| HF-Mirror | `https://hf-mirror.com` | `HF_ENDPOINT=https://hf-mirror.com`（默认已设） |
| ModelScope | `https://modelscope.cn` | `LRC_MODEL_MIRROR=modelscope` |

### 11.4 术语表

| 术语 | 定义 |
|------|------|
| **结晶（Crystallization）** | LRC 后台批量聚类相似记忆，生成 Synthesis 类型抽象记忆的过程 |
| **洛书向量** | 9 维语义坐标向量，受洛书幻和约束，用于快速聚类与索引 |
| **Embedder** | 嵌入器，将文本转为高维向量的统一抽象（v0.6.0 引入） |
| **信息增量** | 1 - 簇内平均余弦相似度，衡量簇内多样性，低于 0.01 跳过结晶 |
| **三阶段锁安全** | Phase1 持锁加载 → Phase2 无锁 I/O → Phase3 持锁写入，避免长锁持有 |
| **MCP** | Model Context Protocol，AI 助手与工具交互的标准协议 |

---

## 12. 文档评审与签核

| 角色 | 姓名 | 签核日期 | 备注 |
|------|------|---------|------|
| 产品负责人 | 创世者设计 | 2026-07-26 | 初版起草 |
| 技术负责人 | （待签） | | |
| 用户代表 | （待签） | | |

---

**文档结束**

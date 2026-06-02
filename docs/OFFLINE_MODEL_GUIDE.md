# Smart Match 语义模型离线安装指南

> 适用场景：网络受限、内网环境、或下载速度慢——希望手动下载模型文件，完全离线使用。

---

## 方式一：放到项目 `models/` 文件夹（最简单，推荐）

LRC 启动时会优先检查项目根目录下的 `models/` 文件夹。只需把模型文件放进去即可。

### 1. 下载模型文件

打开浏览器，访问 HuggingFace 国内镜像站：

```
https://hf-mirror.com/microsoft/graphcodebert-base/tree/main
```

逐个下载以下文件：

| 文件名 | 大小 | 作用 |
|------|------|------|
| `config.json` | ~1 KB | 模型结构配置 |
| `tokenizer.json` | ~800 KB | 代码分词器 |
| `model.safetensors` | ~500 MB | 模型权重 |

> 如果只有 `pytorch_model.bin` 格式也可以，LRC 会自动识别。

### 2. 放到正确位置

在 LRC 项目根目录下创建 `models/microsoft--graphcodebert-base/` 文件夹，把下载的文件放进去：

```
你的LRC项目目录/
├── models/
│   └── microsoft--graphcodebert-base/
│       ├── config.json
│       ├── tokenizer.json
│       └── model.safetensors
├── Cargo.toml
└── src/
```

> 注意：文件夹名是 `microsoft--graphcodebert-base`（`/` 替换为 `--`）。

### 3. 重启服务

LRC 启动时会优先检测 `models/` 文件夹，发现后直接加载，完全不走网络。

```bash
code-memory-server --src-dir ./src --port 3099
```

看到以下日志说明加载成功：

```
✓ 使用本地模型: .../models/microsoft--graphcodebert-base
本地模型加载成功 (hidden_size=768, device=CPU)
```

---

## 方式二：使用 huggingface-cli 下载（命令行）

如果你有 Python 环境：

```bash
# 安装 huggingface_hub
pip install huggingface_hub

# 从国内镜像下载到 LRC 的 models/ 文件夹
huggingface-cli download microsoft/graphcodebert-base \
  --local-dir ./models/microsoft--graphcodebert-base \
  --endpoint https://hf-mirror.com
```

---

## 方式三：放到 HuggingFace 缓存目录

如果你已经通过 `huggingface-cli` 或其他方式下载过模型，可以放到缓存目录：

### 1. 找到缓存目录

**Windows（PowerShell）：**
```powershell
echo "$env:USERPROFILE\.cache\huggingface\hub\"
```

**Linux / macOS：**
```bash
echo ~/.cache/huggingface/hub/
```

### 2. 确认目标路径

模型缓存目录结构如下：

```
~/.cache/huggingface/hub/
└── models--microsoft--graphcodebert-base/
    └── snapshots/
        └── <一串哈希值>/     ← 这是关键！
            ├── config.json
            ├── tokenizer.json
            └── model.safetensors
```

### 3. 获取快照哈希值

在 `models--microsoft--graphcodebert-base/refs/` 目录下有一个 `main` 文件，内容是当前版本的哈希值。

### 4. 放置文件并验证

```bash
# 假设哈希值为 abc123def456...
cp config.json ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
cp tokenizer.json ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
cp model.safetensors ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
```

---

## 方式四：使用 HF_ENDPOINT 环境变量自动下载

如果你能访问网络，LRC 会自动使用国内镜像 `hf-mirror.com` 下载模型：

**Windows（PowerShell）：**
```powershell
# LRC 默认已设置 HF_ENDPOINT，直接启动即可
code-memory-server --src-dir ./src --port 3099
```

**Linux / macOS：**
```bash
# LRC 默认已设置 HF_ENDPOINT，直接启动即可
code-memory-server --src-dir ./src --port 3099
```

如果使用了代理，添加 `--proxy` 参数：

```bash
code-memory-server --src-dir ./src --port 3099 --proxy http://127.0.0.1:7890
```

---

## 验证安装

启动 Smart Match 模式，观察日志输出：

```bash
cargo run --features server,ml -- --src-dir ./src --port 3099
```

如果看到以下日志，说明模型加载成功：

```
模型: microsoft/graphcodebert-base (hf-mirror.com)
external encoder loaded (hidden_size=768, device=CPU)
搜索模式: Smart Match（语义理解 · 首次启动需下载模型）
```

如果看到错误，请检查：

| 错误信息 | 可能原因 | 解决方法 |
|---------|---------|---------|
| `config.json: ...` | 文件未找到或路径错误 | 检查文件是否在正确的快照目录下 |
| `tokenizer.json: ...` | 分词器文件缺失 | 确认下载了 `tokenizer.json` |
| `模型文件下载失败` | 权重文件缺失或格式不匹配 | 确认下载了 `model.safetensors` 或 `pytorch_model.bin` |
| `hf-hub init` | 缓存目录无法访问 | 检查 `~/.cache/huggingface/` 目录权限 |

---

## 切换模型

如果想使用其他 CodeBERT 系列模型，设置环境变量：

```bash
# 使用 CodeBERT（而非默认的 GraphCodeBERT）
export LRC_MODEL_ID=microsoft/codebert-base

# 使用 CodeBERT MLM（掩码语言模型变体）
export LRC_MODEL_ID=microsoft/codebert-base-mlm
```

> 注意：切换模型后需要重新下载对应的模型文件。不同模型的缓存目录不同。

---

## 常见问题

### Q：必须用 safetensors 格式吗？

优先使用 `model.safetensors`。如果只有 `pytorch_model.bin`，程序也能加载，但会慢一些。

### Q：能把模型放在项目目录里吗？

**可以！** 这是推荐方式。在项目根目录创建 `models/microsoft--graphcodebert-base/` 文件夹，放入模型文件即可。LRC 启动时会优先加载本地模型，完全不走网络。详见上方"方式一"。

### Q：能完全离线使用吗？

可以。只要模型文件已放到 `models/` 文件夹或缓存目录，启动 Smart Match 模式时不会联网。程序只在首次加载（文件缺失）时尝试下载。

### Q：模型文件太大，有更小的替代吗？

目前 GraphCodeBERT 是最小可用的代码语义模型。我们正在评估更轻量的替代方案（如 MiniLM），详见 [模型评估报告](MODEL_EVALUATION.md)。
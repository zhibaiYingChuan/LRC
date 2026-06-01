# Smart Match 语义模型离线安装指南

> 适用场景：网络受限、内网环境、或下载速度慢——希望手动下载模型文件，完全离线使用。

---

## 你需要下载什么

Smart Match 模式默认使用 **GraphCodeBERT**（`microsoft/graphcodebert-base`），共需要 3 个文件：

| 文件名 | 大小 | 作用 |
|--------|------|------|
| `config.json` | ~1 KB | 模型结构配置 |
| `tokenizer.json` | ~800 KB | 代码分词器 |
| `model.safetensors` | ~500 MB | 模型权重（主力） |

> 总计约 **500 MB**。如果下载的是 `pytorch_model.bin` 格式，大小相近。

---

## 方式一：从国内镜像下载（推荐）

### 1. 下载模型文件

打开浏览器，访问 HuggingFace 国内镜像站：

```
https://hf-mirror.com/microsoft/graphcodebert-base/tree/main
```

逐个下载上面表格中的 3 个文件，保存到本地任意目录（比如 `D:\models\graphcodebert-base\`）。

> 如果镜像站无法访问，可以尝试原始站：
> `https://huggingface.co/microsoft/graphcodebert-base/tree/main`

### 2. 找到缓存目录

下载完成后，需要把文件放到 `hf_hub` 的缓存目录中。执行以下命令查看缓存路径：

**Windows（PowerShell）：**
```powershell
echo "$env:USERPROFILE\.cache\huggingface\hub\"
```

**Linux / macOS：**
```bash
echo ~/.cache/huggingface/hub/
```

### 3. 确认目标路径

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

> 问题：`<一串哈希值>` 是什么？这是 HuggingFace 的版本快照 ID。你可以通过以下方式获取：

### 4. 获取快照哈希值

**方法 A：先让程序下载一次（推荐）**

连接网络，启动一次 Smart Match 模式，程序会自动下载模型。下载完成后，缓存目录中就会出现正确的哈希值文件夹。此时：

```bash
# 启动一次（会下载模型，耗时约 3-5 分钟）
code-memory-server --src-dir ./src --port 3099
# Ctrl+C 停止
```

然后进入缓存目录，记下 `snapshots/` 下的文件夹名（就是一串哈希值）。

**方法 B：从 refs 文件获取**

在 `models--microsoft--graphcodebert-base/refs/` 目录下有一个 `main` 文件，内容是当前版本的哈希值。

### 5. 放置文件并验证

```bash
# 假设哈希值为 abc123def456...
# 将下载的 3 个文件复制到该目录下

cp config.json ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
cp tokenizer.json ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
cp model.safetensors ~/.cache/huggingface/hub/models--microsoft--graphcodebert-base/snapshots/abc123def456.../
```

---

## 方式二：使用 huggingface-cli 下载（命令行）

如果你有 Python 环境，这是最简单的方式：

```bash
# 安装 huggingface_hub
pip install huggingface_hub

# 从国内镜像下载（推荐）
huggingface-cli download microsoft/graphcodebert-base \
  --local-dir ./models/graphcodebert-base \
  --endpoint https://hf-mirror.com

# 或从官方源下载
huggingface-cli download microsoft/graphcodebert-base \
  --local-dir ./models/graphcodebert-base
```

下载完成后，`./models/graphcodebert-base/` 目录下就是所有需要的文件。

---

## 方式三：使用 HF_ENDPOINT 环境变量

如果你能访问网络，只是下载慢，可以设置镜像端点并让程序自动下载：

**Windows（PowerShell）：**
```powershell
$env:HF_ENDPOINT = "https://hf-mirror.com"
code-memory-server --src-dir ./src --port 3099
```

**Linux / macOS：**
```bash
export HF_ENDPOINT=https://hf-mirror.com
code-memory-server --src-dir ./src --port 3099
```

程序会自动从镜像站下载并缓存，无需手动操作。

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

目前 `hf_hub` 固定使用 `~/.cache/huggingface/hub/` 作为缓存目录。后续版本会支持自定义模型路径。

### Q：能完全离线使用吗？

可以。只要模型文件已下载到缓存目录，启动 Smart Match 模式时不会联网。程序只在首次加载（文件缺失）时尝试下载。

### Q：模型文件太大，有更小的替代吗？

目前 GraphCodeBERT 是最小可用的代码语义模型。我们正在评估更轻量的替代方案（如 MiniLM），详见 [模型评估报告](MODEL_EVALUATION.md)。
#!/bin/bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

echo "========================================"
echo "   Loong Recall (L-RC) 一键安装脚本"
echo "========================================"
echo ""
echo "本脚本将依次完成：检测 Rust 环境 → 编译 → 配置 IDE MCP 连接"
echo ""

# 1. 检测 Rust 环境
if ! command -v cargo &> /dev/null; then
    echo "[错误] 未检测到 Rust 环境。"
    echo "请安装 Rust：curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "国内用户建议使用镜像：https://mirrors.ustc.edu.cn/rust-static/rustup/"
    exit 1
fi

# 1.5 解析命令行参数
WITH_MODELS=false
for arg in "$@"; do
    case "$arg" in
        --with-models|-m)
            WITH_MODELS=true
            ;;
        *)
            echo "未知参数: $arg"
            echo "用法: ./install.sh [--with-models]"
            echo "  --with-models, -m  编译 ML 增强模式并预下载语义模型"
            exit 1
            ;;
    esac
done

# 2. 编译项目
if $WITH_MODELS; then
    echo "[1/3] 正在编译 Loong Recall（ML 语义增强模式）..."
    echo "   包含 ML 模型支持，首次运行会自动下载约 500MB 模型"
    echo "   国内用户已自动配置 HF_ENDPOINT=https://hf-mirror.com 镜像加速"
    cargo build --release --features server,ml
else
    echo "[1/3] 正在编译 Loong Recall（首次编译约 5-10 分钟）..."
    echo "   默认 zero-dependency 模式，无需下载模型"
    echo "   国内用户如遇 crates.io 下载缓慢，可配置 Cargo 镜像："
    echo "   在 ~/.cargo/config.toml 中添加："
    echo "     [source.crates-io]"
    echo "     replace-with = 'ustc'"
    echo "     [source.ustc]"
    echo "     registry = 'sparse+https://mirrors.ustc.edu.cn/crates.io-index/'"
    echo ""
    cargo build --release --features server
fi

SERVER_PATH="$SCRIPT_DIR/target/release/code-memory-server"
SRC_DIR="$SCRIPT_DIR/src"
echo "编译完成！"

# 3. Smart Match 增强模式说明
if $WITH_MODELS; then
    echo ""
    echo "[2/3] 正在预下载语义模型（约 500MB，首次运行需此步骤）..."
    echo "   使用 HF 镜像: https://hf-mirror.com"
    echo "   模型: microsoft/graphcodebert-base"
    echo ""
    # 通过临时运行服务来触发模型下载
    HF_ENDPOINT="https://hf-mirror.com" timeout 120 "$SERVER_PATH" --mode smart --port 18999 --db-path "$SCRIPT_DIR/.loong-recall/data" --src-dir "$SRC_DIR" 2>&1 || true
    # 检查模型是否下载成功
    if [ -f "$SCRIPT_DIR/models/microsoft--graphcodebert-base/config.json" ]; then
        echo "   模型预下载完成，ML 语义搜索已就绪！"
    else
        echo "[提示] 模型下载可能未完成，首次运行服务时会自动继续下载。"
    fi
    echo ""
    echo "   已编译为 ML 语义增强模式，启动时自动启用语义搜索："
    echo "    \"$SERVER_PATH\" --mode smart --src-dir <项目路径>"
else
    echo ""
    echo "[可选] 如需更高精度的语义搜索，可启用 ML 模式："
    echo "  cargo build --release --features server,ml"
    echo "  启用后首次运行会自动下载约 500MB 模型文件。"
    echo "  国内用户可设环境变量 HF_ENDPOINT=https://hf-mirror.com 加速。"
    echo ""
    echo "  当前已编译为 fast 模式（词向量编码），可直接使用："
    echo "    \"$SERVER_PATH\" --mode fast --src-dir <项目路径>"
    echo ""
fi

# 4. 查找可用的 IDE 并配置 MCP
echo "[2/3] 正在搜索本地 IDE..."

# 辅助函数：配置 IDE 的 MCP 连接
config_ide() {
    local ide_name="$1"
    local config_file="$2"
    local config_dir
    config_dir="$(dirname "$config_file")"

    echo "   发现 $ide_name，正在配置 MCP..."

    # 确保配置目录存在
    mkdir -p "$config_dir"

    if [ -f "$config_file" ]; then
        # 检查是否已有 loong-recall 配置
        if grep -q "loong-recall" "$config_file" 2>/dev/null; then
            echo "   已存在 loong-recall 配置，跳过。"
            return
        fi
        # 文件存在但不含 loong-recall，提示手动合并
        echo ""
        echo "   [提示] $config_file 已存在其他 MCP 配置。"
        echo "   请手动将以下内容合并到该文件的 \"mcpServers\" 对象中："
        echo ""
        echo "   \"loong-recall\": {"
        echo "     \"command\": \"$SERVER_PATH\","
        echo "     \"args\": [\"--src-dir\", \"$SRC_DIR\", \"--stdio\"]"
        echo "   }"
        echo ""
    else
        # 配置文件不存在，直接创建
        cat > "$config_file" << EOF
{
  "mcpServers": {
    "loong-recall": {
      "command": "$SERVER_PATH",
      "args": ["--src-dir", "$SRC_DIR", "--stdio"]
    }
  }
}
EOF
        echo "   已创建 $config_file。"
    fi
}

# 4a. Trae
if [ -d "$HOME/Library/Application Support/Trae" ]; then
    config_ide "Trae" "$HOME/Library/Application Support/Trae/User/mcp.json"
elif [ -d "$HOME/.config/Trae" ]; then
    config_ide "Trae" "$HOME/.config/Trae/User/mcp.json"
fi

# 4b. Trae CN
if [ -d "$HOME/Library/Application Support/Trae CN" ]; then
    config_ide "Trae CN" "$HOME/Library/Application Support/Trae CN/User/mcp.json"
elif [ -d "$HOME/.config/Trae CN" ]; then
    config_ide "Trae CN" "$HOME/.config/Trae CN/User/mcp.json"
fi

# 4c. Cursor
if [ -d "$HOME/Library/Application Support/Cursor" ]; then
    config_ide "Cursor" "$HOME/Library/Application Support/Cursor/mcp.json"
elif [ -d "$HOME/.config/Cursor" ]; then
    config_ide "Cursor" "$HOME/.config/Cursor/mcp.json"
fi

# 4d. VS Code
if [ -d "$HOME/Library/Application Support/Code" ]; then
    config_ide "VS Code" "$HOME/Library/Application Support/Code/User/mcp.json"
elif [ -d "$HOME/.config/Code" ]; then
    config_ide "VS Code" "$HOME/.config/Code/User/mcp.json"
fi

echo ""
echo "[3/3] 安装完成！"
echo "========================================"
echo ""
echo "请重启你的 IDE（Trae / Cursor / VS Code），"
echo "AI 助手将自动识别 Loong Recall 工具。"
echo ""
echo "你可以手动启动服务测试："
echo "  \"$SERVER_PATH\" --src-dir \"$SRC_DIR\" --port 3099"
echo ""
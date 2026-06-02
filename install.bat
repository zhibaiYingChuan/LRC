@echo off
chcp 65001 >nul
title Loong Recall 一键安装
color 0A
echo ========================================
echo    Loong Recall (L-RC) 一键安装脚本
echo ========================================
echo.
echo 本脚本将依次完成：检测 Rust 环境 → 编译 → 配置 IDE MCP 连接
echo.

REM 1. 检测 Rust 环境
where cargo >nul 2>nul
if %errorlevel% neq 0 (
    echo [错误] 未检测到 Rust 环境。
    echo 请先安装 Rust：https://rustup.rs/
    echo 国内用户建议使用镜像：https://mirrors.ustc.edu.cn/rust-static/rustup/
    pause
    exit /b 1
)

REM 2. 获取脚本所在目录
set "SCRIPT_DIR=%~dp0"
set "SCRIPT_DIR=%SCRIPT_DIR:~0,-1%"
cd /d "%SCRIPT_DIR%"

REM 3. 编译项目
echo [1/3] 正在编译 Loong Recall（首次编译约 2-5 分钟）...
cargo build --release --features server
if %errorlevel% neq 0 (
    echo [错误] 编译失败，请检查上方错误信息。
    pause
    exit /b 1
)
echo 编译完成！

REM 4. 模型下载指引（Smart Match 模式需要）
echo.
echo [提示] 如果使用 Smart Match 模式（语义搜索），首次启动需要下载模型。
echo 自动使用国内镜像 hf-mirror.com，约 500MB。
echo.
echo 如果下载缓慢，可手动下载模型放到 models/ 目录：
echo   1. 访问 https://hf-mirror.com/microsoft/graphcodebert-base
echo   2. 下载所有文件到: %SCRIPT_DIR%\models\microsoft--graphcodebert-base\
echo   3. 重启服务即可（LRC 自动优先加载本地模型）
echo.
echo 如果使用代理，启动时添加 --proxy http://127.0.0.1:端口
echo.

REM 5. 计算可执行文件路径（使用正斜杠，兼容 JSON）
set "SERVER_PATH=%SCRIPT_DIR%\target\release\code-memory-server.exe"
set "SERVER_PATH_JSON=%SERVER_PATH:\=/%"
set "SRC_DIR_JSON=%SCRIPT_DIR:\=/%/src"

REM 6. 查找可用的 IDE 并配置 MCP
echo [2/3] 正在搜索本地 IDE...

REM 5a. Trae
set "TRAE_USER=%APPDATA%\Trae\User"
if exist "%TRAE_USER%" (
    call :config_ide "Trae" "%TRAE_USER%\mcp.json"
)
REM Trae CN 变体
set "TRAE_CN_USER=%APPDATA%\Trae CN\User"
if exist "%TRAE_CN_USER%" (
    call :config_ide "Trae CN" "%TRAE_CN_USER%\mcp.json"
)

REM 5b. Cursor
set "CURSOR_DIR=%APPDATA%\Cursor"
if exist "%CURSOR_DIR%" (
    call :config_ide "Cursor" "%CURSOR_DIR%\mcp.json"
)

REM 5c. VS Code
set "CODE_USER=%APPDATA%\Code\User"
if exist "%CODE_USER%" (
    call :config_ide "VS Code" "%CODE_USER%\mcp.json"
)

echo.
echo [3/3] 安装完成！
echo ========================================
echo.
echo 请重启你的 IDE（Trae / Cursor / VS Code），
echo AI 助手将自动识别 Loong Recall 工具。
echo.
echo 你可以手动启动服务测试：
echo   "%SERVER_PATH%" --src-dir "%SCRIPT_DIR%\src" --port 3099
echo.
pause
exit /b 0

REM =============================================
REM 子程序：为指定 IDE 配置 MCP 连接
REM 参数：%1 = IDE 名称，%2 = 配置文件路径
REM =============================================
:config_ide
set "IDE_NAME=%1"
set "CONFIG_FILE=%2"
set "CONFIG_DIR=%~dp2"

echo   发现 %IDE_NAME%，正在配置 MCP...

REM 如果配置文件所在目录不存在，创建它
if not exist "%CONFIG_DIR%" mkdir "%CONFIG_DIR%"

REM 生成 MCP 配置 JSON 内容
set "MCP_JSON={ \"mcpServers\": { \"loong-recall\": { \"command\": \"%SERVER_PATH_JSON%\", \"args\": [\"--src-dir\", \"%SRC_DIR_JSON%\", \"--stdio\"] } } }"

if exist "%CONFIG_FILE%" (
    REM 文件已存在，检查是否已有 loong-recall 配置
    findstr /c:"loong-recall" "%CONFIG_FILE%" >nul 2>&1
    if %errorlevel% equ 0 (
        echo   已存在 loong-recall 配置，跳过。
        exit /b
    )
    REM 文件存在但不含 loong-recall，提示手动合并
    echo.
    echo   [提示] %CONFIG_FILE% 已存在其他 MCP 配置。
    echo   请手动将以下内容合并到该文件的 "mcpServers" 对象中：
    echo.
    echo   "loong-recall": {
    echo     "command": "%SERVER_PATH_JSON%",
    echo     "args": ["--src-dir", "%SRC_DIR_JSON%", "--stdio"]
    echo   }
    echo.
) else (
    REM 配置文件不存在，直接创建
    echo %MCP_JSON% > "%CONFIG_FILE%"
    echo   已创建 %CONFIG_FILE%。
)
exit /b
@echo off
REM ============================================================
REM LRC 开发版启动（双击可直接运行）
REM 启动开发版实例，端口 3100，数据目录与稳定版隔离
REM 稳定版（桌面端）: 端口 3099
REM ============================================================

cd /d "%~dp0.."

echo ============================================
echo   LRC 开发版 - 双击启动
echo   端口: 3100
echo   数据目录: %%USERPROFILE%%\.loong-recall\dev\data
echo   稳定版桌面端不受影响（端口 3099）
echo ============================================
echo.

REM 检查 cargo 是否可用
where cargo >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo [错误] 未找到 cargo，请确保 Rust 已安装
    pause
    exit /b 1
)

REM 检查是否已有进程占用端口 3100
netstat -ano | findstr ":3100" >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    echo [警告] 端口 3100 已被占用，请先关闭旧进程
    echo 可以运行: taskkill /F /IM lrc-sidecar.exe /IM code-memory-server.exe
    pause
)

REM 确保数据目录存在
if not exist "%USERPROFILE%\.loong-recall\dev\data" (
    mkdir "%USERPROFILE%\.loong-recall\dev\data"
    echo [创建] 数据目录: %%USERPROFILE%%\.loong-recall\dev\data
)

echo [启动] 开发版 LRC（首次启动会编译，请稍候...）
echo [提示] 按 Ctrl+C 停止开发版
echo.
cargo run --features server -- --port 3100 --data-dir "%USERPROFILE%\.loong-recall\dev\data" --global

pause
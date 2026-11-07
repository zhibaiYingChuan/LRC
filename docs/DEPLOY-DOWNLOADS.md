# 国内下载页部署说明（sfang.cc/longyi）

> 目标：把桌面端三个安装包放到自有服务器，国内用户直连下载。
> 状态：**v0.10.2 已执行完成（2026-11-07）**。

---

## 一、实际落点

| 项 | 值 |
|---|---|
| 服务器 | `139.224.231.170`（阿里云 Windows Server，主机名 `iZm9f8iqhu5ee6Z`） |
| SSH | `ssh -i C:\Users\Administrator\.ssh\aml_deploy_rebuild Administrator@139.224.231.170` |
| 安装包目录 | `C:\wwwroot\sfang.cc\download\dist\downloads\` |
| 公网地址 | `https://sfang.cc/xd/downloads/<文件名>` |
| 站点源码 | `G:\SFanggw`（Next.js 15，App Router） |
| 页面源码 | `G:\SFanggw\src\app\longyi\page.tsx` |

nginx 配置（`C:\nginx\conf\nginx.conf`）：

```nginx
location /xd/downloads/ {
    alias C:/wwwroot/sfang.cc/download/dist/downloads/;
}
```

> ⚠ **路径易错点**：`/xd/downloads/` 实际映射到 `sfang.cc\download\dist\downloads\`，
> **不是** `sfang.cc\xd\dist\downloads\`。v0.10.2 首次上传时按旧文档传错目录，
> 公网 404；以 nginx 配置为准。

---

## 二、已上传的文件（v0.10.2）

| 平台 | 文件名 | 字节 |
|---|---|---:|
| Windows | `lrc-desktop-v0.10.2-windows-x86_64-setup.exe` | 6,786,683 |
| macOS (Apple Silicon) | `lrc-desktop-v0.10.2-macos-arm64.dmg` | 8,423,588 |
| Linux | `lrc-desktop-v0.10.2-linux-amd64.deb` | 9,732,484 |

每个文件旁都有同名 `.sha256`。

**注意**：服务器直连 GitHub 会 `curl: (35) Connection was reset`，所以必须**本地下载后 scp**，不能在服务器上 curl。
GitHub 直连下载极慢（~17KB/s），用 `https://ghfast.top/` 代理下载后再用 `.sha256` 校验。

```powershell
$d = "$env:TEMP\lrc-dl-v0102"
$base = "https://ghfast.top/https://github.com/zhibaiYingChuan/LRC/releases/download/v0.10.2"
foreach ($f in @("lrc-desktop-v0.10.2-windows-x86_64-setup.exe",
                 "lrc-desktop-v0.10.2-macos-arm64.dmg",
                 "lrc-desktop-v0.10.2-linux-amd64.deb")) {
  curl.exe -L --retry 3 -s -o "$d\$f" "$base/$f"
}
scp -i C:\Users\Administrator\.ssh\aml_deploy_rebuild "$d\lrc-desktop-v0.10.2-*" `
  Administrator@139.224.231.170:C:/wwwroot/sfang.cc/download/dist/downloads/
```

> `latest.json` 属于**玄盾**的 Tauri 更新器清单，与 LRC 无关，**不要改**。

---

## 三、页面改动

新增两个文件：

- `src/app/longyi/downloads.ts` —— 三个下载地址的单一真源（**普通模块，不能带 `"use client"`**）
- `src/app/longyi/DownloadButton.tsx` —— 按 UA 推荐平台安装包

`page.tsx` 改动：

- 首屏主按钮：`查看开源仓库` → `下载 <平台> 版`，开源仓库降为次按钮
- 第六章「方式一 · 下载桌面端」：补三平台直链 + 版本与校验说明

### 踩过的坑

从 `"use client"` 文件导出常量、再被服务端组件引用时，Next 会把它替换成**客户端引用代理**，
服务端渲染取到 `undefined`，结果是 `<a>` 渲染出来但**没有 href**（页面看着正常，点了没反应）。
所以 `DOWNLOADS` 必须放在独立的普通模块里。

---

## 四、部署流程（本地构建 → 上传 .next）

服务器只有 1 核 1.9GB，**禁止在服务器上构建**。

```powershell
# 1) 本地构建
cd G:\SFanggw
npm run build
Get-Content .next\BUILD_ID          # 必须存在，否则是 dev 残留

# 2) 暂存（排除 .next/cache，否则 200MB+）
robocopy G:\SFanggw\.next "$env:TEMP\nextstage" /E /XD cache /NFL /NDL /NJH /NJS /NP

# 3) 打包（必须用 ZipFile，且先删旧 zip）
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory(
  "$env:TEMP\nextstage", "G:\SFanggw\next-new.zip",
  [System.IO.Compression.CompressionLevel]::Optimal, $false)

# 4) 上传
scp -i C:\Users\Administrator\.ssh\aml_deploy_rebuild G:\SFanggw\next-new.zip `
  Administrator@139.224.231.170:C:/wwwroot/sfang.cc/next-new.zip

# 5) 服务器执行（停任务 → 备份 .next → 解压 → icacls → 启动）
ssh -i C:\Users\Administrator\.ssh\aml_deploy_rebuild Administrator@139.224.231.170 `
  "C:\wwwroot\sfang.cc\deploy-next.bat"
```

`deploy-next.bat` 会把旧 `.next` 移到 `.next.bak-deploy`，回滚只需反向改名并重启 `SFangNext`。

---

## 五、验证（v0.10.2 已通过）

```powershell
# 1) 三个包公网可下载，Content-Length 与源文件一致
foreach ($f in @("lrc-desktop-v0.10.2-windows-x86_64-setup.exe",
                 "lrc-desktop-v0.10.2-macos-arm64.dmg",
                 "lrc-desktop-v0.10.2-linux-amd64.deb")) {
  $r = Invoke-WebRequest "https://sfang.cc/xd/downloads/$f" -Method Head -UseBasicParsing
  "$f  $($r.StatusCode)  $($r.Headers['Content-Length'])"
}
# 实测：200 / 6786683、200 / 8423588、200 / 9732484

# 2) 页面三个链接都在
(Invoke-WebRequest "https://sfang.cc/longyi" -UseBasicParsing).Content `
  | Select-String "lrc-desktop-v0.10.2-[a-z0-9_.-]+" -AllMatches
# 实测：exe / dmg / deb 三条齐全

# 3) 哈希一致
Get-FileHash <本地文件> -Algorithm SHA256   # 与 .sha256 文件比对
# 实测：三个全部 match=True
```

---

## 六、后续发版标准流程（v0.10.2 起固定）

1. **GitHub 发布**：版本号 10 处同步 → 更新 CHANGELOG → commit → `git tag vX.Y.Z` → push tag，等 Release 工作流产出三平台安装包。
2. **写发布页更新日志**：`gh release edit vX.Y.Z --notes-file <notes.md>`。
3. **下载安装包**：用 `https://ghfast.top/` 代理下载（GitHub 直连 ~17KB/s），并用 `.sha256` 校验。
4. **上传服务器**：`scp` 到 `C:/wwwroot/sfang.cc/download/dist/downloads/`（**不是** `xd/dist/downloads/`）。
5. **更新页面版本**：改 `G:\SFanggw\src\app\longyi\downloads.ts` 的 `VERSION`。
6. **构建并部署**：`npm run build` → robocopy 暂存（排除 cache）→ ZipFile 打包 → scp `next-new.zip` → 服务器执行 `deploy-next.bat`。
7. **验证**：三个公网 URL 200 + Content-Length 一致；`/longyi` 页面三条链接齐全；SHA256 一致。
8. **记录**：把本次版本号、文件大小、SHA256 前 16 位写入记忆库。

> 旧版本安装包可保留（不删），便于回滚与历史下载。

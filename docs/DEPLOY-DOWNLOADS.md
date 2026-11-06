# 国内下载页部署说明（sfang.cc/longyi）

> 目标：把 v0.10.0 的三个安装包放到自有服务器，国内用户直连下载。
> 状态：**待执行** —— 需要服务器访问权限。

---

## 一、要上传的三个文件

| 平台 | 文件名 | 大小 |
|---|---|---|
| Windows | `lrc-desktop-v0.10.0-windows-x86_64-setup.exe` | 6.5 MB |
| macOS (Apple Silicon) | `lrc-desktop-v0.10.0-macos-arm64.dmg` | 8.0 MB |
| Linux | `lrc-desktop-v0.10.0-linux-amd64.deb` | 9.3 MB |
| Linux (免安装) | `lrc-desktop-v0.10.0-linux-x86_64.AppImage` | 84 MB |

来源：`https://github.com/zhibaiYingChuan/LRC/releases/download/v0.10.0/<文件名>`

---

## 二、上传（二选一）

### 方式 A：服务器上直接拉（推荐，不用先下载到本地）

```bash
ssh <user>@sfang.cc
mkdir -p /var/www/sfang.cc/longyi/downloads
cd /var/www/sfang.cc/longyi/downloads

BASE=https://github.com/zhibaiYingChuan/LRC/releases/download/v0.10.0
for f in \
  lrc-desktop-v0.10.0-windows-x86_64-setup.exe \
  lrc-desktop-v0.10.0-macos-arm64.dmg \
  lrc-desktop-v0.10.0-linux-amd64.deb \
  lrc-desktop-v0.10.0-linux-x86_64.AppImage ; do
  curl -fL -o "$f" "$BASE/$f"
  curl -fL -o "$f.sha256" "$BASE/$f.sha256"
done

ls -lh
```

### 方式 B：本地下载后 scp

```powershell
$base = 'https://github.com/zhibaiYingChuan/LRC/releases/download/v0.10.0'
$files = @(
  'lrc-desktop-v0.10.0-windows-x86_64-setup.exe',
  'lrc-desktop-v0.10.0-macos-arm64.dmg',
  'lrc-desktop-v0.10.0-linux-amd64.deb',
  'lrc-desktop-v0.10.0-linux-x86_64.AppImage'
)
New-Item -ItemType Directory -Force .\downloads | Out-Null
foreach ($f in $files) { Invoke-WebRequest "$base/$f" -OutFile ".\downloads\$f" }
scp .\downloads\* <user>@sfang.cc:/var/www/sfang.cc/longyi/downloads/
```

---

## 三、nginx 配置（加在 longyi 的 server 块里）

```nginx
location /longyi/downloads/ {
    alias /var/www/sfang.cc/longyi/downloads/;
    autoindex off;
    add_header Content-Disposition "attachment";
    # 大文件断点续传
    add_header Accept-Ranges bytes;
    expires 7d;
}
```

重载：`sudo nginx -t && sudo nginx -s reload`

---

## 四、下载页 HTML（粘到 longyi 页面）

```html
<section class="downloads">
  <h2>下载 LRC v0.10.0</h2>
  <p class="sub">本地优先的 AI 代码记忆 · 数据不出本机</p>

  <div class="dl-grid">
    <a class="dl-card" href="/longyi/downloads/lrc-desktop-v0.10.0-windows-x86_64-setup.exe">
      <span class="dl-os">Windows</span>
      <span class="dl-file">.exe 安装包 · 6.5 MB</span>
    </a>
    <a class="dl-card" href="/longyi/downloads/lrc-desktop-v0.10.0-macos-arm64.dmg">
      <span class="dl-os">macOS</span>
      <span class="dl-file">.dmg · Apple Silicon · 8.0 MB</span>
    </a>
    <a class="dl-card" href="/longyi/downloads/lrc-desktop-v0.10.0-linux-amd64.deb">
      <span class="dl-os">Linux</span>
      <span class="dl-file">.deb · Debian/Ubuntu · 9.3 MB</span>
    </a>
    <a class="dl-card" href="/longyi/downloads/lrc-desktop-v0.10.0-linux-x86_64.AppImage">
      <span class="dl-os">Linux</span>
      <span class="dl-file">.AppImage · 免安装 · 84 MB</span>
    </a>
  </div>

  <p class="note">
    校验文件完整性：每个安装包旁都有 <code>.sha256</code> 文件。
    Windows 用 <code>certutil -hashfile &lt;文件&gt; SHA256</code>，
    macOS/Linux 用 <code>sha256sum &lt;文件&gt;</code>。
  </p>
  <p class="note">
    国内直连较慢时，可改用
    <a href="https://github.com/zhibaiYingChuan/LRC/releases/tag/v0.10.0">GitHub Release</a>。
  </p>
</section>
```

---

## 五、验证

```bash
# 1) 文件可下载
curl -I https://sfang.cc/longyi/downloads/lrc-desktop-v0.10.0-windows-x86_64-setup.exe
# 期望：HTTP/2 200，Content-Length 与源文件一致

# 2) 哈希一致
curl -sL https://sfang.cc/longyi/downloads/lrc-desktop-v0.10.0-windows-x86_64-setup.exe \
  | sha256sum
# 与 .sha256 文件里的值比对

# 3) 页面链接可点
curl -s https://sfang.cc/longyi | grep -c 'longyi/downloads/'
# 期望：4
```

---

## 六、需要你提供的信息

我这边没有服务器访问权限，无法直接上传。需要以下任一项：

| 方式 | 需要什么 |
|---|---|
| SSH | 主机、用户名、密钥路径（或密码） |
| 面板 | 宝塔/1Panel 等面板地址与账号 |
| 你自己执行 | 按上面「方式 A」在服务器上跑一遍即可 |

上传完成后告诉我，我来验证链接与哈希。

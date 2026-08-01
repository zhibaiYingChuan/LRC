// 同步 JS 表达式 - 测试 WebView2 CDP 连接
JSON.stringify({
  title: document.title,
  url: location.href,
  bodyLen: document.body && document.body.innerText ? document.body.innerText.length : 0,
  isDesktop: typeof window.__TAURI__ !== 'undefined' || typeof window.__TAURI_INTERNALS__ !== 'undefined'
})
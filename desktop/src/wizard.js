/**
 * LRC Desktop 配置向导
 *
 * 新流程（Agent 为中心）：
 * 1. 自动扫描所有 AI 工具 → 区分 IDE 型（多项目）和单应用型
 * 2. 对 IDE 型扫描项目列表 → 用户勾选 → 单应用直接配置
 * 3. 完成配置 + LLM 可选
 *
 * 契约：通过 Tauri invoke 调用 Rust 后端命令
 */
(function () {
  'use strict';

  // ── v0.5.8 新增：主题切换功能 ──
  // 支持浅色（Latte 宋韵雅色）和深色（Mocha 夜色）主题
  // 用户偏好保存在 localStorage，页面加载时自动恢复
  const THEME_STORAGE_KEY = 'lrc-theme';
  const THEME_LIGHT = 'light';
  const THEME_DARK = 'dark';

  /**
   * 初始化主题 — 页面加载时调用
   * 从 localStorage 读取用户偏好，未设置时默认浅色
   */
  function initTheme() {
    const savedTheme = localStorage.getItem(THEME_STORAGE_KEY) || THEME_LIGHT;
    applyTheme(savedTheme);
  }

  /**
   * 应用主题到 document 元素
   * @param {string} theme - 'light' 或 'dark'
   */
  function applyTheme(theme) {
    const toggleBtn = document.getElementById('theme-toggle');
    if (theme === THEME_DARK) {
      document.documentElement.setAttribute('data-theme', 'dark');
      if (toggleBtn) toggleBtn.textContent = '🌙';
    } else {
      document.documentElement.removeAttribute('data-theme');
      if (toggleBtn) toggleBtn.textContent = '☀️';
    }
  }

  /**
   * 切换主题（浅色 ↔ 深色）
   */
  function toggleTheme() {
    const currentTheme = document.documentElement.getAttribute('data-theme') === 'dark'
      ? THEME_DARK
      : THEME_LIGHT;
    const newTheme = currentTheme === THEME_DARK ? THEME_LIGHT : THEME_DARK;
    applyTheme(newTheme);
    localStorage.setItem(THEME_STORAGE_KEY, newTheme);
    console.log('[LRC] 主题已切换为:', newTheme);
  }

  // 页面加载时初始化主题
  initTheme();

  // ── 状态管理 ──
  let currentStep = 1;
  const config = {
    // Agent 检测结果
    allAgents: [],
    selectedAgents: [],       // 用户勾选的 Agent（含 IDE 和桌面应用）
    selectedProjects: [],     // 用户勾选的项目路径
    // LLM 配置
    llmProvider: 'deepseek',
    llmApiKey: null,
    llmModel: 'deepseek-chat',
    llmBaseUrl: 'https://api.deepseek.com/v1',
    ollamaModel: 'llama3',
    ollamaUrl: 'http://localhost:11434',
    // 多窗口 LRC 记录
    multiWindowEnabled: true, // 同一项目多窗口同时记录（上限 5 个，默认开启）
    // Sidecar 端口
    port: 3099,
    // v0.5.7 新增：保存 sidecar 启动失败的错误信息，用于在摘要页展示给用户
    lastSidecarError: null,
  };

  // ── 工具函数 ──
  const $ = (id) => document.getElementById(id);

  /**
   * HTML 转义函数 — 防止 XSS 注入
   * 将所有用户输入和后端数据在插入 HTML 前进行转义
   * 使用字符映射表确保所有特殊字符被正确转义
   */
  const HTML_ESCAPE_MAP = {
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
    '/': '&#47;',
  };
  function escapeHtml(str) {
    if (str == null || typeof str !== 'string') return '';
    return str.replace(/[&<>"'/]/g, (ch) => HTML_ESCAPE_MAP[ch] || ch);
  }

  /**
   * 安全设置 innerHTML — 仅用于纯静态 HTML
   * 如果内容包含动态数据，必须先通过 escapeHtml 转义
   */
  function safeSetHtml(el, html) {
    if (typeof el === 'string') el = $(el);
    if (el) el.innerHTML = html;
  }

  async function tauriInvoke(cmd, args = {}) {
    if (window.__TAURI_INVOKE__) {
      return window.__TAURI_INVOKE__(cmd, args);
    }
    // v0.5.4 修复：Tauri v2 全局变量是 __TAURI__，不是 __TAURI_INTERNALS__（v1）
    // 需要 tauri.conf.json 中设置 "withGlobalTauri": true
    if (window.__TAURI__?.core?.invoke) {
      return window.__TAURI__.core.invoke(cmd, args);
    }
    // 兼容旧版本 Tauri v1
    if (window.__TAURI_INTERNALS__?.invoke) {
      return window.__TAURI_INTERNALS__.invoke(cmd, args);
    }
    console.warn('[LRC] 非 Tauri 环境，使用 HTTP fallback');
    return null;
  }

  /**
   * v0.5.7 新增：带超时的 tauriInvoke 包装
   *
   * 解决问题：某些后端命令（如 detect_agents、start_sidecar）可能因
   * 文件 I/O 或进程启动耗时较长，导致前端无限等待。
   *
   * @param {string} cmd - 命令名
   * @param {object} args - 命令参数
   * @param {number} timeoutMs - 超时毫秒数（默认 15 秒）
   * @returns {Promise<any>} 命令结果，超时后 reject
   */
  async function tauriInvokeWithTimeout(cmd, args = {}, timeoutMs = 15000) {
    let timerId = null;
    const timeoutPromise = new Promise((_, reject) => {
      timerId = setTimeout(() => reject(new Error(`命令 ${cmd} 超时（${timeoutMs}ms）`)), timeoutMs);
    });
    const invokePromise = tauriInvoke(cmd, args);
    try {
      return await Promise.race([invokePromise, timeoutPromise]);
    } finally {
      // 审计修复：无论成功还是超时，都清理定时器，避免悬挂的 reject 调用
      if (timerId) clearTimeout(timerId);
    }
  }

  /**
   * v0.5.4 新增：Tauri 事件监听辅助函数
   * 
   * 与 tauriInvoke 类似，优先使用 window.__TAURI_LISTEN__（dev 环境导入），
   * 回退到 __TAURI_INTERNALS__.listen（Release 环境）。
   * 返回 unlisten 函数，调用后可取消监听。
   */
  async function tauriListen(event, callback) {
    if (window.__TAURI_LISTEN__) {
      return window.__TAURI_LISTEN__(event, callback);
    }
    // v0.5.4 修复：Tauri v2 全局变量是 __TAURI__，不是 __TAURI_INTERNALS__（v1）
    if (window.__TAURI__?.event?.listen) {
      return window.__TAURI__.event.listen(event, callback);
    }
    // 兼容旧版本 Tauri v1
    if (window.__TAURI_INTERNALS__?.listen) {
      return window.__TAURI_INTERNALS__.listen(event, callback);
    }
    console.warn('[LRC] 事件监听不可用，跳过:', event);
    return null;
  }

  // ── 步骤导航 ──
  function goToStep(step) {
    // v0.5.4 P0-3 修复：更新进度条
    const progressBar = $('progress-bar-fill');
    if (progressBar) {
      if (step === 'done' || step === 3) {
        progressBar.style.width = '100%';
      } else if (step === 2) {
        progressBar.style.width = '66%';
      } else {
        progressBar.style.width = '33%';
      }
    }

    // v0.5.4 修复：特殊处理 'done' 字符串，parseInt('done') 返回 NaN
    if (step === 'done' || step === 3) {
      document.querySelectorAll('.wizard-panel').forEach(p => p.classList.remove('active'));
      const donePanel = $('panel-step-done');
      if (donePanel) donePanel.classList.add('active');
      // 所有步骤标记为已完成
      document.querySelectorAll('.step').forEach(s => s.classList.add('done'));
      document.querySelectorAll('.step').forEach(s => s.classList.remove('active'));
      document.querySelectorAll('.step-connector').forEach(c => c.classList.add('done'));
      currentStep = 'done';
      return;
    }

    document.querySelectorAll('.wizard-panel').forEach((p) => p.classList.remove('active'));
    document.querySelectorAll('.step').forEach((s) => {
      const sNum = parseInt(s.dataset.step);
      s.classList.remove('active', 'done');
      if (sNum < step) s.classList.add('done');
      if (sNum === step) s.classList.add('active');
    });
    document.querySelectorAll('.step-connector').forEach((c, i) => {
      c.classList.toggle('done', i < step - 1);
    });

    const panel = $(`panel-step-${step}`);
    if (panel) panel.classList.add('active');
    currentStep = step;

    // v0.5.4 P0-3 修复：步骤 2 初始化 LLM 配置表单
    if (step === 2) initWizardLlmForm();
  }

  // ── 步骤 1：自动检测 Agent ──
  async function loadAgents() {
    const listEl = $('agent-list');
    let progressUnlisten = null;

    // v0.5.4 修复：监听后端进度事件，显示"正在检测 Trae... (3/22)"
    try {
      progressUnlisten = await tauriListen('agent-detect-progress', (event) => {
        const payload = event.payload;
        if (payload && listEl) {
          const safeName = escapeHtml(payload.name || '');
          listEl.innerHTML = `<div class="loading">正在检测 ${safeName}... (${payload.current}/${payload.total})</div>`;
        }
      });
    } catch (e) {
      // 事件监听失败不阻断主流程（非 Tauri 环境或旧版后端）
      console.warn('[LRC] 进度事件监听失败，使用兜底显示:', e);
    }

    // 初始显示
    listEl.innerHTML = '<div class="loading">正在扫描 AI 工具...</div>';

    try {
      // v0.5.7 修复：使用带超时的 invoke，避免卡在"正在扫描 AI 工具..."
      let agents = await tauriInvokeWithTimeout('detect_agents', {}, 10000);
      // v0.5.4 修复：检测完成后取消进度监听，避免内存泄漏
      if (progressUnlisten) {
        try { progressUnlisten(); } catch (e) { /* 忽略取消监听错误 */ }
      }
      if (!agents || agents.length === 0) {
        // P1-03 修复：不再显示硬编码的 7 种 fallback 列表
        // 改为提示用户检查后端连接状态
        listEl.innerHTML = '<div class="error-message">无法获取 AI 工具列表。<br>请确认 LRC Desktop 后端服务已启动。</div>';
        $('btn-step-1-next').disabled = true;
        return;
      }
      config.allAgents = agents;

      // 按分类分组渲染
      const categories = [
        { key: 'ide', title: 'IDE 内嵌 AI 工具（可管理多个项目）', desc: '勾选后扫描项目列表' },
        { key: 'desktop', title: '独立桌面应用', desc: '勾选后直接配置 MCP 连接' },
        { key: 'ai-assistant', title: 'AI 助手类工具', desc: '勾选后配置 MCP 连接' },
        { key: 'custom', title: '自定义', desc: '提供通用连接信息' },
      ];

      let html = '';
      let hasInstalled = false;

      for (const cat of categories) {
        const catAgents = agents.filter((a) => a.category === cat.key);
        if (catAgents.length === 0) continue;

        const installedInCat = catAgents.filter((a) => a.installed);
        if (installedInCat.length === 0) {
          // 该分类没有已安装的 Agent，折叠显示
          html += `<details class="agent-category-details">
            <summary>${cat.title} <span class="cat-count">(${installedInCat.length}/${catAgents.length} 已安装)</span></summary>`;
        } else {
          html += `<div class="agent-category-header">${cat.title} <span class="cat-desc">— ${cat.desc}</span></div>`;
        }

        for (const agent of catAgents) {
          const installed = agent.installed;
          if (installed) hasInstalled = true;
          const isIDE = cat.key === 'ide';
          const safeId = escapeHtml(agent.id);
          const safeName = escapeHtml(agent.name);
          const safeIcon = escapeHtml(agent.icon || '');
          // v0.5.7 修复：只有支持 MCP 的已安装工具才自动勾选，与 syncInitialAgentSelection 保持一致
          const shouldCheck = installed && agent.supports_mcp;
          html += `
            <label class="agent-item ${installed ? '' : 'disabled'}">
              <input type="checkbox" value="${safeId}"
                data-category="${escapeHtml(cat.key)}"
                ${shouldCheck ? 'checked' : ''}
                ${!installed ? 'disabled' : ''}>
              <span class="agent-icon">${safeIcon}</span>
              <div class="agent-info">
                <span class="agent-name">${safeName}</span>
                ${isIDE && installed ? '<span class="ide-badge">含项目列表</span>' : ''}
                ${installed && !agent.supports_mcp ? '<span class="ide-badge" style="background:var(--text-tertiary);">不支持 MCP</span>' : ''}
              </div>
              <span class="agent-status ${installed ? 'installed' : 'not-installed'}">
                ${installed ? '已安装' : '未安装'}
              </span>
            </label>`;
        }

        if (installedInCat.length === 0) {
          html += '</details>';
        }
      }

      listEl.innerHTML = html;
      // v0.5.4 修复：事件委托替代全局命名空间，代理 checkbox 变更
      listEl.addEventListener('change', function (ev) {
        const cb = ev.target;
        if (cb && cb.tagName === 'INPUT' && cb.type === 'checkbox') {
          const id = cb.value;
          if (cb.checked) {
            if (!config.selectedAgents.includes(id)) {
              config.selectedAgents.push(id);
            }
          } else {
            config.selectedAgents = config.selectedAgents.filter((a) => a !== id);
          }
        }
      });
      syncInitialAgentSelection();  // 同步初始勾选状态
      $('btn-step-1-next').disabled = !hasInstalled;

      // v0.5.4 P0-3 修复：Agent 检测完成后，自动加载项目列表
      if (hasInstalled) {
        loadProjectsInStep1();
      }

      if (hasInstalled) {
        $('btn-step-1-next').textContent = '下一步：配置 LLM（可选）→';
      } else {
        listEl.innerHTML += '<div class="no-agents">未检测到已安装的 AI 工具。<br>请先安装 Trae、Cursor、VS Code 等 IDE，或 Claude Desktop、Gemini CLI 等 AI 工具。</div>';
      }
    } catch (e) {
      // v0.5.4 修复：异常时也取消进度监听
      if (progressUnlisten) {
        try { progressUnlisten(); } catch (ue) { /* 忽略取消监听错误 */ }
      }
      const errMsg = (e && typeof e === 'string') ? e : (e?.message || '检测失败');
      // v0.5.7 修复：超时后恢复 UI 状态 + 添加重试按钮
      // 之前超时后按钮仍 disabled，用户无法操作，必须重启应用
      const isTimeout = errMsg.includes('超时') || errMsg.includes('timeout');
      const hint = isTimeout
        ? '检测超时，可能是杀毒软件扫描或系统响应慢。可点击下方按钮重试，或重启应用。'
        : '请确认 LRC 后端服务已启动，或重启应用后重试。';
      listEl.innerHTML = `<div class="error-message">
        <strong>AI 工具检测失败</strong><br>
        ${escapeHtml(errMsg)}<br>
        <small style="color:var(--text-secondary);">${hint}</small>
      </div>
      <button id="btn-retry-detect" class="btn btn-secondary" style="margin-top:12px;">重新检测</button>`;
      // 恢复按钮状态：禁用下一步，但启用重试按钮
      const nextBtn = $('btn-step-1-next');
      if (nextBtn) nextBtn.disabled = true;
      const retryBtn = $('btn-retry-detect');
      if (retryBtn) {
        retryBtn.addEventListener('click', () => {
          if (retryBtn) retryBtn.disabled = true;
          retryBtn.textContent = '正在重新检测...';
          loadAgents();
        });
      }
    }
  }

  // v0.5.4 P0-3 新增：在步骤 1 中加载项目列表（Agent 检测完成后触发）
  async function loadProjectsInStep1() {
    const projectSection = $('project-section');
    const listEl = $('project-list');
    if (!projectSection || !listEl) return;

    projectSection.style.display = 'block';
    listEl.innerHTML = '<div class="loading">正在扫描项目...</div>';

    // 区分 IDE 和桌面应用
    const ideAgents = config.selectedAgents.filter((id) => {
      const agent = config.allAgents.find((a) => a.id === id);
      return agent && agent.category === 'ide';
    });

    let html = '';

    // 扫描 IDE 项目
    if (ideAgents.length > 0) {
      try {
        // v0.5.7 修复：使用超时包装，避免 scan_ide_projects 卡住
        const projects = await tauriInvokeWithTimeout('scan_ide_projects', { ideIds: ideAgents }, 15000);
        if (projects && projects.length > 0) {
          const grouped = {};
          for (const p of projects) {
            if (!grouped[p.ide_id]) grouped[p.ide_id] = { name: p.ide_name, projects: [] };
            grouped[p.ide_id].projects.push(p);
          }

          for (const [ideId, group] of Object.entries(grouped)) {
            const safeIdeId = escapeHtml(ideId);
            const safeGroupName = escapeHtml(group.name);
            html += `<div class="project-group">
              <div class="project-group-header">${safeGroupName} 项目 (${group.projects.length})</div>`;
            for (const p of group.projects) {
              const safePath = escapeHtml(p.path);
              const safeName = escapeHtml(p.name);
              html += `
                <label class="project-item">
                  <input type="checkbox" value="${safePath}" checked
                    data-ide="${safeIdeId}">
                  <div class="project-info">
                    <span class="project-name">${safeName}</span>
                    <span class="project-path">${safePath}</span>
                  </div>
                </label>`;
            }
            html += '</div>';
          }
        } else {
          html += `<div class="no-projects">
            <p>未扫描到 IDE 项目，请使用上方「选择文件夹」按钮手动添加。</p>
          </div>`;
        }
      } catch (e) {
        html += `<div class="error-message">项目扫描失败：${escapeHtml(e.message || String(e))}</div>`;
      }
    } else {
      html += `<div class="no-projects">
        <p>未选择 IDE 类型的 AI 工具，请使用上方「选择文件夹」按钮手动添加项目。</p>
      </div>`;
    }

    listEl.innerHTML = html;

    // 事件委托：代理项目 checkbox 变更
    listEl.addEventListener('change', function (ev) {
      const cb = ev.target;
      if (cb && cb.tagName === 'INPUT' && cb.type === 'checkbox') {
        const path = cb.value;
        if (cb.checked) {
          if (!config.selectedProjects.includes(path)) {
            config.selectedProjects.push(path);
          }
        } else {
          config.selectedProjects = config.selectedProjects.filter((p) => p !== path);
        }
      }
    });

    // 同步项目初始勾选状态
    syncInitialProjectSelection();
  }

  // v0.5.4 P0-3 新增：步骤 1 中选择文件夹按钮
  const btnPickFolder1 = $('btn-step-1-pick-folder');
  if (btnPickFolder1) {
    btnPickFolder1.addEventListener('click', async () => {
      btnPickFolder1.disabled = true;
      btnPickFolder1.textContent = '正在打开文件夹选择器...';
      try {
        const dir = await tauriInvoke('pick_project_dir');
        if (dir) {
          addManualProjectToStep1(dir);
          const pickedEl = $('step-1-picked-folder');
          if (pickedEl) {
            pickedEl.textContent = '已选择: ' + dir;
            pickedEl.style.display = 'inline';
          }
          console.log('[配置向导] 用户选择了文件夹:', dir);
        }
      } catch (e) {
        console.warn('[配置向导] 选择文件夹失败:', e);
      }
      btnPickFolder1.disabled = false;
      btnPickFolder1.textContent = '📁 选择文件夹...';
    });
  }

  function addManualProjectToStep1(dir) {
    config.selectedProjects.push(dir);
    const safeDir = escapeHtml(dir);
    const safeName = escapeHtml(dir.split('\\').pop() || dir);
    const listEl = $('project-list');
    const html = `
      <label class="project-item">
        <input type="checkbox" value="${safeDir}" checked>
        <div class="project-info">
          <span class="project-name">${safeName} (手动添加)</span>
          <span class="project-path">${safeDir}</span>
        </div>
      </label>`;
    listEl.insertAdjacentHTML('beforeend', html);
  }

  // 同步项目初始勾选状态
  function syncInitialProjectSelection() {
    const checkboxes = document.querySelectorAll('.project-item input[type="checkbox"]:checked');
    checkboxes.forEach((cb) => {
      if (!config.selectedProjects.includes(cb.value)) {
        config.selectedProjects.push(cb.value);
      }
    });
  }

  // v0.5.4 修复：_wizToggleAgent 已由事件委托替代，移除全局命名空间污染

  // 初始化：从 Agent 数据直接同步 selectedAgents（不依赖 DOM 状态）
  function syncInitialAgentSelection() {
    // v0.5.7 修复：只自动选中支持 MCP 的已安装工具
    // 根因：之前所有 installed=true 的工具都被选中，包括不支持 MCP 的工具
    // （如通义灵码、豆包 MarsCode 等），导致底部状态栏显示"Agent 5 个"
    // 但实际只有 2 个支持 MCP 的 AI 工具能被配置
    config.allAgents.forEach((agent) => {
      if (agent.installed && agent.supports_mcp && !config.selectedAgents.includes(agent.id)) {
        config.selectedAgents.push(agent.id);
      }
    });
  }

  $('btn-step-1-next').addEventListener('click', async () => {
    if (config.selectedAgents.length > 0) {
      // v0.5.4 P0-3 修复：Agent 检测完成后立即配置 MCP，然后进入 LLM 配置步骤
      const btn = $('btn-step-1-next');
      btn.disabled = true;
      btn.textContent = '正在配置 MCP...';
      try {
        // v0.5.7 修复：使用超时包装，避免 configure_agents 卡住
        const result = await tauriInvokeWithTimeout('configure_agents', {
          agentIds: config.selectedAgents,
          port: config.port || 3099,
        }, 30000);
        console.log('[配置向导] MCP 配置完成:', result);
        // v0.5.4 P2-18 修复：保存 MCP 配置结果，供 showSummary 显示
        config.mcpConfigResult = result;
      } catch (e) {
        console.warn('[配置向导] MCP 配置失败（非致命）:', e);
        config.mcpConfigResult = null;
      }
      btn.disabled = false;
      btn.textContent = '下一步：配置 LLM（可选）→';
      goToStep(2);
    }
  });

  // v0.5.4 P0-3 修复：跳过 Agent 检测，直接进入 LLM 配置步骤
  // 用户嫌检测太慢时，可以走这条快速路径
  $('btn-step-1-skip').addEventListener('click', async () => {
    console.log('[配置向导] 用户选择跳过 Agent 检测，进入快速路径');
    // 标记步骤1为已完成（视觉上）
    document.querySelectorAll('.wizard-steps .step').forEach((s, i) => {
      if (i === 0) s.classList.add('done');
      if (i === 1) s.classList.add('active');
    });
    // 更新进度条
    const progressBar = $('progress-bar-fill');
    if (progressBar) progressBar.style.width = '66%';
    // 后台静默加载 Agent 列表（供后续自动配置 MCP 使用）
    try {
      // v0.5.7 修复：跳过路径也使用超时包装
      const agents = await tauriInvokeWithTimeout('detect_agents', {}, 10000);
      if (agents) {
        config.allAgents = agents;
        // v0.5.7 修复：只自动选中支持 MCP 的已安装工具（与 syncInitialAgentSelection 一致）
        agents.filter(a => a.installed && a.supports_mcp).forEach(a => {
          if (!config.selectedAgents.includes(a.id)) {
            config.selectedAgents.push(a.id);
          }
        });
        console.log('[配置向导] 后台检测完成，已安装且支持 MCP 的 Agent:', config.selectedAgents.length, '个');

        // v0.5.4 P2-18 修复：跳过检测路径也必须调用 configure_agents 写入 MCP 配置
        // 修复前：此路径不调用 configure_agents，导致 MCP 配置从未写入，用户在 AI 工具中看不到 LRC
        if (config.selectedAgents.length > 0) {
          try {
            const port = config.port || 3099;
            // v0.5.7 修复：跳过路径也使用超时包装
            const result = await tauriInvokeWithTimeout('configure_agents', {
              agentIds: config.selectedAgents,
              port: port,
            }, 30000);
            console.log('[配置向导] MCP 配置完成（跳过检测路径）:', result);
            // 保存 MCP 配置结果，供 showSummary 显示
            config.mcpConfigResult = result;
          } catch (e) {
            console.warn('[配置向导] MCP 配置失败（非致命，跳过检测路径）:', e);
            config.mcpConfigResult = null;
          }
        }
      }
    } catch (e) {
      console.warn('[配置向导] 后台 Agent 检测失败（非致命）:', e);
    }
    goToStep(2);
  });

  // ── v0.5.4 P0-3 修复：步骤 2 改为 LLM 配置（可选）──
  // 初始化 LLM 配置表单（同步当前 config 状态）
  function initWizardLlmForm() {
    const providerEl = $('wizard-llm-provider');
    if (!providerEl) return;

    // 同步提供商选择
    providerEl.value = config.llmProvider || 'deepseek';
    updateWizardLlmProviderUI();

    // 同步已填写的 API Key 和模型
    if (config.llmApiKey) {
      const keyEl = $('wizard-llm-api-key');
      if (keyEl) keyEl.value = config.llmApiKey;
    }
    if (config.llmModel) {
      const modelEl = $('wizard-llm-model');
      if (modelEl) modelEl.value = config.llmModel;
    }
  }

  // 提供商切换时更新 UI
  function updateWizardLlmProviderUI() {
    const provider = $('wizard-llm-provider')?.value;
    const isOllama = provider === 'ollama';

    const apiSection = $('wizard-llm-api-section');
    if (apiSection) apiSection.style.display = isOllama ? 'none' : 'block';

    const ollamaFields = $('wizard-ollama-fields');
    if (ollamaFields) ollamaFields.style.display = isOllama ? 'block' : 'none';

    // 更新 Key 链接
    const providerInfo = LLM_PROVIDERS[provider];
    const keyLink = $('wizard-llm-key-link');
    if (keyLink && providerInfo) {
      // v0.5.7 修复：Ollama 和 custom 无需 API Key，隐藏获取链接
      if (providerInfo.keyUrl) {
        keyLink.href = providerInfo.keyUrl;
        keyLink.textContent = `获取 ${providerInfo.name} API Key →`;
        keyLink.style.display = 'inline';
      } else {
        keyLink.style.display = 'none';
      }
    }

    // v0.5.7 修复：根据提供商更新模型名称 placeholder
    const modelEl = $('wizard-llm-model');
    if (modelEl && providerInfo) {
      modelEl.placeholder = providerInfo.model ? `例如：${providerInfo.model}` : '输入模型名称';
    }
  }

  // 提供商下拉切换事件
  const wizardLlmProvider = $('wizard-llm-provider');
  if (wizardLlmProvider) {
    wizardLlmProvider.addEventListener('change', updateWizardLlmProviderUI);
  }

  // 步骤 2 上一步按钮
  $('btn-step-2-prev').addEventListener('click', () => goToStep(1));

  // 步骤 2 跳过按钮
  $('btn-step-2-skip').addEventListener('click', async () => {
    console.log('[配置向导] 用户跳过 LLM 配置');
    await finishConfiguration();
  });

  // 步骤 2 保存并完成按钮
  $('btn-step-2-next').addEventListener('click', async () => {
    const btn = $('btn-step-2-next');
    btn.disabled = true;
    btn.textContent = '正在保存...';

    // 保存 LLM 配置
    const provider = $('wizard-llm-provider')?.value || 'deepseek';
    try {
      let llmString = '';
      if (provider === 'ollama') {
        const ollamaModel = $('wizard-ollama-model')?.value || 'llama3';
        const ollamaUrl = $('wizard-ollama-url')?.value || 'http://localhost:11434';
        // v0.5.7 修复：使用 || 分隔符（与后端 to_llm_api_string 保持一致）
        llmString = `ollama||${ollamaModel}||${ollamaUrl}`;
        config.llmProvider = 'ollama';
        config.llmModel = ollamaModel;
        config.ollamaUrl = ollamaUrl;
      } else {
        const apiKey = ($('wizard-llm-api-key')?.value || '').trim();
        if (apiKey) {
          const model = $('wizard-llm-model')?.value || LLM_PROVIDERS[provider]?.model || '';
          const baseUrl = LLM_PROVIDERS[provider]?.url || '';
          // v0.5.7 修复：使用 || 分隔符（支持 API Key 中包含冒号）
          llmString = `openai||${apiKey}||${model}||${baseUrl}`;
          config.llmApiKey = apiKey;
          config.llmModel = model;
          config.llmBaseUrl = baseUrl;
          config.llmProvider = provider;
        }
      }

      if (llmString) {
        await tauriInvokeWithTimeout('save_llm_config', { llmApi: llmString }, 5000);
        console.log('[配置向导] LLM 配置已保存');
      }
    } catch (e) {
      console.warn('[配置向导] LLM 配置保存失败（非致命）:', e);
    }

    // v0.5.7 修复：不再提前恢复按钮状态，由 finishConfiguration() 统一管理
    await finishConfiguration();
  });

  // v0.5.4 P0-3 新增：统一的完成配置流程（LLM 保存后或跳过 LLM 都走这里）
  // v0.5.7 修复：添加 loading 指示器 + 超时包装 + 全局规则写入保障
  async function finishConfiguration() {
    const btn = $('btn-step-2-next');
    const skipBtn = $('btn-step-2-skip');
    if (btn) { btn.disabled = true; btn.textContent = '正在配置...'; }
    if (skipBtn) skipBtn.disabled = true;

    // v0.5.7 新增：显示配置进度提示
    const progressEl = $('config-progress');
    const showProgress = (msg) => {
      if (progressEl) {
        progressEl.textContent = msg;
        progressEl.style.display = 'block';
      }
    };

    // 保存项目目录
    showProgress('正在保存项目目录...');
    if (config.selectedProjects.length > 0) {
      try { await tauriInvokeWithTimeout('set_project_dir', { projectDir: config.selectedProjects[0] }, 5000); } catch (e) {
        console.warn('[配置向导] 项目目录设置失败:', e);
      }
    }

    // v0.5.7 修复：先启动 sidecar 获取实际端口，再用实际端口配置 MCP
    // 修复前：configure_agents 在 sidecar 启动前调用，使用默认端口 3099
    // 如果 sidecar 实际使用其他端口（如 3100+），MCP 配置会写入错误端口
    showProgress('正在启动后台服务...');
    let sidecarStarted = false;
    const actualPort = await startSidecarWithConfig(
      config.selectedProjects[0] || null,
      config.port || null,
      config.multiWindowEnabled
    );
    if (actualPort) {
      sidecarStarted = true;
      config.port = actualPort; // 保存实际端口供后续使用
    }

    // v0.5.6 修复：调用 configure_agents 确保全局规则文件写入
    // v0.5.7 修复：使用 sidecar 实际端口配置 MCP，避免端口不匹配
    if (config.selectedAgents.length > 0) {
      showProgress('正在配置 MCP 连接和全局规则...');
      try {
        const portForConfig = actualPort || config.port || 3099;
        await tauriInvokeWithTimeout('configure_agents', {
          agentIds: config.selectedAgents,
          port: portForConfig,
        }, 30000); // configure_agents 涉及文件写入，给 30 秒
        console.log('[配置向导] 配置 Agent 完成（MCP 配置 + 全局规则文件已写入，端口:', portForConfig, '）');
      } catch (e) {
        console.warn('[配置向导] 配置 Agent 失败（非致命，规则文件可能未写入）:', e);
      }
    }

    // v0.5.4 P0-4 新增：调用后端验证命令，获取完整验证结果
    let verifyResult = null;
    try {
      // 等待 sidecar 完全就绪
      showProgress('正在验证配置...');
      await new Promise(r => setTimeout(r, 1500));
      verifyResult = await tauriInvokeWithTimeout('verify_setup', {}, 10000);
      console.log('[配置向导] 验证结果:', verifyResult);
    } catch (e) {
      console.warn('[配置向导] 验证调用失败（非致命）:', e);
    }

    // v0.5.4 P2-16 修复：标记配置完成，持久化 setup_complete=true
    try {
      await tauriInvokeWithTimeout('mark_complete', {}, 5000);
      console.log('[配置向导] 已标记配置完成');
    } catch (e) {
      console.warn('[配置向导] 标记完成失败（非致命）:', e);
    }

    // 隐藏进度提示
    if (progressEl) progressEl.style.display = 'none';
    if (btn) { btn.disabled = false; btn.textContent = '保存并完成配置 →'; }
    if (skipBtn) skipBtn.disabled = false;

    showSummary(sidecarStarted, verifyResult);
    goToStep(3);
    pollStatus();
    updateStatusBar(null);
  }

  // ── LLM 配置（傻瓜化一键配置）──
  // 常见提供商的 API 地址、默认模型、获取 Key 的链接
  // v0.5.1 重构：所有 LLM 配置统一在设置面板中完成，向导仅保留此常量供设置面板使用
  const LLM_PROVIDERS = {
    // ── 国产优先 ──
    deepseek:   { name: 'DeepSeek',       url: 'https://api.deepseek.com/v1',           model: 'deepseek-chat',       keyUrl: 'https://platform.deepseek.com/api_keys',     keyHint: 'sk-...',    desc: '国产性价比之王，代码能力极强' },
    qwen:       { name: '通义千问',       url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen-plus', keyUrl: 'https://bailian.console.aliyun.com/',     keyHint: 'sk-...',    desc: '阿里云出品，中文理解出色' },
    zhipu:      { name: '智谱 GLM',       url: 'https://open.bigmodel.cn/api/paas/v4',   model: 'glm-4',               keyUrl: 'https://open.bigmodel.cn/usercenter/apikeys', keyHint: 'xxx.xxx', desc: '清华系，GLM 系列模型' },
    minimax:    { name: 'MiniMax',         url: 'https://api.minimax.chat/v1',            model: 'abab6.5s-chat',       keyUrl: 'https://platform.minimax.com/user-center/basic-information/interface-key', keyHint: 'eyJ...', desc: '海螺AI同款，长文本支持好' },
    moonshot:   { name: 'Moonshot (Kimi)', url: 'https://api.moonshot.cn/v1',            model: 'moonshot-v1-8k',      keyUrl: 'https://platform.moonshot.cn/console/api-keys',  keyHint: 'sk-...', desc: 'Kimi 同款，超长上下文' },
    bytedance:  { name: '豆包 (ByteDance)',url: 'https://ark.cn-beijing.volces.com/api/v3', model: 'doubao-pro-32k', keyUrl: 'https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey', keyHint: '...', desc: '字节跳动出品，性价比高' },
    stepfun:    { name: '阶跃星辰',       url: 'https://api.stepfun.com/v1',              model: 'step-1-8k',          keyUrl: 'https://platform.stepfun.com/',                keyHint: 'sk-...',    desc: 'Step 系列，多模态能力强' },
    baichuan:   { name: '百川智能',       url: 'https://api.baichuan-ai.com/v1',          model: 'Baichuan4',          keyUrl: 'https://platform.baichuan-ai.com/',             keyHint: 'sk-...',    desc: '百川大模型，金融医疗领域强' },
    // ── 国际厂商 ──
    openai:     { name: 'OpenAI',          url: 'https://api.openai.com/v1',              model: 'gpt-4o',             keyUrl: 'https://platform.openai.com/api-keys',         keyHint: 'sk-...',    desc: 'GPT-4o，综合能力最强' },
    // ── 本地/自定义 ──
    ollama:     { name: 'Ollama 本地模型', url: 'http://localhost:11434',                 model: 'llama3',             keyUrl: null,         keyHint: '无需 Key（本地运行）', desc: '免费本地运行，数据不出电脑' },
    custom:     { name: '自定义 API',      url: '',                                        model: '',                   keyUrl: null,         keyHint: '',          desc: '手动填写任何兼容 OpenAI 的 API 地址' },
  };

  /**
   * v0.5.1 重构：提取统一的 sidecar 启动辅助函数
   * 
   * @param {string} srcDir - 项目目录路径
   * @param {number|null} port - 端口号
   * @param {boolean} multiWindow - 是否启用多窗口记录
   * @returns {Promise<number|null>} 返回端口号，失败返回 null
   */
  async function startSidecarWithConfig(srcDir, port, multiWindow) {
    try {
      // v0.5.7 修复：使用超时包装，避免 start_sidecar 卡住（含健康检查，给 60 秒）
      const actualPort = await tauriInvokeWithTimeout('start_sidecar', {
        srcDir: srcDir || null,
        port: port || null,
        multiWindow: multiWindow ? 5 : 1,
      }, 60000);
      if (actualPort) {
        config.port = actualPort;
        config.lastSidecarError = null;  // 清除上次错误
        console.log('[配置向导] Sidecar 已启动，端口:', actualPort);
        return actualPort;
      }
      config.lastSidecarError = 'Sidecar 启动后未返回端口号';
      return null;
    } catch (e) {
      // v0.5.7 修复：保存后端返回的错误信息，用于在摘要页展示给用户
      // 之前只 console.warn 会导致用户看到 hardcoded 的笼统文案
      const msg = (typeof e === 'string') ? e : (e?.message || '启动失败');
      console.warn('[配置向导] Sidecar 启动失败:', msg);
      config.lastSidecarError = msg;
      return null;
    }
  }

  // v0.5.4 P0-3 修复：btn-step-2-prev 和 btn-step-2-finish 已在上方新步骤 2 逻辑中重新绑定

  // ── 完成页「配置 LLM 和 Agent」按钮 ──
  $('btn-open-settings-from-done')?.addEventListener('click', () => {
    showSettingsPanel();
  });

  // ── 完成页 ──
  /** v0.5.4 P0-4 增强：显示配置摘要 + 验证结果 + 首次记忆引导 */
  function showSummary(sidecarStarted, verifyResult) {
    const agentNames = {};
    config.allAgents.forEach((a) => { agentNames[a.id] = a.name; });

    const agentList = config.selectedAgents
      .map((id) => escapeHtml(agentNames[id] || id))
      .join(', ');

    const projectList = config.selectedProjects
      .map((p) => escapeHtml(p.split('\\').pop()))
      .join(', ');

    const safePort = escapeHtml(String(config.port));
    const safeModel = escapeHtml(config.llmModel || '');

    // v0.5.4 P0-4 增强：根据 verify_setup 结果生成验证卡片
    let verifyHtml = '';
    if (verifyResult) {
      const allOk = verifyResult.all_ok;
      verifyHtml = `<div class="summary-banner ${allOk ? 'success' : 'error'}">
          <span class="banner-icon">${allOk ? '&#x2705;' : '&#x26A0;&#xFE0F;'}</span>
          <div>
            <strong>${allOk ? '配置验证通过！' : '配置验证发现问题'}</strong>
            <span style="font-size:12px;color:${allOk ? 'var(--text-secondary)' : 'var(--text-tertiary)'};">${escapeHtml(verifyResult.suggestion || '')}</span>
          </div>
        </div>
        <div class="verify-checklist">
          <div class="verify-item ${verifyResult.sidecar_running ? 'ok' : 'fail'}">
            <span class="verify-icon">${verifyResult.sidecar_running ? '&#x2705;' : '&#x274C;'}</span>
            <span>${escapeHtml(verifyResult.sidecar_message || '')}</span>
          </div>
          <div class="verify-item ${verifyResult.agents_configured ? 'ok' : 'fail'}">
            <span class="verify-icon">${verifyResult.agents_configured ? '&#x2705;' : '&#x274C;'}</span>
            <span>${escapeHtml(verifyResult.agents_message || '')}</span>
          </div>
          <div class="verify-item ${verifyResult.llm_configured ? 'ok' : 'info'}">
            <span class="verify-icon">${verifyResult.llm_configured ? '&#x2705;' : '&#x2139;&#xFE0F;'}</span>
            <span>${escapeHtml(verifyResult.llm_message || '')}</span>
          </div>
          <div class="verify-item ${verifyResult.project_configured ? 'ok' : 'fail'}">
            <span class="verify-icon">${verifyResult.project_configured ? '&#x2705;' : '&#x274C;'}</span>
            <span>${escapeHtml(verifyResult.project_message || '')}</span>
          </div>
        </div>`;
    } else {
      // 回退：无验证结果时使用旧版横幅
      verifyHtml = sidecarStarted
        ? `<div class="summary-banner success">
            <span class="banner-icon">&#x2705;</span>
            <div>
              <strong>后台服务启动成功！</strong>
              <span style="font-size:12px;color:var(--text-secondary);">LRC 服务运行在端口 ${safePort}</span>
            </div>
          </div>`
        : `<div class="summary-banner error">
            <span class="banner-icon">&#x26A0;&#xFE0F;</span>
            <div>
              <strong>后台服务启动失败</strong>
              <span style="font-size:12px;color:var(--text-tertiary);">${escapeHtml(config.lastSidecarError || '未知错误，请查看日志或重启应用')}</span>
            </div>
          </div>`;
    }

    // v0.5.4 P2-18/P2-19 修复：MCP 配置说明 — 基于 Trae 官方文档的正确信息
    // 参考：https://docs.trae.ai/ide/model-context-protocol
    let mcpGuideHtml = '';
    if (config.mcpConfigResult && Array.isArray(config.mcpConfigResult) && config.mcpConfigResult.length > 0) {
      const successItems = config.mcpConfigResult.filter(r => !r.includes(' — 配置写入失败') && !r.includes(' — 不支持 MCP'));
      const failedItems = config.mcpConfigResult.filter(r => r.includes(' — 配置写入失败'));
      const manualItems = config.mcpConfigResult.filter(r => r.includes(' — 请手动配置') || r.includes(' — HTTP 端点'));

      mcpGuideHtml = `<div class="summary-mcp-guide">
          <div class="mcp-guide-header">
            <span class="step-icon">&#x1F527;</span>
            <strong>MCP 配置状态</strong>
          </div>
          <div class="mcp-guide-body">`;

      if (successItems.length > 0) {
        mcpGuideHtml += `<div class="mcp-item ok">&#x2705; 已自动配置（HTTP 模式）：${escapeHtml(successItems.join('、'))}</div>`;
      }
      if (failedItems.length > 0) {
        mcpGuideHtml += `<div class="mcp-item fail">&#x274C; 配置失败：${escapeHtml(failedItems.join('、'))}</div>`;
      }
      if (manualItems.length > 0) {
        mcpGuideHtml += `<div class="mcp-item info">&#x2139;&#xFE0F; 需手动配置：${escapeHtml(manualItems.join('、'))}</div>`;
      }

      mcpGuideHtml += `<div class="mcp-tip" style="margin-top:8px;padding:8px;background:var(--accent-alpha-08);border-radius:6px;font-size:12px;color:var(--text-secondary);">
          <strong>&#x1F4A1; 使用说明（基于 Trae 官方文档）：</strong><br>
          &#x26A0;&#xFE0F; <strong>重要</strong>：LRC 使用 HTTP 模式连接到桌面端 sidecar，<strong>请先启动桌面端应用</strong>，否则 AI 工具无法连接。<br><br>
          <strong>配置方式</strong>（Trae 官方文档：<a href="https://docs.trae.ai/ide/model-context-protocol" style="color:var(--accent);">https://docs.trae.ai/ide/model-context-protocol</a>）：<br>
          1. <strong>Trae / Trae CN</strong>：设置 → MCP → 添加 → 手动添加，粘贴以下 JSON：<br>
          &nbsp;&nbsp;&nbsp;<code>{"mcpServers":{"lrc-memory":{"url":"http://127.0.0.1:${safePort}/mcp"}}}</code><br>
          2. <strong>配置文件位置</strong>（已自动写入）：<br>
          &nbsp;&nbsp;&nbsp;Trae CN：<code>%APPDATA%/Trae CN/User/mcp.json</code><br>
          &nbsp;&nbsp;&nbsp;Trae 国际版：<code>%APPDATA%/Trae/User/mcp.json</code><br>
          3. <strong>HTTP 端点</strong>：<code>http://127.0.0.1:${safePort}/mcp</code>（桌面端应用运行时可用）<br>
          4. <strong>项目级 MCP</strong>（可选）：在项目根目录创建 <code>.trae/mcp.json</code>，格式同上
        </div>
      </div>
    </div>`;
    } else {
      mcpGuideHtml = `<div class="summary-mcp-guide">
        <div class="mcp-guide-header">
          <span class="step-icon">&#x1F527;</span>
          <strong>MCP 配置状态</strong>
        </div>
        <div class="mcp-guide-body">
          <div class="mcp-item info">&#x2139;&#xFE0F; 未检测到已安装的 AI 工具，或配置未完成</div>
          <div class="mcp-tip" style="margin-top:8px;padding:8px;background:var(--accent-alpha-08);border-radius:6px;font-size:12px;color:var(--text-secondary);">
            <strong>&#x1F4A1; 手动添加 MCP 服务器（基于 Trae 官方文档）：</strong><br>
            &#x26A0;&#xFE0F; <strong>重要</strong>：LRC 使用 HTTP 模式，<strong>请先启动桌面端应用</strong>。<br><br>
            <strong>Trae / Trae CN</strong>：设置 → MCP → 添加 → 手动添加，粘贴以下 JSON：<br>
            <code>{"mcpServers":{"lrc-memory":{"url":"http://127.0.0.1:${safePort}/mcp"}}}</code><br><br>
            官方文档：<a href="https://docs.trae.ai/ide/model-context-protocol" style="color:var(--accent);">https://docs.trae.ai/ide/model-context-protocol</a>
          </div>
        </div>
      </div>`;
    }

    // v0.5.4 新增：首次记忆引导 — 告诉用户接下来该做什么
    const firstMemoryGuide = (verifyResult && verifyResult.all_ok)
      ? `<div class="summary-first-memory">
          <div class="first-memory-header">
            <span class="step-icon">&#x1F680;</span>
            <strong>接下来，试试创建你的第一条代码记忆：</strong>
          </div>
          <ol class="first-memory-steps">
            <li>打开你的 AI 工具（Trae、Cursor 等），确认 MCP 已配置</li>
            <li>在 AI 工具中提问：<code>帮我理解这个项目的整体架构</code></li>
            <li>LRC 将索引你的代码，并为后续问题提供上下文记忆</li>
            <li>点击下方「打开仪表盘」查看记忆索引状态</li>
          </ol>
        </div>`
      : '';

    $('config-summary').innerHTML = `
      ${verifyHtml}
      ${mcpGuideHtml}
      <div class="summary-item"><span class="check">&#x2705;</span> 已配置 AI 工具：${agentList || '无'}</div>
      <div class="summary-item"><span class="check">&#x2705;</span> 索引项目：${projectList || '无'} (${config.selectedProjects.length} 个)</div>
      <div class="summary-item"><span class="check">&#x2705;</span> LLM：${config.llmApiKey ? safeModel : '未配置'}</div>
      <div class="summary-item"><span class="check">${config.multiWindowEnabled ? '&#x1F7E2;' : '&#x26AA;'}</span> 多窗口记录：${config.multiWindowEnabled ? '已开启（上限 5 个）' : '未开启'}</div>
      <div class="summary-item"><span class="check">&#x1F512;</span> 数据安全：所有数据存储在本地，绝不上传</div>
      ${firstMemoryGuide}`;

    // 根据验证结果更新按钮
    const btn = $('btn-open-dashboard');
    const allOk = verifyResult ? verifyResult.all_ok : sidecarStarted;
    if (allOk) {
      btn.disabled = false;
      btn.textContent = '&#x1F4CA; 打开仪表盘，查看记忆索引';
      btn.title = `http://127.0.0.1:${config.port}/dashboard`;
    } else {
      btn.disabled = false;
      btn.textContent = '&#x1F504; 重试启动服务';
      btn.title = '点击重新尝试启动后台服务';
    }
  }

  $('btn-open-dashboard').addEventListener('click', async () => {
    const btn = $('btn-open-dashboard');

    // 如果按钮显示"重试启动服务"，先尝试启动 sidecar
    if (btn.textContent === '重试启动服务') {
      btn.disabled = true;
      btn.textContent = '正在启动...';
      const port = await startSidecarWithConfig(
        config.selectedProjects[0] || null,
        config.port || null,
        config.multiWindowEnabled
      );
      if (port) {
        btn.textContent = '打开仪表盘';
        btn.title = `http://127.0.0.1:${config.port}/dashboard`;
      } else {
        btn.textContent = '启动失败，请检查日志';
        btn.disabled = false;
        return;
      }
      btn.disabled = false;
    }

    // 在 iframe 中打开仪表盘（统一在主窗口内，不弹新窗口、不导航）
    const url = `http://127.0.0.1:${config.port}/dashboard`;
    btn.disabled = true;
    btn.textContent = '正在连接...';

    // 先尝试健康检查，确认 sidecar 已就绪
    let isReady = false;
    for (let i = 0; i < 6; i++) {
      try {
        const resp = await fetch(`http://127.0.0.1:${config.port}/health`, {
          signal: AbortSignal.timeout(1000),
        });
        if (resp.ok) {
          isReady = true;
          break;
        }
      } catch (e) {
        await new Promise((r) => setTimeout(r, 500));
      }
    }

    if (isReady) {
      // 在主窗口 iframe 中显示仪表盘（统一入口，不导航主窗口）
      showDashboardEmbed(config.port);
      btn.textContent = '仪表盘已打开';
    } else {
      btn.textContent = '服务未就绪，请稍后重试';
      console.warn(`[配置向导] Sidecar 健康检查失败 (port=${config.port})`);
    }
    btn.disabled = false;
  });

  // ── 重新启动后台服务 ──
  $('btn-restart-service')?.addEventListener('click', async () => {
    const btn = $('btn-restart-service');
    const dashBtn = $('btn-open-dashboard');
    btn.disabled = true;
    btn.textContent = '正在停止旧服务...';

    try {
      // 1. 停止旧的 sidecar
      await tauriInvoke('stop_sidecar');
      await new Promise(r => setTimeout(r, 1000));

      // 2. 启动新的 sidecar（v0.5.1 重构：使用统一辅助函数）
      btn.textContent = '正在启动服务...';
      const port = await startSidecarWithConfig(
        config.selectedProjects[0] || null,
        config.port || null,
        config.multiWindowEnabled
      );

      if (port) {
        config.port = port;
        btn.textContent = '✅ 服务已启动！';
        // 更新仪表盘按钮
        dashBtn.disabled = false;
        dashBtn.textContent = '打开仪表盘';
        dashBtn.title = `http://127.0.0.1:${port}/dashboard`;
        // 更新状态栏
        updateStatusBar({ running: true, port, state: 'Running' });
        // 3秒后恢复按钮文字
        setTimeout(() => {
          btn.textContent = '🔄 重新启动后台服务';
          btn.disabled = false;
        }, 3000);
      } else {
        btn.textContent = '❌ 启动失败，请稍后重试';
        btn.disabled = false;
      }
    } catch (e) {
      console.error('[重启服务] 失败:', e);
      btn.textContent = '❌ 启动失败：' + (e.message || e);
      btn.disabled = false;
    }
  });

  // ── 初始化 ──

  /**
   * v0.5.4 P2-14 新增：显示 Sidecar 状态通知
   * 在屏幕右上角显示一个临时通知，几秒后自动消失
   * @param {string} message - 通知消息
   * @param {string} type - 通知类型：'success' | 'error'
   */
  function showSidecarNotification(message, type) {
    // 移除已有通知
    const existing = document.getElementById('sidecar-notification');
    if (existing) existing.remove();

    const notification = document.createElement('div');
    notification.id = 'sidecar-notification';
    notification.style.cssText = [
      'position:fixed',
      'top:20px',
      'right:20px',
      'z-index:10000',
      'padding:12px 20px',
      'border-radius:8px',
      'box-shadow:0 4px 12px rgba(0,0,0,0.15)',
      'font-size:14px',
      'color:#fff',
      'max-width:360px',
      'transition:opacity 0.3s',
      type === 'error' ? 'background:#B84838' : 'background:#5B7C63',
    ].join(';');
    notification.textContent = message;

    document.body.appendChild(notification);

    // 5 秒后自动消失
    setTimeout(() => {
      notification.style.opacity = '0';
      setTimeout(() => notification.remove(), 300);
    }, 5000);
  }

  /**
   * v0.5.4 P2-14 新增：设置 Sidecar 心跳检测事件监听
   * 监听后端发出的 sidecar-crash 和 sidecar-recovered 事件
   */
  async function setupSidecarHealthListener() {
    try {
      // 监听崩溃恢复事件
      await tauriListen('sidecar-recovered', (event) => {
        const payload = event.payload || {};
        console.log('[LRC] Sidecar 自动恢复:', payload);
        showSidecarNotification(
          payload.message || '后台服务已自动恢复',
          'success'
        );
        // 刷新状态栏
        refreshSidecarStatus();
      });

      // 监听连续崩溃事件
      await tauriListen('sidecar-crash', (event) => {
        const payload = event.payload || {};
        console.error('[LRC] Sidecar 崩溃:', payload);
        showSidecarNotification(
          payload.message || '后台服务异常，请手动重启',
          'error'
        );
        // 刷新状态栏
        refreshSidecarStatus();
      });
    } catch (e) {
      console.warn('[LRC] Sidecar 心跳事件监听设置失败:', e);
    }
  }

  /**
   * v0.5.4 P2-14 新增：刷新 Sidecar 状态栏
   * 心跳事件触发后，更新前端显示的运行状态
   */
  async function refreshSidecarStatus() {
    try {
      const status = await tauriInvoke('get_sidecar_status');
      const statusDot = document.querySelector('.status-dot');
      const statusText = document.querySelector('.status-text');
      if (statusDot && statusText) {
        if (status.running) {
          statusDot.style.background = '#5B7C63';
          statusText.textContent = `运行中（端口 ${status.port || '未知'}）`;
        } else {
          statusDot.style.background = '#B84838';
          statusText.textContent = '已停止';
        }
      }
    } catch (e) {
      // 状态刷新失败不影响主流程
      console.warn('[LRC] 状态刷新失败:', e);
    }
  }

  async function init() {
    // v0.5.8 新增：绑定主题切换按钮事件
    const themeToggleBtn = document.getElementById('theme-toggle');
    if (themeToggleBtn) {
      themeToggleBtn.addEventListener('click', toggleTheme);
    }

    // v0.5.4 P2-14 新增：监听 Sidecar 心跳检测事件
    // 崩溃恢复时显示"服务已自动恢复"，连续失败时显示"服务异常"
    setupSidecarHealthListener();

    // 检查是否从托盘"切换项目"进入
    const isSwitchProject = window.location.hash === '#wizard-switch-project';

    try {
      const state = await tauriInvoke('get_wizard_state');
      if (state && state.setup_complete && !isSwitchProject) {
        config.selectedAgents = state.configured_agents || [];
        config.selectedProjects = state.project_dir ? [state.project_dir] : [];
        // v0.5.4 修复：检测配置损坏状态，提示用户
        if (state.config_corrupted) {
          console.warn('[配置向导] 配置文件已损坏，已使用默认配置恢复');
          // 显示警告横幅（非阻塞，用户可继续使用）
          const introBox = document.querySelector('.wizard-intro-box');
          if (introBox) {
            introBox.style.borderColor = '#B84838';
            introBox.querySelector('.intro-text').innerHTML = 
              '<strong style="color:#B84838;">⚠️ 配置文件已损坏</strong> 之前的配置无法读取，已使用默认配置。' +
              '请重新配置项目路径和 LLM API Key。';
          }
        }
        showReadyPanel(state);
        return;
      }
    } catch (e) {
      console.warn('[配置向导] 状态检查失败:', e);
    }

    // 从托盘"切换项目"进入 → 直接跳到步骤1（项目选择在步骤1中）
    if (isSwitchProject) {
      document.querySelector('.wizard-container h1').textContent = '切换项目';
      document.querySelector('.wizard-subtitle').textContent = '选择新项目，LRC 将重新索引代码';
      // 更新进度条
      const progressBar = $('progress-bar-fill');
      if (progressBar) progressBar.style.width = '33%';
      // 预先加载 Agent 列表
      try {
        // v0.5.7 修复：使用超时包装
        const agents = await tauriInvokeWithTimeout('detect_agents', {}, 10000);
        if (agents) config.allAgents = agents;
      } catch (e) {
        console.warn('[配置向导] Agent 检测预加载失败:', e);
      }
      goToStep(1);
      // 自动加载项目列表
      setTimeout(() => loadProjectsInStep1(), 500);
      return;
    }

    goToStep(1);
    loadAgents();
  }

  // v0.5.1 重构：原 showReadyPanel 已由下方 override 完全替代
  // 旧实现使用 panel-step-done 流程，新实现使用 ready-panel 流程
  // 此处保留函数签名以维持代码结构，实际实现在下方 override 中
  async function showReadyPanel(state) {
    // 此函数在 IIFE 底部被覆盖为 ready-panel 版本
    // 实际逻辑见下方 override
  }

  // 暴露到 window 以便测试和外部调用
  window.Wizard = { init };
  window.Wizard.LLM_PROVIDERS = LLM_PROVIDERS;
  console.log('[配置向导] Wizard 模块已加载，等待 Tauri API 就绪...');

  // ══════════════════════════════════════════════════════════════
  // 设置面板逻辑（LLM 配置 + Agent 配置引导）
  // ══════════════════════════════════════════════════════════════

  // 设置面板 LLM 提供商切换
  const settingsProviderEl = $('settings-llm-provider');
  if (settingsProviderEl) {
    settingsProviderEl.addEventListener('change', function () {
      const provider = this.value;
      const isOllama = provider === 'ollama';
      const isCustom = provider === 'custom';

      // 切换 Ollama 字段
      const ollamaFields = $('settings-ollama-fields');
      if (ollamaFields) ollamaFields.style.display = isOllama ? 'block' : 'none';
      const ollamaModelField = $('settings-ollama-model-field');
      if (ollamaModelField) ollamaModelField.style.display = isOllama ? 'block' : 'none';

      // 切换 API Key 区域
      const apiSection = $('settings-llm-api-section');
      if (apiSection) apiSection.style.display = isOllama ? 'none' : 'block';

      // 更新默认 URL 和模型
      const providerInfo = LLM_PROVIDERS[provider];
      if (providerInfo && $('settings-llm-base-url')) {
        $('settings-llm-base-url').value = providerInfo.url || '';
      }
      if (providerInfo && $('settings-llm-model')) {
        $('settings-llm-model').placeholder = providerInfo.model || '';
      }

      // 更新 Key 提示和链接
      if ($('settings-llm-key-hint')) {
        $('settings-llm-key-hint').textContent = providerInfo ? providerInfo.keyHint || '格式：sk-...' : '格式：sk-...';
      }
      // v0.5.7 修复：Ollama 和 custom 无需 API Key，隐藏获取链接
      const settingsKeyLink = $('settings-llm-key-link');
      if (settingsKeyLink && providerInfo) {
        if (providerInfo.keyUrl) {
          settingsKeyLink.href = providerInfo.keyUrl;
          settingsKeyLink.textContent = `获取 ${providerInfo.name} API Key →`;
          settingsKeyLink.style.display = 'inline';
        } else {
          settingsKeyLink.style.display = 'none';
        }
      }
    });
  }

  // 设置面板 LLM 测试连接
  // 通过 Rust 后端代理，避免浏览器 CSP 限制（fetch 在 WebView 中会被拦截）
  const settingsBtnTest = $('settings-btn-test-llm');
  if (settingsBtnTest) {
    settingsBtnTest.addEventListener('click', async () => {
      const btn = $('settings-btn-test-llm');
      const resultEl = $('settings-llm-test-result');
      btn.disabled = true;
      btn.textContent = '测试中...';

      try {
        const provider = $('settings-llm-provider').value;
        // v0.5.4 修复：API Key 输入清洗，trim 去除复制粘贴带入的空白
        const apiKey = ($('settings-llm-api-key').value || '').trim();
        const baseUrl = $('settings-llm-base-url').value || LLM_PROVIDERS[provider]?.url || '';
        const model = $('settings-llm-model').value || LLM_PROVIDERS[provider]?.model || null;

        // 通过 Rust 后端代理测试连接（避免 CSP 限制）
        // v0.5.7 修复：使用超时包装（网络请求，给 30 秒）
        const result = await tauriInvokeWithTimeout('test_llm_connection', {
          provider,
          apiKey,
          baseUrl,
          model,
        }, 30000);

        if (result.success) {
          resultEl.style.display = 'block';
          resultEl.className = 'llm-test-result success';
          resultEl.textContent = `✅ ${result.message}`;
        } else {
          resultEl.style.display = 'block';
          resultEl.className = 'llm-test-result error';
          resultEl.textContent = `❌ ${result.message}`;
        }
      } catch (e) {
        resultEl.style.display = 'block';
        resultEl.className = 'llm-test-result error';
        // v0.5.4 P1-6 修复：提取后端已翻译的错误消息
        const msg = (typeof e === 'string') ? e : (e?.message || e || '连接失败');
        resultEl.textContent = `❌ ${escapeHtml(msg)}`;
      } finally {
        btn.disabled = false;
        btn.textContent = '测试连接';
      }
    });
  }

  // 设置面板保存 LLM 配置
  const settingsBtnSave = $('settings-btn-save-llm');
  if (settingsBtnSave) {
    settingsBtnSave.addEventListener('click', async () => {
      const btn = $('settings-btn-save-llm');
      const statusEl = $('settings-llm-status');
      btn.disabled = true;
      btn.textContent = '保存中...';

      const provider = $('settings-llm-provider').value;
      let llmString = '';

      if (provider === 'ollama') {
        const ollamaModel = $('settings-ollama-model').value || 'llama3';
        const ollamaUrl = $('settings-ollama-url').value || 'http://localhost:11434';
        // v0.5.7 修复：使用 || 分隔符（与后端 to_llm_api_string 保持一致）
        llmString = `ollama||${ollamaModel}||${ollamaUrl}`;
        config.llmProvider = 'ollama';
        config.llmModel = ollamaModel;
        config.ollamaUrl = ollamaUrl;
      } else {
        // v0.5.4 修复：API Key 输入清洗，trim 去除复制粘贴带入的空白
        const apiKey = ($('settings-llm-api-key').value || '').trim();
        const model = $('settings-llm-model').value || LLM_PROVIDERS[provider]?.model || '';
        const baseUrl = $('settings-llm-base-url').value || LLM_PROVIDERS[provider]?.url || '';
        if (apiKey) {
          // v0.5.7 修复：使用 || 分隔符（支持 API Key 中包含冒号）
          llmString = `openai||${apiKey}||${model}||${baseUrl}`;
          config.llmApiKey = apiKey;
          config.llmModel = model;
          config.llmBaseUrl = baseUrl;
          config.llmProvider = provider;
        }
      }

      try {
        // v0.5.7 修复：使用超时包装
        await tauriInvokeWithTimeout('save_llm_config', { llmApi: llmString }, 5000);
        statusEl.style.display = 'block';
        statusEl.className = 'llm-status success';
        statusEl.textContent = '✅ LLM 配置已保存';
        updateStatusBar(null);
        // 3秒后隐藏状态
        setTimeout(() => { statusEl.style.display = 'none'; }, 3000);
      } catch (e) {
        statusEl.style.display = 'block';
        statusEl.className = 'llm-status error';
        const msg = (typeof e === 'string') ? e : (e?.message || e || '保存失败');
        statusEl.textContent = `❌ ${escapeHtml(msg)}`;
      } finally {
        btn.disabled = false;
        btn.textContent = '💾 保存 LLM 配置';
      }
    });
  }

  // 设置面板清除 LLM 配置
  const settingsBtnClear = $('settings-btn-clear-llm');
  if (settingsBtnClear) {
    settingsBtnClear.addEventListener('click', async () => {
      try {
        await tauriInvoke('clear_llm_config');
        $('settings-llm-api-key').value = '';
        $('settings-llm-model').value = '';
        config.llmApiKey = null;
        config.llmModel = '';
        config.llmProvider = 'deepseek';
        const statusEl = $('settings-llm-status');
        statusEl.style.display = 'block';
        statusEl.className = 'llm-status success';
        statusEl.textContent = '✅ LLM 配置已清除';
        updateStatusBar(null);
        setTimeout(() => { statusEl.style.display = 'none'; }, 3000);
      } catch (e) {
        console.warn('[设置] 清除 LLM 配置失败:', e);
      }
    });
  }

  // 🔄 v0.5.4 新增：重新配置 Agent MCP 按钮
  const reconfigureBtn = document.getElementById('btn-reconfigure-agents');
  const reconfigureStatus = document.getElementById('reconfigure-status');
  if (reconfigureBtn && reconfigureStatus) {
    reconfigureBtn.addEventListener('click', async () => {
      reconfigureBtn.disabled = true;
      reconfigureStatus.textContent = '正在检测 AI 工具...';
      try {
        // 获取已检测到的已安装 Agent
        // v0.5.7 修复：使用超时包装 + 只选择支持 MCP 的工具
        const agents = await tauriInvokeWithTimeout('detect_installed_agents', {}, 10000);
        const installedIds = agents.filter(a => a.installed && a.supports_mcp).map(a => a.id);
        if (installedIds.length === 0) {
          reconfigureStatus.textContent = '⚠️ 未检测到支持 MCP 的已安装 AI 工具';
          return;
        }
        // 配置选中的 Agent
        reconfigureStatus.textContent = `正在配置 ${installedIds.length} 个 AI 工具...`;
        const state = await tauriInvoke('get_wizard_state');
        const port = state.sidecar_port || 3099;
        if (!state.sidecar_running) {
          reconfigureStatus.textContent = '⚠️ Sidecar 未启动，仅写入配置文件（需稍后启动 Sidecar）';
        }
        // v0.5.7 修复：使用超时包装，避免 configure_agents 卡住
        const results = await tauriInvokeWithTimeout('configure_agents', {
          agentIds: installedIds,
          port
        }, 30000);
        const successCount = results.filter(r => !r.includes(' — 配置写入失败')).length;
        reconfigureStatus.textContent = `✅ 完成，成功配置 ${successCount}/${installedIds.length} 个 AI 工具`;
        console.log('[重新配置 AI 工具] 结果:', results);
      } catch (e) {
        reconfigureStatus.textContent = `❌ 配置失败: ${e}`;
        console.error('[重新配置 Agent] 错误:', e);
      } finally {
        reconfigureBtn.disabled = false;
        // 5秒后清空状态
        setTimeout(() => {
          reconfigureStatus.textContent = '';
        }, 8000);
      }
    });
  }

  // Agent 配置标签切换
  const agentTabs = document.querySelectorAll('#agent-config-tabs .agent-tab');
  agentTabs.forEach(tab => {
    tab.addEventListener('click', () => {
      agentTabs.forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
      const tabId = tab.dataset.tab;
      document.querySelectorAll('.agent-config-panel').forEach(p => p.style.display = 'none');
      const panel = document.getElementById(`panel-${tabId}`);
      if (panel) panel.style.display = 'block';
    });
  });

  // ══════════════════════════════════════════════════════════════
  // 已就绪面板逻辑
  // ══════════════════════════════════════════════════════════════

  /** 更新已就绪面板状态 */
  async function updateReadyPanelStatus() {
    try {
      const state = await tauriInvoke('get_wizard_state');
      if (!state) return;

      // 后台服务状态
      const sidecarIcon = $('ready-sidecar-icon');
      const sidecarText = $('ready-sidecar-text');
      if (sidecarIcon && sidecarText) {
        if (state.sidecar_running) {
          sidecarIcon.textContent = '✅';
          sidecarText.textContent = `运行中（端口 ${state.sidecar_port || '3099'}）`;
        } else {
          sidecarIcon.textContent = '⚠️';
          sidecarText.textContent = '未启动';
        }
      }

      // LLM 配置状态
      const llmIcon = $('ready-llm-icon');
      const llmText = $('ready-llm-text');
      if (llmIcon && llmText) {
        if (state.llm_configured) {
          llmIcon.textContent = '✅';
          llmText.textContent = '已配置';
        } else {
          llmIcon.textContent = '⚪';
          llmText.textContent = '未配置（可选）';
        }
      }

      // Agent 状态
      const agentsIcon = $('ready-agents-icon');
      const agentsText = $('ready-agents-text');
      if (agentsIcon && agentsText) {
        // v0.5.8 修复：只计数支持 MCP 的已配置工具，与 updateStatusBar 保持一致
        // 修复前：直接用 state.configured_agents.length，导致不支持 MCP 的工具也被计数
        //         （如通义灵码、豆包 MarsCode 等），显示"5 个"但实际只有 2 个能配置
        const allConfigured = state.configured_agents || [];
        let count;
        if (config.allAgents && config.allAgents.length > 0) {
          count = allConfigured.filter((id) => {
            const agent = config.allAgents.find((a) => a.id === id);
            return agent && agent.supports_mcp;
          }).length;
        } else {
          count = allConfigured.length;
        }
        if (count > 0) {
          agentsIcon.textContent = '✅';
          agentsText.textContent = `${count} 个 AI 工具已配置`;
        } else {
          agentsIcon.textContent = '⚠️';
          agentsText.textContent = '未配置 AI 工具';
        }
      }

      // 更新关于端口
      const aboutPort = $('about-port');
      if (aboutPort) {
        aboutPort.textContent = state.sidecar_port || '3099';
      }
    } catch (e) {
      console.warn('[已就绪面板] 状态更新失败:', e);
    }
  }

  // 已就绪面板按钮事件
  const btnReadySettings = $('btn-ready-settings');
  if (btnReadySettings) {
    btnReadySettings.addEventListener('click', () => showSettingsPanel());
  }

  const btnReadyDashboard = $('btn-ready-dashboard');
  if (btnReadyDashboard) {
    btnReadyDashboard.addEventListener('click', async () => {
      await openDashboardInNewWindow();
    });
  }

  const btnReadyWizard = $('btn-ready-wizard');
  if (btnReadyWizard) {
    btnReadyWizard.addEventListener('click', async () => {
      // v0.5.4 P0-3 修复：切换项目时跳到步骤 1（项目选择在步骤 1 中）
      document.querySelector('.wizard-container h1').textContent = '切换项目';
      document.querySelector('.wizard-subtitle').textContent = '选择新项目，LRC 将重新索引代码';
      // 更新进度条
      const progressBar = $('progress-bar-fill');
      if (progressBar) progressBar.style.width = '33%';
      try {
        // v0.5.7 修复：使用超时包装
        const agents = await tauriInvokeWithTimeout('detect_agents', {}, 10000);
        if (agents) config.allAgents = agents;
      } catch (e) {
        console.warn('[配置向导] Agent 检测预加载失败:', e);
      }
      $('ready-panel').style.display = 'none';
      $('config-wizard').style.display = 'block';
      goToStep(1);
      // 自动加载项目列表
      setTimeout(() => loadProjectsInStep1(), 500);
    });
  }

  // v0.5.3 新增：重置配置按钮 — 清除所有配置，重新进入向导
  const btnReadyReset = $('btn-ready-reset');
  if (btnReadyReset) {
    btnReadyReset.addEventListener('click', async () => {
      if (!confirm('确定要重置所有配置吗？\n\n这将清除项目、AI 工具和 MCP 配置（LLM API Key 会保留）。\n下次打开时将重新进入配置向导。')) {
        return;
      }
      btnReadyReset.disabled = true;
      btnReadyReset.textContent = '正在重置...';
      try {
        await tauriInvoke('reset_wizard');
        console.log('[配置向导] 配置已重置');
        // 刷新页面，重新进入向导
        window.location.reload();
      } catch (e) {
        console.warn('[配置向导] 重置失败:', e);
        btnReadyReset.disabled = false;
        btnReadyReset.textContent = '重置失败，请重试';
      }
    });
  }

  // ══════════════════════════════════════════════════════════════
  // v0.5.4 P1-8 新增：首次使用引导逻辑
  // ══════════════════════════════════════════════════════════════

  /** 30 秒体验：自动写入测试记忆并检索 */
  async function quickstartTest() {
    const btn = $('btn-quickstart-test');
    const result = $('quickstart-test-result');
    if (!btn || !result) return;

    btn.disabled = true;
    btn.textContent = '⏳ 正在执行测试...';
    result.style.display = 'block';
    result.className = 'quickstart-test-result';
    result.innerHTML = '<div class="qs-test-step">步骤 1/3：正在写入测试记忆...</div>';

    const port = config.port || 3099;
    const baseUrl = `http://127.0.0.1:${port}`;

    try {
      // 步骤 1：写入测试记忆
      // v0.5.7 修复：改为通用测试内容，避免硬编码特定技术栈
      const testContent = 'LRC 测试记忆：这是一条示例记忆，用于验证 LRC 记忆系统是否正常工作。你可以通过 AI 工具调用 remember 工具存储任何项目信息。';
      const writeResp = await fetch(`${baseUrl}/v1/memories/consolidate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          memories: [{
            content: testContent,
            memory_type: 'fact',
            importance: 7,
            tags: ['quickstart', 'test'],
            project: 'quickstart-demo'
          }]
        }),
        signal: AbortSignal.timeout(10000),
      });

      if (!writeResp.ok) throw new Error('写入失败：HTTP ' + writeResp.status);
      const writeData = await writeResp.json();

      result.innerHTML += `<div class="qs-test-step qs-test-success">✓ 步骤 1 完成：已写入 ${writeData.stored || 1} 条记忆</div>`;
      result.innerHTML += '<div class="qs-test-step">步骤 2/3：正在检索记忆...</div>';

      // 步骤 2：检索记忆
      await new Promise(r => setTimeout(r, 500)); // 等待索引完成
      const searchResp = await fetch(`${baseUrl}/v1/memories/enrich`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          query: 'LRC 记忆系统 示例',
          top_k: 3
        }),
        signal: AbortSignal.timeout(10000),
      });

      if (!searchResp.ok) throw new Error('检索失败：HTTP ' + searchResp.status);
      const searchData = await searchResp.json();

      result.innerHTML += `<div class="qs-test-step qs-test-success">✓ 步骤 2 完成：检索到 ${searchData.total || searchData.memories.length} 条相关记忆</div>`;
      result.innerHTML += '<div class="qs-test-step">步骤 3/3：验证完成</div>';

      // 显示检索到的记忆
      if (searchData.memories && searchData.memories.length > 0) {
        const memList = searchData.memories.slice(0, 2).map(m => {
          const preview = m.content.length > 60 ? m.content.slice(0, 60) + '...' : m.content;
          return `<div class="qs-test-memory">📝 ${preview} (${(m.score * 100).toFixed(0)}%)</div>`;
        }).join('');
        result.innerHTML += `<div class="qs-test-step qs-test-success">✓ 步骤 3 完成：记忆系统工作正常！</div>`;
        result.innerHTML += `<div class="qs-test-memories"><strong>检索到的记忆：</strong>${memList}</div>`;
      } else {
        result.innerHTML += '<div class="qs-test-step qs-test-warning">⚠ 步骤 3：未检索到记忆，可能索引尚未完成，请稍后再试</div>';
      }

      result.innerHTML += '<div class="qs-test-final">🎉 测试完成！LRC 记忆系统已正常工作。现在可以在你的 AI 编程工具中使用了。</div>';

      // 标记步骤完成
      markQuickstartStepDone(2);
      markQuickstartStepDone(3);
    } catch (e) {
      result.innerHTML += `<div class="qs-test-step qs-test-error">✗ 测试失败：${e.message}</div>`;
      result.innerHTML += '<div class="qs-test-hint">请确认后台服务已启动（状态卡片显示"运行中"）。如问题持续，点击"⚙️ 设置"检查配置。</div>';
    } finally {
      btn.disabled = false;
      btn.textContent = '⚡ 30 秒体验：自动写入测试记忆';
    }
  }

  /** 标记引导步骤完成 */
  function markQuickstartStepDone(stepNum) {
    const step = $(`qs-step-${stepNum}`);
    if (!step) return;
    const check = step.querySelector('.qs-step-check');
    const num = step.querySelector('.qs-step-num');
    if (check) check.style.display = 'inline';
    if (num) num.style.background = 'var(--jade, #5B7C63)';
    step.classList.add('completed');
  }

  /** 跳过引导 */
  function skipQuickstart() {
    const guide = $('quickstart-guide');
    if (guide) {
      guide.style.transition = 'opacity 0.3s';
      guide.style.opacity = '0';
      setTimeout(() => { guide.style.display = 'none'; }, 300);
    }
  }

  // 绑定引导按钮事件
  const btnQuickstartTest = $('btn-quickstart-test');
  if (btnQuickstartTest) {
    btnQuickstartTest.addEventListener('click', quickstartTest);
  }

  const btnQuickstartSkip = $('btn-quickstart-skip');
  if (btnQuickstartSkip) {
    btnQuickstartSkip.addEventListener('click', skipQuickstart);
  }

  // 打开仪表盘按钮：标记步骤 4 完成
  const btnReadyDashboardForGuide = $('btn-ready-dashboard');
  if (btnReadyDashboardForGuide) {
    btnReadyDashboardForGuide.addEventListener('click', () => {
      markQuickstartStepDone(4);
    });
  }

  // 设置面板关闭按钮
  const btnSettingsClose = $('btn-settings-close');
  if (btnSettingsClose) {
    btnSettingsClose.addEventListener('click', () => {
      showReadyOrWizard();
    });
  }

  // ══════════════════════════════════════════════════════════════
  // 面板切换函数
  // ══════════════════════════════════════════════════════════════

  /** 显示设置面板 */
  function showSettingsPanel() {
    $('config-wizard').style.display = 'none';
    $('ready-panel').style.display = 'none';
    $('settings-panel').style.display = 'block';
    // 加载当前 LLM 配置
    loadCurrentLlmConfig();
    // 加载多窗口状态
    loadMultiWindowConfig();
    // 加载项目列表
    loadProjectList();
    // 更新面板状态
    updateReadyPanelStatus();
  }

  /** 加载多窗口配置到设置面板 */
  function loadMultiWindowConfig() {
    const toggle = $('settings-multi-window-enabled');
    const info = $('settings-multi-window-info');
    if (toggle) {
      toggle.checked = config.multiWindowEnabled;
    }
    if (info) {
      const badge = info.querySelector('.info-badge');
      if (badge) {
        badge.textContent = config.multiWindowEnabled ? '已启用' : '已禁用';
        badge.style.background = config.multiWindowEnabled ? '#5B7C63' : '#8A8680';
      }
      const hint = info.querySelector('.form-hint');
      if (hint) {
        hint.textContent = config.multiWindowEnabled
          ? '当前限制：5 个窗口。可在下方关闭此功能。'
          : '多窗口记录已关闭，同项目仅允许 1 个窗口。';
      }
    }
  }

  /** 加载项目列表到设置面板 */
  async function loadProjectList() {
    const listEl = $('settings-project-list');
    if (!listEl) return;

    try {
      const projects = await tauriInvoke('list_sidecar_projects');
      if (!projects || projects.length === 0) {
        // 显示当前选中的项目
        if (config.selectedProjects && config.selectedProjects.length > 0) {
          listEl.innerHTML = config.selectedProjects.map(p => `
            <div class="project-item">
              <span class="project-path">${escapeHtml(p)}</span>
              <span class="project-status">
                <span class="status-dot stopped"></span>
                <span style="font-size:11px;color:#8A8680;">未启动</span>
              </span>
            </div>
          `).join('');
        } else {
          listEl.innerHTML = '<span class="form-hint">暂无项目。点击下方按钮添加项目目录。</span>';
        }
        return;
      }

      listEl.innerHTML = projects.map(p => `
        <div class="project-item">
          <span class="project-path">${escapeHtml(p.project_dir || p.src_dir || '')}</span>
          <span class="project-status">
            <span class="status-dot ${p.running ? 'running' : 'stopped'}"></span>
            <span style="font-size:11px;color:${p.running ? '#5B7C63' : '#8A8680'};">
              ${p.running ? '运行中' : '未启动'}
            </span>
            ${p.port ? `<span style="font-size:11px;color:#6E6A63;">:${p.port}</span>` : ''}
          </span>
        </div>
      `).join('');
    } catch (e) {
      console.warn('[设置] 加载项目列表失败:', e);
      listEl.innerHTML = '<span class="form-hint">加载项目列表失败，请检查后台服务状态。</span>';
    }
  }

  /** 设置面板：多窗口开关事件 */
  const settingsMultiWindowToggle = $('settings-multi-window-enabled');
  if (settingsMultiWindowToggle) {
    settingsMultiWindowToggle.addEventListener('change', (e) => {
      config.multiWindowEnabled = e.target.checked;
      loadMultiWindowConfig();
      console.log('[设置] 多窗口 LRC 记录:', config.multiWindowEnabled ? '已开启（上限5个）' : '已关闭');
    });
  }

  /** 设置面板：添加项目 */
  const btnAddProject = $('btn-settings-add-project');
  if (btnAddProject) {
    btnAddProject.addEventListener('click', async () => {
      try {
        const dir = await tauriInvoke('pick_project_dir');
        if (dir) {
          if (!config.selectedProjects.includes(dir)) {
            config.selectedProjects.push(dir);
          }
          await tauriInvoke('set_project_dir', { projectDir: dir });
          await loadProjectList();
          updateReadyPanelStatus();
        }
      } catch (e) {
        console.warn('[设置] 添加项目失败:', e);
      }
    });
  }

  /** 设置面板：刷新项目列表 */
  const btnRefreshProjects = $('btn-settings-refresh-projects');
  if (btnRefreshProjects) {
    btnRefreshProjects.addEventListener('click', () => {
      loadProjectList();
      updateReadyPanelStatus();
    });
  }

  /** 加载当前 LLM 配置到设置面板 */
  async function loadCurrentLlmConfig() {
    try {
      const llmConfig = await tauriInvoke('get_llm_config');
      if (llmConfig && llmConfig.configured) {
        // 设置提供商
        if ($('settings-llm-provider') && llmConfig.llm_type) {
          $('settings-llm-provider').value = llmConfig.llm_type;
          // 触发 change 事件以更新 UI
          $('settings-llm-provider').dispatchEvent(new Event('change'));
        }
        // 设置模型
        if ($('settings-llm-model') && llmConfig.model) {
          $('settings-llm-model').value = llmConfig.model;
        }
      }
    } catch (e) {
      console.warn('[设置] 加载 LLM 配置失败:', e);
    }
  }

  /** 显示已就绪面板或向导 */
  function showReadyOrWizard() {
    $('settings-panel').style.display = 'none';
    $('config-wizard').style.display = 'block';
    $('ready-panel').style.display = 'none';
    // 重新检查向导状态
    window.Wizard.init();
  }

  /** 在主窗口 iframe 中打开仪表盘（统一入口，不弹新窗口） */
  async function openDashboardInNewWindow() {
    showDashboardEmbed(config.port);
  }

  // ══════════════════════════════════════════════════════════════
  // Hash 路由监听
  // ══════════════════════════════════════════════════════════════
  window.addEventListener('hashchange', () => {
    // 先隐藏仪表盘 iframe（如果正在显示）
    hideDashboardEmbed();

    const hash = window.location.hash;
    if (hash === '#settings') {
      showSettingsPanel();
    } else if (hash === '#wizard-switch-project') {
      showReadyOrWizard();
    } else if (hash === '#dashboard') {
      // 仪表盘通过 iframe 显示
      showDashboardEmbed(config.port);
    } else if (hash === '#about') {
      showSettingsPanel();
      // 滚动到关于区域
      setTimeout(() => {
        const aboutSection = document.querySelector('.settings-section:last-of-type');
        if (aboutSection) aboutSection.scrollIntoView({ behavior: 'smooth' });
      }, 100);
    } else {
      showReadyOrWizard();
    }
  });

  // ══════════════════════════════════════════════════════════════
  // v0.5.1 重构：覆盖 showReadyPanel 为新版 ready-panel 流程
  // 旧版 panel-step-done 流程已废弃，统一使用 ready-panel
  // ══════════════════════════════════════════════════════════════
  showReadyPanel = async function (state) {
    // 隐藏向导，显示已就绪面板
    $('config-wizard').style.display = 'none';
    $('settings-panel').style.display = 'none';
    const readyPanel = $('ready-panel');
    if (readyPanel) readyPanel.style.display = 'block';

    // 保存状态
    if (state.project_dir) config.selectedProjects = [state.project_dir];
    if (state.configured_agents) config.selectedAgents = state.configured_agents;
    if (state.sidecar_port) config.port = state.sidecar_port;

    // 更新状态
    await updateReadyPanelStatus();

    // 自动启动 sidecar（如果未运行）（v0.5.1 重构：使用统一辅助函数）
    if (!state.sidecar_running && state.project_dir) {
      console.log('[已就绪] 自动启动后台服务...');
      const port = await startSidecarWithConfig(
        state.project_dir,
        config.port || null,
        config.multiWindowEnabled
      );
      if (port) {
        await updateReadyPanelStatus();
      }
    }

    // 启动状态轮询
    startStatusPolling();
    // v0.5.4 修复：定期更新就绪面板，保存引用防止内存泄漏
    if (readyPanelPollTimer) clearInterval(readyPanelPollTimer);
    readyPanelPollTimer = setInterval(updateReadyPanelStatus, 10000);
  };

  // v0.5.4 修复：面板切换函数移入 Wizard 命名空间，避免全局命名空间污染
  window.Wizard.showSettingsPanel = showSettingsPanel;
  window.Wizard.showReadyOrWizard = showReadyOrWizard;
  window.Wizard.openDashboardInNewWindow = openDashboardInNewWindow;

  // ══════════════════════════════════════════════════════════════
  // 仪表盘 iframe 嵌入层（统一在主窗口内，不创建新窗口）
  // ══════════════════════════════════════════════════════════════

  /** 在 iframe 中显示仪表盘（统一入口，不弹新窗口） */
  function showDashboardEmbed(port) {
    const embedEl = $('dashboard-embed');
    const iframe = $('dashboard-iframe');
    const portEl = $('dashboard-embed-port');
    if (!embedEl || !iframe) return;

    const actualPort = port || config.port || 3099;
    const url = `http://127.0.0.1:${actualPort}/dashboard?embedded=tauri`;

    iframe.src = url;
    if (portEl) portEl.textContent = `:${actualPort}`;
    embedEl.style.display = 'flex';

    // 隐藏所有面板
    $('config-wizard').style.display = 'none';
    $('ready-panel').style.display = 'none';
    $('settings-panel').style.display = 'none';
    $('status-bar').style.display = 'none';

    console.log('[仪表盘] 已嵌入 iframe, port=' + actualPort);
  }

  /** 隐藏仪表盘 iframe，返回主界面 */
  function hideDashboardEmbed() {
    const embedEl = $('dashboard-embed');
    if (embedEl) embedEl.style.display = 'none';
    $('status-bar').style.display = 'flex';

    // 恢复主界面
    showReadyOrWizard();
    console.log('[仪表盘] 已返回主界面');
  }

  // 仪表盘返回按钮
  const btnDashboardBack = $('btn-dashboard-back');
  if (btnDashboardBack) {
    btnDashboardBack.addEventListener('click', hideDashboardEmbed);
  }

  // v0.5.4 修复：仪表盘函数移入 Wizard 命名空间
  window.Wizard.showDashboardEmbed = showDashboardEmbed;
  window.Wizard.hideDashboardEmbed = hideDashboardEmbed;

  // v0.5.4 P1-5 修复：最近活动追踪
  let lastActivityTime = null;  // 最后活动时间戳
  let lastActivityText = '';    // 最后活动描述

  // ── 左下角状态栏：实时轮询 sidecar 状态 ──
  let statusPollTimer = null;
  // v0.5.4 修复：保存就绪面板轮询引用，防止内存泄漏
  let readyPanelPollTimer = null;

  /** v0.5.4 P1-5 修复：更新状态栏 UI — 用户友好的三态指示 */
  async function updateStatusBar(status) {
    const iconEl = $('status-bar-icon');
    const labelEl = $('status-bar-label');
    const activityEl = $('status-bar-activity');
    const detailPort = $('status-detail-port');
    const detailLlm = $('status-detail-llm');
    const detailAgents = $('status-detail-agents');
    const detailMcp = $('status-detail-mcp');

    // ── 1. 确定三态 ──
    let stateIcon, stateLabel;

    if (status && status.running && status.healthOk !== false) {
      // 🟢 运行中
      stateIcon = '&#x1F7E2;';
      stateLabel = 'LRC 运行中';
      // 记录活动时间
      if (!lastActivityTime) {
        lastActivityTime = Date.now();
        lastActivityText = '已启动';
      }
    } else if (status && (status.state === 'Starting' || status.state === 'starting')) {
      // 🟡 启动中
      stateIcon = '&#x1F7E1;';
      stateLabel = 'LRC 启动中...';
    } else if (status && status.running && status.healthOk === false) {
      // 🟡 已启动但健康检查失败（等待 AI 工具连接或 MCP 就绪）
      stateIcon = '&#x1F7E1;';
      stateLabel = 'LRC 已启动，等待连接';
    } else {
      // 🔴 未运行
      stateIcon = '&#x1F534;';
      stateLabel = 'LRC 未运行';
    }

    if (iconEl) iconEl.innerHTML = stateIcon;
    if (labelEl) labelEl.textContent = stateLabel;

    // ── 2. 最近活动 ──
    if (activityEl) {
      if (lastActivityTime) {
        const elapsed = Math.floor((Date.now() - lastActivityTime) / 1000);
        if (elapsed < 60) {
          activityEl.textContent = `${elapsed} 秒前 ${lastActivityText}`;
        } else if (elapsed < 3600) {
          activityEl.textContent = `${Math.floor(elapsed / 60)} 分钟前 ${lastActivityText}`;
        } else {
          activityEl.textContent = `${Math.floor(elapsed / 3600)} 小时前 ${lastActivityText}`;
        }
        activityEl.style.display = 'inline';
      } else {
        activityEl.style.display = 'none';
      }
    }

    // ── 3. 技术细节 ──
    if (detailPort) {
      detailPort.textContent = status && status.port ? `:${status.port}` : '—';
    }
    if (detailAgents) {
      // v0.5.7 修复：只计数支持 MCP 的已选中工具，避免显示"5 个"但实际只有 2 个能配置
      const mcpAgentCount = config.selectedAgents.filter((id) => {
        const agent = config.allAgents.find((a) => a.id === id);
        return agent && agent.supports_mcp;
      }).length;
      detailAgents.textContent = mcpAgentCount > 0
        ? `${mcpAgentCount} 个`
        : '0 个';
    }

    // LLM 状态 — 从后端实时读取
    if (detailLlm) {
      try {
        const llmConfig = await tauriInvoke('get_llm_config');
        if (llmConfig && llmConfig.configured) {
          detailLlm.textContent = `${llmConfig.llm_type || ''} ${llmConfig.model || ''}`.trim();
          detailLlm.style.color = '';
        } else {
          detailLlm.textContent = '未配置';
          detailLlm.style.color = 'var(--text-secondary)';
        }
      } catch (e) {
        detailLlm.textContent = detailLlm.textContent || '—';
      }
    }

    // MCP 连接状态 — 基于 Agent 配置和 Sidecar 状态判断
    if (detailMcp) {
      if (status && status.running && config.selectedAgents.length > 0) {
        detailMcp.textContent = '已配置（等待 AI 工具连接）';
        detailMcp.style.color = '';
      } else if (status && status.running) {
        detailMcp.textContent = '未配置 AI 工具';
        detailMcp.style.color = 'var(--warning)';
      } else {
        detailMcp.textContent = '服务未启动';
        detailMcp.style.color = 'var(--text-secondary)';
      }
    }
  }

  /** v0.5.4 P1-5 新增：记录最近活动 */
  function recordActivity(text) {
    lastActivityTime = Date.now();
    lastActivityText = text || '活动';
    // 立即刷新状态栏以更新时间显示
    pollStatus();
  }

  /** v0.5.4 P1-5 新增：状态栏展开/折叠 */
  function toggleStatusBarDetails() {
    const bar = $('status-bar');
    const details = $('status-bar-details');
    if (!bar || !details) return;

    const isExpanded = bar.classList.contains('expanded');
    if (isExpanded) {
      bar.classList.remove('expanded');
      details.style.display = 'none';
    } else {
      bar.classList.add('expanded');
      details.style.display = 'flex';
    }
  }

  // v0.5.4 P1-5 新增：状态栏点击事件（展开/折叠技术细节）
  const statusBarMain = $('status-bar-main');
  if (statusBarMain) {
    statusBarMain.addEventListener('click', toggleStatusBarDetails);
  }

  /** v0.5.4 新增：即时健康检查
   * 配置完成后立即验证 sidecar 是否正常响应，用户无需等待轮询间隔。
   * 最多重试 3 次（每次间隔 1 秒），避免 sidecar 刚启动尚未就绪的误报。
   */
  async function immediateHealthCheck() {
    const port = config.port || 3099;
    const maxRetries = 3;
    for (let i = 0; i < maxRetries; i++) {
      try {
        const resp = await fetch(`http://127.0.0.1:${port}/health`, {
          signal: AbortSignal.timeout(3000),
        });
        if (resp.ok) {
          return { running: true, port };
        }
      } catch (e) {
        // 重试期间静默，最后一次失败才记录
        if (i === maxRetries - 1) {
          console.warn('[健康检查] /health 端点不可达 (port=' + port + '):', e.message);
        }
      }
      if (i < maxRetries - 1) {
        await new Promise((r) => setTimeout(r, 1000));
      }
    }
    return { running: false, port };
  }

  /** 轮询 sidecar 状态（每 5 秒）
   * v0.5.4 修复：get_sidecar_status 返回的是数组，需取第一个元素
   * 同时直接调用 /health 端点作为双重验证，解决状态不一致问题 */
  async function pollStatus() {
    try {
      const statuses = await tauriInvoke('get_sidecar_status');
      // v0.5.4 修复：get_sidecar_status 返回数组，取第一个运行中的实例
      const status = (statuses && statuses.length > 0) ? statuses[0] : null;
      
      // v0.5.4 新增：直接调用 /health 端点双重验证
      // 避免主界面说"未启动"但仪表盘说"运行中"的状态不一致
      let healthOk = false;
      const port = status ? status.port : config.port;
      if (port) {
        try {
          const resp = await fetch(`http://127.0.0.1:${port}/health`);
          healthOk = resp.ok;
        } catch (e) {
          // /health 不可达，服务确实未启动
        }
      }
      
      // 综合判断：Tauri 状态 + 直接 health 检查
      updateStatusBar({
        running: status ? status.running : false,
        port: status ? status.port : null,
        state: status ? status.state : 'Stopped',
        healthOk: healthOk,
      });
    } catch (e) {
      // 静默处理（非 Tauri 环境或服务未初始化）
    }
  }

  /** 启动状态轮询 */
  function startStatusPolling() {
    if (statusPollTimer) return;
    // 首次立即检查
    pollStatus();
    // 每 5 秒轮询
    statusPollTimer = setInterval(pollStatus, 5000);
  }

  /** 停止状态轮询 */
  function stopStatusPolling() {
    if (statusPollTimer) {
      clearInterval(statusPollTimer);
      statusPollTimer = null;
    }
    // v0.5.4 修复：同时清除就绪面板轮询定时器
    if (readyPanelPollTimer) {
      clearInterval(readyPanelPollTimer);
      readyPanelPollTimer = null;
    }
  }

  // 启动轮询（在 init 完成后调用）
  const originalInit = init;
  window.Wizard.init = async function () {
    await originalInit();
    startStatusPolling();
    // 完成后更新 LLM 状态
    updateStatusBar(null);
  };

  // v0.5.4 修复：全局事件委托，替代内联 onclick 调用，减少全局命名空间污染
  document.addEventListener('click', function (ev) {
    const target = ev.target.closest('[data-action]');
    if (!target) return;
    const action = target.getAttribute('data-action');
    if (action === 'open-settings') {
      ev.preventDefault();
      showSettingsPanel();
    }
  });

  // v0.5.4 P1-5 修复：暴露 recordActivity 到 Wizard 命名空间
  window.Wizard.recordActivity = recordActivity;

  // v0.5.4 修复：_wizUpdateStatus 改为局部函数，无需全局命名空间
  const _wizUpdateStatus = function () {
    pollStatus();
    updateStatusBar(null);
  };

  // ── 自动启动 ──
  // Tauri Release 环境：module import 可能不可用，直接使用 __TAURI_INTERNALS__ 回退
  // 测试环境：window.__TAURI_INVOKE__ 由 test_ui.html 的 Mock 设置
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => window.Wizard.init());
  } else {
    window.Wizard.init();
  }
  console.log('[配置向导] 自动启动完成');

  // v0.5.5 P1-2：监听后端 open-settings 事件，从仪表盘打开设置面板
  // 仪表盘嵌入模式下，用户点击"修改配置"按钮 → 后端发送 open-settings 事件 → 前端打开设置面板
  tauriListen('open-settings', () => {
    console.log('[配置向导] 收到 open-settings 事件，打开设置面板');
    showSettingsPanel();
  }).catch(e => {
    console.warn('[配置向导] 监听 open-settings 事件失败:', e);
  });

  // v0.5.4 修复：页面卸载时清理所有定时器，防止内存泄漏
  window.addEventListener('beforeunload', () => {
    stopStatusPolling();
    console.log('[配置向导] 已清理所有定时器');
  });
})();
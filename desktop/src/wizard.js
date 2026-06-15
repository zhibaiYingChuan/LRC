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
    multiWindowEnabled: false, // 同一项目多窗口同时记录（上限 3 个）
    // Sidecar 端口
    port: 3099,
  };

  // ── 工具函数 ──
  const $ = (id) => document.getElementById(id);

  async function tauriInvoke(cmd, args = {}) {
    if (window.__TAURI_INVOKE__) {
      return window.__TAURI_INVOKE__(cmd, args);
    }
    // Tauri v2 真实 API（桌面端打包后使用）
    if (window.__TAURI_INTERNALS__?.invoke) {
      return window.__TAURI_INTERNALS__.invoke(cmd, args);
    }
    console.warn('[LRC] 非 Tauri 环境，使用 HTTP fallback');
    return null;
  }

  // ── 步骤导航 ──
  function goToStep(step) {
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

    if (step === 2) loadProjects();
  }

  // ── 步骤 1：自动检测 Agent ──
  async function loadAgents() {
    const listEl = $('agent-list');
    listEl.innerHTML = '<div class="loading">正在扫描 AI 工具...</div>';

    try {
      let agents = await tauriInvoke('detect_agents');
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
        { key: 'ide', title: 'IDE 内嵌 Agent（可管理多个项目）', desc: '勾选后扫描项目列表' },
        { key: 'desktop', title: '独立桌面应用', desc: '勾选后直接配置 MCP 连接' },
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
          html += `
            <label class="agent-item ${installed ? '' : 'disabled'}">
              <input type="checkbox" value="${agent.id}"
                data-category="${cat.key}"
                ${installed ? 'checked' : ''}
                ${!installed ? 'disabled' : ''}
                onchange="window._wizToggleAgent('${agent.id}', this.checked)">
              <span class="agent-icon">${agent.icon}</span>
              <div class="agent-info">
                <span class="agent-name">${agent.name}</span>
                ${isIDE && installed ? '<span class="ide-badge">含项目列表</span>' : ''}
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
      syncInitialAgentSelection();  // 同步初始勾选状态
      $('btn-step-1-next').disabled = !hasInstalled;

      if (hasInstalled) {
        $('btn-step-1-next').textContent = '下一步：扫描项目 →';
      } else {
        listEl.innerHTML += '<div class="no-agents">未检测到已安装的 AI 工具。<br>请先安装 Trae、Cursor 或 VS Code 等 IDE。</div>';
      }
    } catch (e) {
      listEl.innerHTML = `<div class="error-message">检测失败：${e.message || e}</div>`;
    }
  }

  // 全局函数（checkbox onchange 回调）
  window._wizToggleAgent = function (id, checked) {
    if (checked) {
      if (!config.selectedAgents.includes(id)) {
        config.selectedAgents.push(id);
      }
    } else {
      config.selectedAgents = config.selectedAgents.filter((a) => a !== id);
    }
  };

  // 初始化：从 Agent 数据直接同步 selectedAgents（不依赖 DOM 状态）
  function syncInitialAgentSelection() {
    // 已安装的 Agent 自动勾选
    config.allAgents.forEach((agent) => {
      if (agent.installed && !config.selectedAgents.includes(agent.id)) {
        config.selectedAgents.push(agent.id);
      }
    });
  }

  $('btn-step-1-next').addEventListener('click', () => {
    if (config.selectedAgents.length > 0) {
      goToStep(2);
    }
  });

  // ── 步骤 2：IDE 项目选择 + 单应用直接配置 ──
  async function loadProjects() {
    const listEl = $('project-list');
    listEl.innerHTML = '<div class="loading">正在扫描项目...</div>';

    // 区分 IDE 和桌面应用
    const ideAgents = config.selectedAgents.filter((id) => {
      const agent = config.allAgents.find((a) => a.id === id);
      return agent && agent.category === 'ide';
    });
    const desktopAgents = config.selectedAgents.filter((id) => {
      const agent = config.allAgents.find((a) => a.id === id);
      return agent && (agent.category === 'desktop' || agent.category === 'custom');
    });

    let html = '';

    // 扫描 IDE 项目
    if (ideAgents.length > 0) {
      try {
        const projects = await tauriInvoke('scan_ide_projects', { ideIds: ideAgents });
        if (projects && projects.length > 0) {
          // 按 IDE 分组
          const grouped = {};
          for (const p of projects) {
            if (!grouped[p.ide_id]) grouped[p.ide_id] = { name: p.ide_name, projects: [] };
            grouped[p.ide_id].projects.push(p);
          }

          for (const [ideId, group] of Object.entries(grouped)) {
            html += `<div class="project-group">
              <div class="project-group-header">${group.name} 项目 (${group.projects.length})</div>`;
            for (const p of group.projects) {
              html += `
                <label class="project-item">
                  <input type="checkbox" value="${p.path}" checked
                    data-ide="${ideId}"
                    onchange="window._wizToggleProject('${p.path.replace(/'/g, "\\'")}', this.checked)">
                  <div class="project-info">
                    <span class="project-name">${p.name}</span>
                    <span class="project-path">${p.path}</span>
                  </div>
                </label>`;
            }
            html += '</div>';
          }
        } else {
          html += `<div class="no-projects">
            <p>未扫描到 IDE 项目。</p>
            <p class="hint">请确保你的 IDE 中已打开至少一个项目，或手动输入项目路径：</p>
            <div class="form-group">
              <div class="input-with-button">
                <input type="text" id="manual-project-dir" class="form-input"
                  placeholder="输入项目目录路径...">
                <button id="btn-add-project" class="btn btn-secondary">添加</button>
              </div>
            </div>
          </div>`;
        }
      } catch (e) {
        html += `<div class="error-message">项目扫描失败：${e.message || e}</div>`;
      }
    }

    // 桌面应用：直接显示配置状态
    if (desktopAgents.length > 0) {
      html += '<div class="project-group"><div class="project-group-header">独立应用（直接配置 MCP）</div>';
      for (const id of desktopAgents) {
        const agent = config.allAgents.find((a) => a.id === id);
        if (agent) {
          html += `
            <div class="desktop-agent-item">
              <span class="agent-icon">${agent.icon}</span>
              <span class="agent-name">${agent.name}</span>
              <span class="agent-status installed">将自动配置</span>
            </div>`;
        }
      }
      html += '</div>';
    }

    listEl.innerHTML = html || '<div class="no-projects">未选择任何 Agent，请返回上一步。</div>';

    // 同步项目初始勾选状态
    syncInitialProjectSelection();

    // 手动添加项目按钮
    setTimeout(() => {
      const btnAdd = $('btn-add-project');
      if (btnAdd) {
        btnAdd.addEventListener('click', () => {
          const dir = $('manual-project-dir').value.trim();
          if (dir) addManualProject(dir);
        });
      }
    }, 0);
  }

  window._wizToggleProject = function (path, checked) {
    if (checked) {
      if (!config.selectedProjects.includes(path)) {
        config.selectedProjects.push(path);
      }
    } else {
      config.selectedProjects = config.selectedProjects.filter((p) => p !== path);
    }
  };

  // 同步项目初始勾选状态
  function syncInitialProjectSelection() {
    const checkboxes = document.querySelectorAll('.project-item input[type="checkbox"]:checked');
    checkboxes.forEach((cb) => {
      if (!config.selectedProjects.includes(cb.value)) {
        config.selectedProjects.push(cb.value);
      }
    });
  }

  function addManualProject(dir) {
    config.selectedProjects.push(dir);
    const name = dir.split('\\').pop() || dir;
    const listEl = $('project-list');
    const html = `
      <label class="project-item">
        <input type="checkbox" value="${dir}" checked onchange="window._wizToggleProject('${dir.replace(/'/g, "\\'")}', this.checked)">
        <div class="project-info">
          <span class="project-name">${name} (手动添加)</span>
          <span class="project-path">${dir}</span>
        </div>
      </label>`;
    listEl.insertAdjacentHTML('beforeend', html);
    $('manual-project-dir').value = '';
  }

  // ── LLM 配置（傻瓜化一键配置）──
  // 常见提供商的 API 地址、默认模型、获取 Key 的链接
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

  // 提供商切换：自动填写 API 地址和默认模型
  function onProviderChange() {
    const provider = $('llm-provider').value;
    config.llmProvider = provider;
    const info = LLM_PROVIDERS[provider];

    if (provider === 'ollama') {
      $('llm-api-section').style.display = 'none';
      $('ollama-fields').style.display = 'block';
      $('ollama-model-field').style.display = 'block';
      // 自动检测本地 Ollama
      autoDetectOllama();
    } else {
      $('llm-api-section').style.display = 'block';
      $('ollama-fields').style.display = 'none';
      $('ollama-model-field').style.display = 'none';
      // 自动填写 API 地址（模型名称不自动填写，用户需手动确认）
      if (info) {
        $('llm-base-url').value = info.url;
        $('llm-base-url').placeholder = info.url || 'https://your-api.com/v1';
        // 仅更新 placeholder 提示推荐的模型名，不自动填充
        // 不同模型 token 价格差异大，用户必须自己确认
        $('llm-model').placeholder = info.model
          ? `推荐：${info.model}（请手动填写确认）`
          : '请输入模型名称';
        // 更新模型提示
        const modelHint = $('llm-model-hint');
        if (modelHint) {
          modelHint.textContent = info.model
            ? `${info.name} 推荐模型：${info.model}。不同模型 token 价格不同，请确认后填写`
            : '不同模型 token 价格差异大，请确认后填写。测试连接成功后会自动检测可用模型';
        }
        // 更新 Key 提示和获取链接
        updateKeyHint(info);
      }
      // 清空之前的测试结果
      clearTestResult();
    }
  }

  // 更新 API Key 输入提示和获取链接
  function updateKeyHint(info) {
    const hintEl = $('llm-key-hint');
    const linkEl = $('llm-key-link');
    if (hintEl && info) {
      hintEl.textContent = info.keyHint ? `格式：${info.keyHint}` : '';
    }
    if (linkEl && info) {
      if (info.keyUrl) {
        linkEl.href = info.keyUrl;
        linkEl.style.display = 'inline';
        linkEl.textContent = `获取 ${$('llm-provider').selectedOptions[0]?.text?.split(' ')[0] || ''} API Key →`;
      } else {
        linkEl.style.display = 'none';
      }
    }
  }

  // 自动检测本地 Ollama 实例
  async function autoDetectOllama() {
    const resultEl = $('llm-test-result');
    try {
      const resp = await fetch('http://localhost:11434/api/tags', {
        signal: AbortSignal.timeout(2000),
      });
      if (resp.ok) {
        const data = await resp.json();
        const models = (data.models || []).map(m => m.name);
        if (models.length > 0) {
          config.ollamaModel = models[0];
          $('ollama-model').value = models[0];
          showTestResult('success',
            `检测到 Ollama 正在运行，已安装 ${models.length} 个模型：${models.slice(0, 3).join(', ')}${models.length > 3 ? '...' : ''}`);
        }
      }
    } catch (e) {
      // Ollama 未运行，静默处理
      showTestResult('warning', '未检测到本地 Ollama 服务，请确保 Ollama 已启动');
    }
  }

  // 测试连接：通过 Tauri 后端代理验证 API Key（避免浏览器 CSP 限制）
  async function testLLMConnection() {
    const provider = $('llm-provider').value;
    const btn = $('btn-test-llm');
    btn.disabled = true;
    btn.textContent = '测试中...';

    try {
      if (provider === 'ollama') {
        const url = $('ollama-url').value || 'http://localhost:11434';
        const result = await tauriInvoke('test_llm_connection', {
          provider: 'ollama',
          apiKey: '',
          baseUrl: url,
          model: null,
        });
        showTestResult(result.success ? 'success' : 'error', result.message);
        if (result.models?.length > 0) {
          $('llm-model').value = result.models[0];
        }
      } else {
        const apiKey = $('llm-api-key').value;
        if (!apiKey) {
          showTestResult('warning', '请先输入 API Key');
          return;
        }
        const baseUrl = $('llm-base-url').value || LLM_PROVIDERS[provider]?.url;
        if (!baseUrl) {
          showTestResult('warning', '请先输入 API 地址');
          return;
        }
        const model = $('llm-model').value || LLM_PROVIDERS[provider]?.model || null;

        const result = await tauriInvoke('test_llm_connection', {
          provider,
          apiKey,
          baseUrl,
          model,
        });

        showTestResult(result.success ? 'success' : 'error', result.message);
        if (result.models?.length > 0) {
          $('llm-model').value = result.models[0];
        }
      }
    } catch (e) {
      showTestResult('error', `连接测试失败：${e.message || e}`);
    } finally {
      btn.disabled = false;
      btn.textContent = '测试连接';
    }
  }

  // 显示测试结果
  function showTestResult(type, message) {
    const el = $('llm-test-result');
    el.style.display = 'block';
    el.className = `llm-test-result test-${type}`;
    el.textContent = `${type === 'success' ? '✅' : type === 'error' ? '❌' : '⚠️'} ${message}`;
  }

  // 清空测试结果
  function clearTestResult() {
    const el = $('llm-test-result');
    el.style.display = 'none';
    el.textContent = '';
  }

  $('llm-provider').addEventListener('change', onProviderChange);
  $('btn-test-llm')?.addEventListener('click', testLLMConnection);

  // 多窗口 LRC 记录开关
  $('multi-window-enabled').addEventListener('change', (e) => {
    config.multiWindowEnabled = e.target.checked;
    const info = $('multi-window-info');
    if (info) {
      info.style.display = e.target.checked ? 'block' : 'none';
    }
    console.log('[配置向导] 多窗口 LRC 记录:', config.multiWindowEnabled ? '已开启（上限3个）' : '已关闭');
  });

  // 上一步
  $('btn-step-2-prev').addEventListener('click', () => goToStep(1));

  // 完成配置
  $('btn-step-2-finish').addEventListener('click', async () => {
    const btn = $('btn-step-2-finish');
    btn.disabled = true;
    btn.textContent = '正在配置...';

    const isSwitchProject = window.location.hash === '#wizard-switch-project';

    // 切换项目模式：使用 switch_project 命令（会自动重启 sidecar）
    if (isSwitchProject && config.selectedProjects.length > 0) {
      try {
        const result = await tauriInvoke('switch_project', {
          projectDir: config.selectedProjects[0],
          multiWindow: config.multiWindowEnabled ? 3 : 1,
        });
        console.log('[配置向导] 项目切换结果:', result);
        config.port = (await tauriInvoke('get_sidecar_status')).port || config.port;
        btn.disabled = false;
        showSummary(true);
        goToStep('done');
        pollStatus();
        updateStatusBar(null);
        // 恢复标题
        document.querySelector('.wizard-container h1').textContent = 'LRC Desktop 已就绪';
        document.querySelector('.wizard-subtitle').textContent = '托盘图标已就绪，右键可打开菜单';
        return;
      } catch (e) {
        console.warn('[配置向导] 项目切换失败:', e);
        btn.disabled = false;
        showSummary(false);
        goToStep('done');
        return;
      }
    }

    // 首次配置模式：正常流程
    // 保存 LLM 配置
    let llmString = '';
    const provider = $('llm-provider').value;
    config.llmProvider = provider;

    if (provider === 'ollama') {
      config.ollamaModel = $('ollama-model').value || 'llama3';
      config.ollamaUrl = $('ollama-url').value || 'http://localhost:11434';
      llmString = `ollama:${config.ollamaModel}:${config.ollamaUrl}`;
    } else {
      config.llmApiKey = $('llm-api-key').value || null;
      config.llmModel = $('llm-model').value || 'gpt-4o';
      config.llmBaseUrl = $('llm-base-url').value || LLM_PROVIDERS[provider]?.url || 'https://api.openai.com/v1';
      if (config.llmApiKey) {
        llmString = `openai:${config.llmApiKey}:${config.llmModel}:${config.llmBaseUrl}`;
      }
    }
    if (llmString) {
      try { await tauriInvoke('save_llm_config', { llmApi: llmString }); } catch (e) {}
    }

    // 保存项目目录（以第一个项目为主目录）
    if (config.selectedProjects.length > 0) {
      try { await tauriInvoke('set_project_dir', { projectDir: config.selectedProjects[0] }); } catch (e) {}
    }

    // 启动 sidecar
    let sidecarStarted = false;
    try {
      const port = await tauriInvoke('start_sidecar', {
        srcDir: config.selectedProjects[0] || null,
        port: config.port || null,
        multiWindow: config.multiWindowEnabled ? 3 : 1,
      });
      if (port) {
        config.port = port;
        sidecarStarted = true;
      }
    } catch (e) {
      console.warn('[配置向导] Sidecar 启动失败:', e);
    }

    // 为选中的 Agent 配置 MCP
    if (config.selectedAgents.length > 0) {
      try {
        await tauriInvoke('configure_agents', {
          agentIds: config.selectedAgents,
          port: config.port,
        });
      } catch (e) {
        console.warn('[配置向导] Agent 配置失败:', e);
      }
    }

    btn.disabled = false;
    showSummary(sidecarStarted);
    goToStep('done');
    // 立即更新状态栏
    pollStatus();
    updateStatusBar(null);

    // ── 自动打开仪表盘（首次配置完成后无需手动点击）──
    if (sidecarStarted) {
      setTimeout(async () => {
        try {
          await tauriInvoke('navigate_main_to_dashboard');
        } catch (e) {
          console.warn('[配置向导] 自动导航失败:', e);
        }
      }, 1500);
    }
  });

  // ── 完成页 ──
  function showSummary(sidecarStarted) {
    const agentNames = {};
    config.allAgents.forEach((a) => { agentNames[a.id] = a.name; });

    const agentList = config.selectedAgents
      .map((id) => agentNames[id] || id)
      .join(', ');

    const projectList = config.selectedProjects
      .map((p) => p.split('\\').pop())
      .join(', ');

    const sidecarStatus = sidecarStarted
      ? '<span class="check">✅</span> 后台服务已启动（端口 ' + config.port + '）'
      : '<span class="check">⚠️</span> 后台服务启动失败，可稍后手动重启';

    $('config-summary').innerHTML = `
      ${sidecarStatus}
      <div class="summary-item"><span class="check">✅</span> 已连接 Agent：${agentList || '无'}</div>
      <div class="summary-item"><span class="check">✅</span> 索引项目：${projectList || '无'} (${config.selectedProjects.length} 个)</div>
      <div class="summary-item"><span class="check">✅</span> LLM：${config.llmApiKey ? config.llmModel : '未配置'}</div>
      <div class="summary-item"><span class="check">${config.multiWindowEnabled ? '🟢' : '⚪'}</span> 多窗口记录：${config.multiWindowEnabled ? '已开启（上限 3 个）' : '未开启'}</div>
      <div class="summary-item"><span class="check">🔒</span> 数据安全：所有数据存储在本地，绝不上传</div>`;

    // 根据 sidecar 状态更新按钮
    const btn = $('btn-open-dashboard');
    if (sidecarStarted) {
      btn.disabled = false;
      btn.textContent = '打开仪表盘';
      btn.title = `http://127.0.0.1:${config.port}/dashboard`;
    } else {
      btn.disabled = false;
      btn.textContent = '重试启动服务';
      btn.title = '点击重新尝试启动后台服务';
    }
  }

  $('btn-open-dashboard').addEventListener('click', async () => {
    const btn = $('btn-open-dashboard');

    // 如果按钮显示"重试启动服务"，先尝试启动 sidecar
    if (btn.textContent === '重试启动服务') {
      btn.disabled = true;
      btn.textContent = '正在启动...';
      try {
        const port = await tauriInvoke('start_sidecar', {
          srcDir: config.selectedProjects[0] || null,
          port: config.port || null,
          multiWindow: config.multiWindowEnabled ? 3 : 1,
        });
        if (port) {
          config.port = port;
          btn.textContent = '打开仪表盘';
          btn.title = `http://127.0.0.1:${config.port}/dashboard`;
        }
      } catch (e) {
        btn.textContent = '启动失败，请检查日志';
        btn.disabled = false;
        return;
      }
      btn.disabled = false;
    }

    // 打开仪表盘（带健康检查，最多等待 3 秒）
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
        // 等待 500ms 后重试
        await new Promise((r) => setTimeout(r, 500));
      }
    }

    if (isReady) {
      // 导航主窗口到仪表盘（同一窗口内过渡，不弹新窗口）
      try {
        await tauriInvoke('navigate_main_to_dashboard');
        btn.textContent = '仪表盘已打开';
      } catch (e) {
        // 回退：创建新窗口
        console.warn('[配置向导] 主窗口导航失败，回退到新窗口:', e);
        try {
          await tauriInvoke('open_dashboard_window');
        } catch (e2) {
          // 最终回退：外部浏览器打开
          if (window.__TAURI_SHELL_OPEN__) {
            window.__TAURI_SHELL_OPEN__(url);
          } else {
            window.open(url, '_blank');
          }
        }
        btn.textContent = '仪表盘已打开';
      }
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

      // 2. 启动新的 sidecar
      btn.textContent = '正在启动服务...';
      const port = await tauriInvoke('start_sidecar', {
        srcDir: config.selectedProjects[0] || null,
        port: config.port || null,
        multiWindow: config.multiWindowEnabled ? 3 : 1,
      });

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
  async function init() {
    // 检查是否从托盘"切换项目"进入
    const isSwitchProject = window.location.hash === '#wizard-switch-project';

    try {
      const state = await tauriInvoke('get_wizard_state');
      if (state && state.setup_complete && !isSwitchProject) {
        config.selectedAgents = state.configured_agents || [];
        config.selectedProjects = state.project_dir ? [state.project_dir] : [];
        showReadyPanel(state);
        return;
      }
    } catch (e) {
      console.warn('[配置向导] 状态检查失败:', e);
    }

    // 从托盘"切换项目"进入 → 直接跳到步骤2
    if (isSwitchProject) {
      document.querySelector('.wizard-container h1').textContent = '切换项目';
      document.querySelector('.wizard-subtitle').textContent = '选择新项目，LRC 将重新索引代码';
      // 标记步骤1为已完成
      document.querySelectorAll('.wizard-steps .step').forEach((s, i) => {
        if (i === 0) s.classList.add('done');
        if (i === 1) s.classList.add('active');
      });
      // 预先加载 Agent 列表
      try {
        const agents = await tauriInvoke('detect_agents');
        if (agents) config.allAgents = agents;
      } catch (e) {}
      goToStep(2);
      return;
    }

    goToStep(1);
    loadAgents();
  }

  async function showReadyPanel(state) {
    $('wizard-steps').style.display = 'none';
    document.querySelectorAll('.wizard-panel').forEach((p) => p.classList.remove('active'));

    // 检查 sidecar 是否在运行
    const sidecarPort = state.sidecar_port || config.port;
    config.port = sidecarPort;

    // 恢复已保存的选中项目
    if (state.project_dir) {
      config.selectedProjects = [state.project_dir];
    }
    if (state.configured_agents) {
      config.selectedAgents = state.configured_agents;
    }

    $('config-summary').innerHTML = `
      <div class="summary-item"><span class="check">✅</span> 项目目录：${state.project_dir || '未设置'}</div>
      <div class="summary-item"><span class="check">✅</span> LLM 配置：${state.llm_configured ? '已配置' : '未配置'}</div>
      <div class="summary-item"><span class="check">✅</span> 已连接 Agent：${(state.configured_agents || []).join(', ') || '无'}</div>
      <div class="summary-item"><span class="check">${state.sidecar_running ? '✅' : '⚠️'}</span> 后台服务：${state.sidecar_running ? '运行中（端口 ' + sidecarPort + '）' : '未启动'}</div>
      <div class="summary-item"><span class="check">🔒</span> 数据安全：所有数据存储在本地，绝不上传</div>`;

    document.querySelector('.wizard-container h1').textContent = 'LRC Desktop 已就绪';
    document.querySelector('.wizard-subtitle').textContent = '托盘图标已就绪，右键可打开菜单';

    // 更新仪表盘按钮
    const btn = $('btn-open-dashboard');
    if (state.sidecar_running) {
      btn.disabled = false;
      btn.textContent = '打开仪表盘';
      btn.title = `http://127.0.0.1:${sidecarPort}/dashboard`;
    } else {
      btn.disabled = false;
      btn.textContent = '启动服务并打开仪表盘';
      btn.title = '点击启动后台服务';
    }

    const donePanel = $('panel-step-done');
    if (donePanel) donePanel.classList.add('active');

    // ── 自动启动：如果已配置但 sidecar 未运行，自动启动 ──
    if (!state.sidecar_running && state.project_dir) {
      console.log('[配置向导] 自动启动后台服务...');
      btn.textContent = '正在自动启动服务...';
      btn.disabled = true;
      try {
        const port = await tauriInvoke('start_sidecar', {
          srcDir: state.project_dir,
          port: config.port || null,
          multiWindow: config.multiWindowEnabled ? 3 : 1,
        });
        if (port) {
          config.port = port;
          console.log('[配置向导] 后台服务已自动启动，端口:', port);
          // 更新摘要显示
          $('config-summary').innerHTML = $('config-summary').innerHTML.replace(
            /后台服务：⚠️.*?<\/div>/,
            `后台服务：✅ 运行中（端口 ${port}）</div>`
          );
          btn.textContent = '打开仪表盘';
          btn.disabled = false;

          // 自动导航到仪表盘
          setTimeout(async () => {
            try {
              await tauriInvoke('navigate_main_to_dashboard');
            } catch (e) {
              console.warn('[配置向导] 自动导航失败:', e);
            }
          }, 1500);
        }
      } catch (e) {
        console.warn('[配置向导] 自动启动失败:', e);
        btn.textContent = '启动服务并打开仪表盘';
        btn.disabled = false;
      }
    } else if (state.sidecar_running) {
      // sidecar 已经在运行，直接自动导航到仪表盘
      setTimeout(async () => {
        try {
          await tauriInvoke('navigate_main_to_dashboard');
        } catch (e) {
          console.warn('[配置向导] 自动导航失败:', e);
        }
      }, 1000);
    }
  }

  // 暴露到 window 以便测试和外部调用
  window.Wizard = { init };
  window.LLM_PROVIDERS = LLM_PROVIDERS;
  window._wizOnProviderChange = onProviderChange;
  console.log('[配置向导] Wizard 模块已加载，等待 Tauri API 就绪...');

  // ── 左下角状态栏：实时轮询 sidecar 状态 ──
  let statusPollTimer = null;

  /** 更新状态栏 UI */
  function updateStatusBar(status) {
    // Sidecar 状态
    const dotEl = document.querySelector('#status-sidecar .status-dot');
    const textEl = document.querySelector('#status-sidecar .status-text');
    if (dotEl && textEl && status) {
      dotEl.className = 'status-dot';
      if (status.running) {
        dotEl.classList.add('online');
        textEl.textContent = `服务运行中 :${status.port || '?'}`;
      } else if (status.state && status.state.includes('Starting')) {
        dotEl.classList.add('warning');
        textEl.textContent = '服务启动中...';
      } else {
        dotEl.classList.add('offline');
        textEl.textContent = '服务未启动';
      }
    }

    // LLM 状态
    const llmDot = document.querySelector('#status-llm .status-dot');
    const llmText = document.querySelector('#status-llm .status-text');
    if (llmDot && llmText) {
      llmDot.className = 'status-dot';
      if (config.llmApiKey || config.llmProvider === 'ollama') {
        llmDot.classList.add('online');
        llmText.textContent = `LLM: ${config.llmProvider} ${config.llmModel || ''}`;
      } else {
        llmDot.classList.add('offline');
        llmText.textContent = 'LLM 未配置';
      }
    }

    // Agent 状态
    const agentDot = document.querySelector('#status-agents .status-dot');
    const agentText = document.querySelector('#status-agents .status-text');
    if (agentDot && agentText) {
      agentDot.className = 'status-dot';
      if (config.selectedAgents.length > 0) {
        agentDot.classList.add('online');
        agentText.textContent = `${config.selectedAgents.length} 个 Agent`;
      } else {
        agentDot.classList.add('offline');
        agentText.textContent = '无 Agent';
      }
    }
  }

  /** 轮询 sidecar 状态（每 5 秒） */
  async function pollStatus() {
    try {
      const status = await tauriInvoke('get_sidecar_status');
      if (status) {
        updateStatusBar(status);
      }
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
  }

  // 启动轮询（在 init 完成后调用）
  const originalInit = init;
  window.Wizard.init = async function () {
    await originalInit();
    startStatusPolling();
    // 完成后更新 LLM 状态
    updateStatusBar(null);
  };

  // 在完成配置后更新状态栏
  const originalShowSummary = showSummary;
  window._wizUpdateStatus = function () {
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
})();
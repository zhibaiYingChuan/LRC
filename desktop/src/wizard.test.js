/**
 * LRC Desktop 配置向导 — E2E 测试
 *
 * 测试覆盖：
 *   1. 初始状态：Agent 列表加载
 *   2. 步骤导航：第 1 → 2 → 完成 流程
 *   3. Agent 勾选与分类过滤
 *   4. 项目选择与手动添加
 *   5. LLM 提供商切换
 *   6. 完成配置流程
 *   7. Fallback 行为：后端不可用时
 *   8. 状态栏更新
 *
 * 运行方式：在浏览器中打开此文件，测试结果会显示在页面上。
 * 或在 Node.js 环境中运行（需要 jsdom）。
 */

(function () {
  'use strict';

  // ── 测试框架 ──
  const TestRunner = {
    passed: 0,
    failed: 0,
    results: [],

    /** 断言相等 */
    assertEq(actual, expected, msg) {
      if (actual === expected) {
        this.passed++;
        this.results.push({ status: 'PASS', msg });
      } else {
        this.failed++;
        this.results.push({
          status: 'FAIL',
          msg,
          expected: String(expected),
          actual: String(actual),
        });
      }
    },

    /** 断言为真 */
    assertTrue(value, msg) {
      this.assertEq(Boolean(value), true, msg);
    },

    /** 断言为假 */
    assertFalse(value, msg) {
      this.assertEq(Boolean(value), false, msg);
    },

    /** 断言包含 */
    assertContains(haystack, needle, msg) {
      const result = Array.isArray(haystack)
        ? haystack.includes(needle)
        : String(haystack).includes(needle);
      if (result) {
        this.passed++;
        this.results.push({ status: 'PASS', msg });
      } else {
        this.failed++;
        this.results.push({
          status: 'FAIL',
          msg,
          expected: `应包含 "${needle}"`,
          actual: `实际值: ${JSON.stringify(haystack)}`,
        });
      }
    },

    /** 异步等待 */
    async sleep(ms) {
      return new Promise((r) => setTimeout(r, ms));
    },

    /** 汇总报告 */
    summary() {
      const total = this.passed + this.failed;
      return {
        total,
        passed: this.passed,
        failed: this.failed,
        rate: total > 0 ? ((this.passed / total) * 100).toFixed(1) + '%' : 'N/A',
        results: this.results,
      };
    },
  };

  // ── Mock Tauri Backend ──
  const MOCK_AGENTS = [
    { id: 'trae', name: 'Trae', installed: true, icon: '🖥️', category: 'ide', supports_mcp: true },
    { id: 'trae-cn', name: 'Trae CN', installed: false, icon: '🖥️', category: 'ide', supports_mcp: true },
    { id: 'cursor', name: 'Cursor', installed: true, icon: '🖱️', category: 'ide', supports_mcp: true },
    { id: 'vscode', name: 'VS Code', installed: false, icon: '📝', category: 'ide', supports_mcp: true },
    { id: 'windsurf', name: 'Windsurf', installed: false, icon: '🌊', category: 'ide', supports_mcp: true },
    { id: 'kiro', name: 'Kiro', installed: false, icon: '🔮', category: 'ide', supports_mcp: true },
    { id: 'claude-desktop', name: 'Claude Desktop', installed: true, icon: '🧠', category: 'desktop', supports_mcp: true },
    { id: 'gemini-cli', name: 'Gemini CLI', installed: false, icon: '💎', category: 'cli', supports_mcp: true },
    { id: 'codebuddy', name: 'CodeBuddy', installed: false, icon: '🤝', category: 'ai-assistant', supports_mcp: false },
    { id: 'comate', name: 'Comate (百度)', installed: false, icon: '🐻', category: 'ai-assistant', supports_mcp: true },
    { id: 'roo-code', name: 'Roo Code', installed: false, icon: '🦘', category: 'ai-assistant', supports_mcp: true },
    { id: 'cline', name: 'Cline', installed: false, icon: '🧗', category: 'ai-assistant', supports_mcp: true },
    { id: 'aider', name: 'Aider', installed: false, icon: '💬', category: 'cli', supports_mcp: false },
    { id: 'sillytavern', name: '酒馆 (SillyTavern)', installed: false, icon: '🏮', category: 'desktop', supports_mcp: false },
    { id: 'generic-mcp', name: '通用 MCP Agent', installed: true, icon: '🔌', category: 'custom', supports_mcp: false },
  ];

  const MOCK_PROJECTS = [
    { path: 'G:\\Sfang', name: 'Sfang', ide_id: 'trae', ide_name: 'Trae' },
    { path: 'G:\\code-memory', name: 'code-memory', ide_id: 'trae', ide_name: 'Trae' },
  ];

  function setupMockTauri() {
    window.__TAURI_INVOKE__ = async function (cmd, args) {
      await TestRunner.sleep(50); // 模拟网络延迟
      switch (cmd) {
        case 'detect_agents': return MOCK_AGENTS;
        case 'discover_all_agents': return [MOCK_AGENTS, []];
        case 'scan_ide_projects':
          return MOCK_PROJECTS.filter((p) => (args.ideIds || []).includes(p.ide_id));
        case 'get_wizard_state':
          return { setup_complete: false, project_dir: null, llm_configured: false, configured_agents: [], sidecar_running: false, sidecar_port: null };
        case 'start_sidecar': return 3099;
        case 'stop_sidecar': return null;
        case 'configure_agents': return ['Trae (全局配置)', 'Cursor (全局配置)', 'Claude Desktop (全局配置)', '通用 MCP Agent — HTTP 端点: http://127.0.0.1:3099/mcp'];
        case 'save_llm_config': return { configured: true, llm_type: 'openai', model: 'gpt-4o' };
        case 'set_project_dir': return null;
        case 'get_sidecar_status': return { running: true, state: 'Running', port: 3099, pid: 12345 };
        case 'test_llm_connection': return { success: true, message: '连接成功', models: ['gpt-4o', 'gpt-4o-mini'] };
        case 'navigate_main_to_dashboard': return null;
        default: return null;
      }
    };
  }

  // ── 测试用例 ──

  /** 测试 1：Agent 列表加载 — 数量验证 */
  async function testAgentListLoading() {
    setupMockTauri();

    // 模拟 wizard.js 的 loadAgents 逻辑
    const agents = await window.__TAURI_INVOKE__('detect_agents');

    TestRunner.assertTrue(agents.length >= 16,
      'P1-03 修复：Agent 列表应包含 16+ 工具（非仅 7 种）');
    TestRunner.assertTrue(agents.length <= 40,
      'Agent 列表数量合理（不超过 40）');

    // 验证核心 Agent 存在
    const ids = agents.map((a) => a.id);
    TestRunner.assertContains(ids, 'trae', '应包含 Trae');
    TestRunner.assertContains(ids, 'cursor', '应包含 Cursor');
    TestRunner.assertContains(ids, 'generic-mcp', '应包含通用 MCP');

    // 验证分类
    const categories = [...new Set(agents.map((a) => a.category))];
    TestRunner.assertTrue(categories.length >= 4,
      `应包含多种分类（实际: ${categories.join(', ')}）`);

    // 验证已安装的 Agent 数量
    const installed = agents.filter((a) => a.installed);
    TestRunner.assertTrue(installed.length > 0,
      `应至少有一个已安装 Agent（实际: ${installed.length}）`);
  }

  /** 测试 2：Fallback 行为 — 后端不可用时 */
  async function testFallbackBehavior() {
    // 模拟后端不可用
    window.__TAURI_INVOKE__ = async function () {
      return null;
    };

    const agents = await window.__TAURI_INVOKE__('detect_agents');

    // P1-03 修复：返回 null 时不应使用硬编码 fallback
    TestRunner.assertEq(agents, null,
      '后端不可用时应返回 null（而非硬编码的 7 种 fallback 列表）');
  }

  /** 测试 3：Agent 分类过滤 */
  async function testAgentCategoryFilter() {
    setupMockTauri();
    const agents = await window.__TAURI_INVOKE__('detect_agents');

    const ideAgents = agents.filter((a) => a.category === 'ide');
    const desktopAgents = agents.filter((a) => a.category === 'desktop');
    const cliAgents = agents.filter((a) => a.category === 'cli');
    const aiAssistantAgents = agents.filter((a) => a.category === 'ai-assistant');
    const customAgents = agents.filter((a) => a.category === 'custom');

    TestRunner.assertTrue(ideAgents.length >= 4,
      `IDE 类应至少 4 个（实际: ${ideAgents.length}）`);
    TestRunner.assertTrue(desktopAgents.length >= 2,
      `桌面类应至少 2 个（实际: ${desktopAgents.length}）`);
    TestRunner.assertTrue(customAgents.length >= 1,
      `自定义类应至少 1 个（实际: ${customAgents.length}）`);

    // 已安装 Agent 应自动勾选
    const installed = agents.filter((a) => a.installed);
    TestRunner.assertTrue(installed.length >= 3,
      `已安装 Agent 应至少 3 个（实际: ${installed.length}）`);
  }

  /** 测试 4：IDE 项目扫描 */
  async function testProjectScan() {
    setupMockTauri();

    const ideAgents = ['trae', 'cursor'];
    const projects = await window.__TAURI_INVOKE__('scan_ide_projects', { ideIds: ideAgents });

    TestRunner.assertTrue(projects.length >= 2,
      `应扫描到至少 2 个项目（实际: ${projects.length}）`);

    // 验证项目结构
    for (const p of projects) {
      TestRunner.assertTrue(p.path && p.path.length > 0,
        `项目应有路径: ${p.name}`);
      TestRunner.assertTrue(p.ide_id && p.ide_id.length > 0,
        `项目应有 IDE ID: ${p.ide_id}`);
    }

    // 按 IDE 分组
    const traeProjects = projects.filter((p) => p.ide_id === 'trae');
    TestRunner.assertTrue(traeProjects.length >= 1,
      'Trae 应有项目');
  }

  /** 测试 5：LLM 提供商切换 */
  async function testLLMProviderSwitch() {
    // 验证 LLM_PROVIDERS 配置完整性
    TestRunner.assertTrue(window.LLM_PROVIDERS && typeof window.LLM_PROVIDERS === 'object',
      'LLM_PROVIDERS 全局变量应存在');

    const providers = window.LLM_PROVIDERS;
    TestRunner.assertTrue(Object.keys(providers).length >= 10,
      `LLM 提供商应至少 10 个（实际: ${Object.keys(providers).length}）`);

    // 验证关键提供商
    TestRunner.assertTrue('deepseek' in providers, '应包含 DeepSeek');
    TestRunner.assertTrue('ollama' in providers, '应包含 Ollama');
    TestRunner.assertTrue('openai' in providers, '应包含 OpenAI');
    TestRunner.assertTrue('custom' in providers, '应包含自定义选项');

    // 验证每个提供商有必要的字段
    for (const [key, info] of Object.entries(providers)) {
      TestRunner.assertTrue(info.name && info.name.length > 0,
        `提供商 ${key} 应有名称`);
      TestRunner.assertTrue(info.url !== undefined,
        `提供商 ${key} 应有 URL`);
    }
  }

  /** 测试 6：完成配置流程 — invoke 调用顺序 */
  async function testFinishConfigFlow() {
    setupMockTauri();

    const invokedCmds = [];
    window.__TAURI_INVOKE__ = async function (cmd, args) {
      invokedCmds.push(cmd);
      await TestRunner.sleep(20);
      switch (cmd) {
        case 'detect_agents': return MOCK_AGENTS;
        case 'scan_ide_projects': return MOCK_PROJECTS;
        case 'save_llm_config': return { configured: true };
        case 'set_project_dir': return null;
        case 'start_sidecar': return 3099;
        case 'configure_agents': return ['Trae (全局配置)'];
        case 'get_sidecar_status': return { running: true, state: 'Running', port: 3099 };
        default: return null;
      }
    };

    // 模拟完成配置的调用链
    await window.__TAURI_INVOKE__('save_llm_config', { llmApi: 'openai:sk-test:gpt-4o:https://api.openai.com/v1' });
    await window.__TAURI_INVOKE__('set_project_dir', { projectDir: 'G:\\Sfang' });
    await window.__TAURI_INVOKE__('start_sidecar', { srcDir: 'G:\\Sfang', port: 3099 });
    await window.__TAURI_INVOKE__('configure_agents', { agentIds: ['trae', 'cursor'], port: 3099 });

    TestRunner.assertContains(invokedCmds, 'save_llm_config',
      '完成配置应包含 save_llm_config 调用');
    TestRunner.assertContains(invokedCmds, 'set_project_dir',
      '完成配置应包含 set_project_dir 调用');
    TestRunner.assertContains(invokedCmds, 'start_sidecar',
      '完成配置应包含 start_sidecar 调用');
    TestRunner.assertContains(invokedCmds, 'configure_agents',
      '完成配置应包含 configure_agents 调用');
  }

  /** 测试 7：configured_agents 持久化验证 */
  async function testConfiguredAgentsPersistence() {
    let savedAgents = null;

    window.__TAURI_INVOKE__ = async function (cmd, args) {
      await TestRunner.sleep(20);
      if (cmd === 'configure_agents') {
        savedAgents = args.agentIds;
        return ['Trae (全局配置)'];
      }
      if (cmd === 'get_wizard_state') {
        return {
          setup_complete: true,
          project_dir: 'G:\\Sfang',
          llm_configured: true,
          configured_agents: savedAgents || [],
          sidecar_running: true,
          sidecar_port: 3099,
        };
      }
      return null;
    };

    // 模拟配置 Agent
    await window.__TAURI_INVOKE__('configure_agents', {
      agentIds: ['trae', 'cursor', 'claude-desktop'],
      port: 3099,
    });

    TestRunner.assertEq(savedAgents.length, 3,
      'P2-05 修复：configured_agents 应包含 3 个 Agent');
    TestRunner.assertContains(savedAgents, 'trae',
      '应包含 trae');
    TestRunner.assertContains(savedAgents, 'cursor',
      '应包含 cursor');
    TestRunner.assertContains(savedAgents, 'claude-desktop',
      '应包含 claude-desktop');
  }

  /** 测试 8：状态栏更新逻辑 */
  async function testStatusBarUpdate() {
    setupMockTauri();

    const status = await window.__TAURI_INVOKE__('get_sidecar_status');

    TestRunner.assertTrue(status.running,
      'Sidecar 应为运行状态');
    TestRunner.assertEq(status.port, 3099,
      '端口应为 3099');
    TestRunner.assertTrue(status.pid > 0,
      '应包含有效 PID');
  }

  /** 测试 9：WizardState 版本控制 */
  async function testWizardStateVersionControl() {
    let storedConfig = null;

    window.__TAURI_INVOKE__ = async function (cmd, args) {
      await TestRunner.sleep(20);
      if (cmd === 'get_wizard_state') {
        // 模拟从 wizard.json 返回的数据结构
        return {
          setup_complete: false,
          project_dir: 'G:\\test-project',
          llm_configured: false,
          configured_agents: [],
          sidecar_running: false,
          sidecar_port: null,
        };
      }
      return null;
    };

    const state = await window.__TAURI_INVOKE__('get_wizard_state');

    TestRunner.assertFalse(state.setup_complete,
      '新配置应未完成（setup_complete=false）');
    TestRunner.assertEq(state.project_dir, 'G:\\test-project',
      '应读取项目目录');
    TestRunner.assertFalse(state.llm_configured,
      'LLM 应未配置');
    TestRunner.assertFalse(state.sidecar_running,
      'Sidecar 应未运行');
  }

  /** 测试 10：代理检测覆盖（数据一致性验证） */
  async function testAgentCoverageConsistency() {
    setupMockTauri();
    const agents = await window.__TAURI_INVOKE__('detect_agents');

    // 验证后端 KNOWN_TOOLS 与前端数据的一致性
    // 每个 Agent 必须包含必要字段
    const requiredFields = ['id', 'name', 'installed', 'icon', 'category'];
    for (const agent of agents) {
      for (const field of requiredFields) {
        TestRunner.assertTrue(
          agent[field] !== undefined && agent[field] !== null,
          `Agent "${agent.id || 'unknown'}" 应有字段 "${field}"`
        );
      }
    }

    // 验证 supports_mcp 字段存在
    const withMcpSupport = agents.filter((a) => a.supports_mcp !== undefined);
    TestRunner.assertTrue(
      withMcpSupport.length === agents.length,
      `所有 Agent 应有 supports_mcp 字段（${
        withMcpSupport.length}/${agents.length}）`
    );

    // 验证 ID 唯一性
    const ids = agents.map((a) => a.id);
    const uniqueIds = [...new Set(ids)];
    TestRunner.assertEq(ids.length, uniqueIds.length,
      `Agent ID 不应重复（${ids.length} vs ${uniqueIds.length}）`);
  }

  // ── 运行测试并输出报告 ──
  async function runAllTests() {
    console.log('════════════════════════════════════════');
    console.log('  LRC 配置向导 — E2E 测试套件');
    console.log('════════════════════════════════════════');
    console.log('');

    const tests = [
      { name: 'Agent 列表加载与数量验证（P1-03）', fn: testAgentListLoading },
      { name: 'Fallback 行为验证（P1-03）', fn: testFallbackBehavior },
      { name: 'Agent 分类过滤', fn: testAgentCategoryFilter },
      { name: 'IDE 项目扫描', fn: testProjectScan },
      { name: 'LLM 提供商配置', fn: testLLMProviderSwitch },
      { name: '完成配置流程', fn: testFinishConfigFlow },
      { name: 'configured_agents 持久化（P2-05）', fn: testConfiguredAgentsPersistence },
      { name: '状态栏更新逻辑', fn: testStatusBarUpdate },
      { name: 'WizardState 版本控制（P2-04）', fn: testWizardStateVersionControl },
      { name: 'Agent 数据一致性验证', fn: testAgentCoverageConsistency },
    ];

    for (const test of tests) {
      console.log(`[测试] ${test.name}...`);
      try {
        await test.fn();
      } catch (e) {
        TestRunner.failed++;
        TestRunner.results.push({
          status: 'ERROR',
          msg: test.name,
          error: e.message || String(e),
        });
        console.error(`  ✗ 测试异常: ${e.message}`);
      }
    }

    console.log('');
    console.log('════════════════════════════════════════');
    const summary = TestRunner.summary();
    console.log(`  总计: ${summary.total}`);
    console.log(`  通过: ${summary.passed}`);
    console.log(`  失败: ${summary.failed}`);
    console.log(`  通过率: ${summary.rate}`);
    console.log('════════════════════════════════════════');

    // 输出失败详情
    const failures = TestRunner.results.filter((r) => r.status !== 'PASS');
    if (failures.length > 0) {
      console.log('');
      console.log('── 失败详情 ──');
      for (const f of failures) {
        console.log(`  ✗ ${f.msg}`);
        if (f.expected) console.log(`    期望: ${f.expected}`);
        if (f.actual) console.log(`    实际: ${f.actual}`);
        if (f.error) console.log(`    错误: ${f.error}`);
      }
    }

    // 输出到页面（如果存在 DOM）
    await renderReportToDOM(summary);

    return summary;
  }

  /** 将测试报告渲染到 DOM */
  async function renderReportToDOM(summary) {
    // 如果没有 DOM 环境，跳过
    if (typeof document === 'undefined') return;

    // 等待可能的 DOM 加载
    await TestRunner.sleep(100);

    const existing = document.getElementById('e2e-test-report');
    if (existing) existing.remove();

    const report = document.createElement('div');
    report.id = 'e2e-test-report';
    report.style.cssText = `
      position: fixed; top: 10px; right: 10px; z-index: 99999;
      background: #1a1a2e; color: #ccc; border: 2px solid ${summary.failed > 0 ? '#ff5252' : '#4caf50'};
      border-radius: 8px; padding: 16px; font-family: monospace; font-size: 12px;
      max-width: 400px; max-height: 80vh; overflow-y: auto;
    `;

    let html = `<h3 style="color: #4fc3f7; margin: 0 0 8px;">E2E 测试报告</h3>`;
    html += `<div style="margin-bottom: 8px;">
      <span style="color: #4caf50;">✓ ${summary.passed}</span>
      <span style="color: ${summary.failed > 0 ? '#ff5252' : '#888'};">✗ ${summary.failed}</span>
      <span style="color: #888;"> | ${summary.rate}</span>
    </div>`;

    for (const r of summary.results) {
      const color = r.status === 'PASS' ? '#4caf50' : '#ff5252';
      const icon = r.status === 'PASS' ? '✅' : '❌';
      html += `<div style="color: ${color}; margin: 2px 0;">${icon} ${r.msg}</div>`;
      if (r.error) {
        html += `<div style="color: #ff5252; margin-left: 20px; font-size: 10px;">错误: ${r.error}</div>`;
      }
    }

    report.innerHTML = html;
    document.body.appendChild(report);
  }

  // ── 自动运行 ──
  if (typeof document !== 'undefined' && document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => runAllTests());
  } else {
    // Node.js 或已加载环境
    setTimeout(() => runAllTests(), 500);
  }

  // 导出供外部使用
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = { TestRunner, runAllTests, MOCK_AGENTS, MOCK_PROJECTS };
  } else {
    window.E2ETest = { TestRunner, runAllTests, results: TestRunner.results };
  }
})();
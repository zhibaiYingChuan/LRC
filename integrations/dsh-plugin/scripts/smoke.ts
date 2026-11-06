/**
 * LRC 客户端冒烟测试（不依赖 DSH，也不依赖真实 LRC 服务）。
 *
 * 用法：node scripts/smoke.ts
 * 原理：在进程内起一个 mock HTTP 服务，验证 lrc-client 的请求形状与错误处理。
 * 这样既不需要启动 LRC，也不会污染真实记忆库。
 */
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { LrcClient } from '../src/lrc-client.ts';

const received: Array<{ path: string; body: unknown }> = [];

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve) => {
    let data = '';
    req.on('data', (chunk) => (data += chunk));
    req.on('end', () => resolve(data));
  });
}

function send(res: ServerResponse, status: number, payload: unknown): void {
  const body = JSON.stringify(payload);
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
  res.end(body);
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? '/', 'http://127.0.0.1');
  const raw = await readBody(req);
  received.push({ path: url.pathname, body: raw ? JSON.parse(raw) : null });

  if (url.pathname === '/v1/context/distill') {
    send(res, 200, {
      items: [
        {
          content: '[决策] 用 pnpm 替代 npm',
          memory_type: 'decision',
          tags: ['decision', 'distilled'],
          importance: 7,
          source_turns: [0, 0],
        },
      ],
      count: 1,
      index_line: '前序对话已提炼 1 条至 LRC（项目：demo）',
    });
    return;
  }
  if (url.pathname === '/v1/context/migrate') {
    send(res, 200, { written: 1, merged: 0, failed: 0 });
    return;
  }
  if (url.pathname === '/v1/context/index') {
    send(res, 200, {
      index_line: 'LRC 归档区 1 条（项目：demo）',
      count: 1,
      total: 1,
      items: [
        { id: 'm1', content_preview: '[归档] 前序对话', memory_type: 'code_context', created_at_ms: 1 },
      ],
    });
    return;
  }
  if (url.pathname === '/v1/context/boom') {
    send(res, 500, { error: 'boom' });
    return;
  }
  send(res, 404, { error: 'not_found' });
});

await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
const address = server.address();
if (address === null || typeof address === 'string') throw new Error('无法获取 mock 端口');
const baseUrl = `http://127.0.0.1:${address.port}`;

let failed = 0;
function check(name: string, condition: boolean, detail?: unknown): void {
  if (condition) {
    console.log(`  PASS  ${name}`);
  } else {
    failed += 1;
    console.log(`  FAIL  ${name}${detail === undefined ? '' : ` -> ${JSON.stringify(detail)}`}`);
  }
}

try {
  const client = new LrcClient({ baseUrl, token: 'test-token' });

  console.log('1) distill');
  const distilled = await client.distill('我们决定用 pnpm 替代 npm。', 'demo', 10);
  check('返回 1 条', distilled.count === 1, distilled);
  check('索引行含条数', distilled.index_line.includes('1 条'), distilled.index_line);
  const distillReq = received.at(-1);
  check('请求体含 text/project/max_items', 
    typeof (distillReq?.body as Record<string, unknown>)?.text === 'string' &&
    (distillReq?.body as Record<string, unknown>)?.project === 'demo' &&
    (distillReq?.body as Record<string, unknown>)?.max_items === 10,
    distillReq?.body);

  console.log('2) migrate');
  const migrated = await client.migrate(distilled.items, 'demo');
  check('written=1', migrated.written === 1, migrated);
  const migrateReq = received.at(-1);
  check('请求体含 items/project',
    Array.isArray((migrateReq?.body as Record<string, unknown>)?.items) &&
    (migrateReq?.body as Record<string, unknown>)?.project === 'demo',
    migrateReq?.body);

  console.log('3) index');
  const index = await client.index('demo', 5);
  check('count=1', index.count === 1, index);
  check('查询串带 project 与 limit', received.at(-1)?.path === '/v1/context/index', received.at(-1)?.path);

  console.log('4) migrateText（提炼 + 迁移组合）');
  const combined = await client.migrateText('我们决定用 pnpm 替代 npm。', 'demo');
  check('distilled=1 且 written=1', combined.distilled === 1 && combined.written === 1, combined);

  console.log('5) 错误路径：5xx 必须抛出且带状态码');
  let threw = false;
  try {
    await client.request('/v1/context/boom', { method: 'GET' });
  } catch (error) {
    threw = true;
    check('错误信息含 500', String(error).includes('500'), String(error));
  }
  check('5xx 抛出异常', threw);

  console.log('6) 错误路径：服务不可达必须抛出');
  const deadClient = new LrcClient({ baseUrl: 'http://127.0.0.1:1', timeoutMs: 1000 });
  let deadThrew = false;
  try {
    await deadClient.index('demo');
  } catch {
    deadThrew = true;
  }
  check('不可达时抛出异常', deadThrew);
} finally {
  // 必须等 close 回调再退出：Windows 上 server.close() 后立刻 process.exit()
  // 会触发 libuv 断言（UV_HANDLE_CLOSING），表现为进程崩溃而非正常退出。
  await new Promise<void>((resolve) => server.close(() => resolve()));
}

console.log(failed === 0 ? '\n全部通过' : `\n${failed} 项失败`);
process.exitCode = failed === 0 ? 0 : 1;

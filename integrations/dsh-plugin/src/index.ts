/**
 * DSH 插件入口：把会话上下文迁移进 LRC 记忆库。
 *
 * ## 当前实现范围（已核实）
 *
 * 只注册一个**工具** `lrc_migrate_context`：模型可显式调用它，把一段对话文本
 * 提炼成结构化记忆并写入 LRC。这部分用的是 DSH 官方公开的 Tool 扩展点
 * （`apply(ctx)` + `ctx.tools.register(defineTool(...))`）。
 *
 * ## 尚未实现的部分（需要核对签名后再写）
 *
 * 真正的"自动接管"应实现 DSH 的压缩后端（`ctx.compaction` seam）：
 * 继承 `CompactionEngine`，在 `compactNow` / `compactIfNeeded` 里先把待压缩区间
 * 迁移到 LRC，再产出摘要。**本文件刻意不写这段** —— 该 seam 的精确签名在
 * `@deepseek-ai/dsh-compaction` 的 `src/index.ts` 里，且 DSH 处于 Developer Preview，
 * 凭记忆写必然产生跑不起来的代码。落地步骤见 README「下一步」。
 *
 * ## 端口纪律
 *
 * 默认 3099（本地正式）。开发联调在 `cordis.patch.yml` 里改成 3111。
 */
import type { Context } from '@deepseek-ai/cordis';
import { defineTool } from '@deepseek-ai/dsh-tools';
import Schema from '@deepseek-ai/schemastery';
import { LrcClient } from './lrc-client.js';

export const name = 'dsh-plugin-lrc';

/** 等工具注册表就绪后再执行 apply。 */
export const inject = ['tools'];

export interface Config {
  /** LRC 服务地址 */
  baseUrl: string;
  /** Bearer Token（服务端设了 LRC_API_TOKEN 时必填） */
  token: string;
  /** 单次迁移最多条数 */
  maxItems: number;
}

export const Config: Schema = Schema.object({
  baseUrl: Schema.string().default('http://127.0.0.1:3099'),
  token: Schema.string().default(''),
  maxItems: Schema.number().default(40),
});

export function apply(ctx: Context, config: Config): void {
  const client = new LrcClient({
    baseUrl: config.baseUrl,
    token: config.token || undefined,
  });

  ctx.tools.register(
    defineTool({
      name: 'lrc_migrate_context',
      description:
        '把一段对话文本提炼成结构化记忆并写入 LRC 记忆库。' +
        '用于上下文即将被压缩、或用户要求"把这段结论记下来"时。' +
        '返回一行索引，可直接展示给用户。',
      parameters: {
        text: {
          type: 'string',
          required: true,
          description: '要提炼的对话原文（可含多轮，按换行分隔）',
        },
        project: {
          type: 'string',
          required: false,
          description: '归属项目名，缺省不归属',
        },
      },
      output: {
        schema: {
          type: 'object',
          additionalProperties: false,
          properties: {
            index_line: { type: 'string', required: true },
            distilled: { type: 'number', required: true },
            written: { type: 'number', required: true },
            merged: { type: 'number', required: true },
          },
        },
        render: (_args: unknown, value: { index_line: string }) => [
          { type: 'text', text: value.index_line },
        ],
      },
      async execute(args: { text: string; project?: string }) {
        try {
          const result = await client.migrateText(args.text, args.project, config.maxItems);
          return {
            index_line: result.indexLine,
            distilled: result.distilled,
            written: result.written,
            merged: result.merged,
          };
        } catch (error) {
          // 不抛给模型：LRC 没起是常见情况，应降级为一句可读提示，
          // 而不是让整轮工具调用失败。
          return {
            index_line: `LRC 未连接，本次未迁移（${String(error).slice(0, 120)}）`,
            distilled: 0,
            written: 0,
            merged: 0,
          };
        }
      },
    }),
  );
}

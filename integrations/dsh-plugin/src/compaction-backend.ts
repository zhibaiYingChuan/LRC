/**
 * LRC 压缩后端：在 DSH 压缩会话时，先把待压缩区间迁移进 LRC，再产出摘要。
 *
 * ## 为什么只重写 `summarize()`
 *
 * 官方 `BasicCompactionEngine` 把"选范围 / 写 compaction 标记 / 替换表层 / 计价"
 * 全部固定在 `compactSurfaceRegion` 里，只留一个子类钩子：
 *
 * > `summarize()` is the sole subclass customization hook; the replay and durable
 * > mutation strategy stays fixed so every pricing decision uses the singleton token meter.
 *
 * 所以本后端**不碰**锁、事件序列与表层替换 —— 那些是官方契约，改了就破坏回放确定性。
 * 我们只在"摘要内容"这一步插入 LRC 迁移，并把索引行拼到摘要最前面。
 *
 * ## 失败策略
 *
 * LRC 不可达时**不阻断压缩**：记一条 warn，返回模型原始摘要。
 * 理由：压缩是宿主的核心能力，不能因为一个旁路记忆服务挂掉就让用户卡住。
 *
 * ## 配置
 *
 * 复用父类的 `Config`（阈值、保留比例等），LRC 相关参数走环境变量：
 * `LRC_BASE` / `LRC_TOKEN` / `LRC_PROJECT` / `LRC_MAX_ITEMS`。
 * 这样不必复制父类那十来个 Schemastery 字段，也不会因父类加字段而失配。
 */
import type { Context } from '@deepseek-ai/cordis';
import { BasicCompactionEngine } from '@deepseek-ai/dsh-compaction-basic';
import type { SummarizationInput, SummaryResult } from '@deepseek-ai/dsh-compaction-basic/summarizer';
import type { Agent } from '@deepseek-ai/dsh-agent';
import type { Message } from '@deepseek-ai/dsh-llm';
import { LrcClient } from './lrc-client.js';

/** 单次送进 LRC 的文本上限（字符）。LRC 的请求体上限是 2 MiB，这里留足余量。 */
const MAX_TEXT_CHARS = 200_000;

/**
 * 从重放消息里抽出纯文本。
 *
 * 只取 `type === 'text'` 的块：图片、工具调用参数等一律跳过 ——
 * LRC 的提炼引擎只处理文本，塞进去只会变成噪声。
 */
function extractText(messages: readonly Message[]): string {
  const parts: string[] = [];
  for (const message of messages) {
    const content = (message as { content?: unknown }).content;
    if (typeof content === 'string') {
      parts.push(content);
      continue;
    }
    if (!Array.isArray(content)) continue;
    for (const block of content) {
      const candidate = block as { type?: string; text?: unknown };
      if (candidate?.type === 'text' && typeof candidate.text === 'string') {
        parts.push(candidate.text);
      }
    }
  }
  const joined = parts.join('\n');
  return joined.length > MAX_TEXT_CHARS ? joined.slice(0, MAX_TEXT_CHARS) : joined;
}

export class LrcCompactionEngine extends BasicCompactionEngine {
  /** 与父类一致：摘要需要 llm，计价需要 tokenMeter，落盘需要 sessions。 */
  static inject = ['llm', 'tokenMeter', 'sessions'];

  /** 复用父类配置（阈值 / 保留比例 / 重试次数等），LRC 参数走环境变量。 */
  static Config = BasicCompactionEngine.Config;

  private readonly lrc: LrcClient;
  private readonly lrcProject: string;
  private readonly lrcMaxItems: number;

  constructor(ctx: Context, config: ConstructorParameters<typeof BasicCompactionEngine>[1] = {}) {
    super(ctx, config);
    this.lrc = new LrcClient();
    this.lrcProject = process.env.LRC_PROJECT ?? 'dsh';
    this.lrcMaxItems = Number.parseInt(process.env.LRC_MAX_ITEMS ?? '40', 10) || 40;
  }

  /**
   * 先迁移到 LRC，再返回"LRC 索引行 + 模型摘要"。
   *
   * 顺序刻意是"先迁移后摘要"：即使摘要失败导致本次压缩回滚，
   * 记忆也已经落库（migrate 幂等，重复迁移只会合并）。
   */
  protected override async summarize(
    input: SummarizationInput,
    agent: Agent,
    signal?: AbortSignal,
  ): Promise<SummaryResult> {
    const base = await super.summarize(input, agent, signal);

    const text = extractText(input.messages);
    if (text.trim().length === 0) return base;

    try {
      const result = await this.lrc.migrateText(text, this.lrcProject, this.lrcMaxItems);
      if (result.distilled === 0) return base;

      const note =
        `[LRC 归档] 本次压缩的 ${result.distilled} 条结论已写入 LRC 记忆库` +
        `（项目：${this.lrcProject}；新增 ${result.written} 条，合并 ${result.merged} 条）。` +
        '需要细节时调用 recall 取全文，不要凭摘要猜测。';

      return {
        ...base,
        summary: [{ type: 'text', text: note }, ...base.summary],
      };
    } catch (error: unknown) {
      // 不阻断压缩：LRC 是旁路，宿主的核心能力不能因它挂掉。
      this.ctx.logger.warn(
        `LRC 迁移失败，本次仅做摘要：${error instanceof Error ? error.message : String(error)}`,
      );
      return base;
    }
  }
}

export default LrcCompactionEngine;

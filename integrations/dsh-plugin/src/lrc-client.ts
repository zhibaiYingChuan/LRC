/**
 * LRC HTTP 客户端（纯 TypeScript，**不依赖 DSH**）。
 *
 * 为什么单独成文件：DSH 插件 API 处于 Developer Preview，签名会变；
 * 而 LRC 的三个接口是稳定的。把 HTTP 逻辑隔离出来，插件 API 变了只需改 index.ts。
 *
 * 端口纪律：默认 3099（本地正式）。开发联调请设 LRC_BASE=http://127.0.0.1:3111。
 */

/** 一条提炼结果（与 LRC `DistilledItem` 对齐）。 */
export interface DistilledItem {
  content: string;
  memory_type: string;
  tags: string[];
  importance: number;
  source_turns: [number, number];
}

export interface DistillResult {
  items: DistilledItem[];
  count: number;
  index_line: string;
}

export interface MigrateResult {
  written: number;
  merged: number;
  failed: number;
}

export interface IndexResult {
  index_line: string;
  count: number;
  total: number;
  items: Array<{
    id: string;
    content_preview: string;
    memory_type: string;
    created_at_ms: number;
  }>;
}

export interface LrcClientOptions {
  /** 服务地址，缺省读 LRC_BASE，再缺省 3099 */
  baseUrl?: string;
  /** Bearer Token，缺省读 LRC_TOKEN */
  token?: string;
  /** 单次请求超时（毫秒），默认 20s */
  timeoutMs?: number;
}

export class LrcClient {
  private readonly baseUrl: string;
  private readonly token?: string;
  private readonly timeoutMs: number;

  constructor(options: LrcClientOptions = {}) {
    const raw = options.baseUrl ?? process.env.LRC_BASE ?? 'http://127.0.0.1:3099';
    this.baseUrl = raw.replace(/\/+$/, '');
    this.token = options.token ?? process.env.LRC_TOKEN;
    this.timeoutMs = options.timeoutMs ?? 20_000;
  }

  private headers(): Record<string, string> {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    return headers;
  }

  /**
   * 底层请求封装（公开以便冒烟测试直接验证错误路径）。
   *
   * 非 2xx 一律抛错并带上状态码与响应体前 300 字符 —— 插件里必须能区分
   * "服务没起"（fetch 抛错）与"服务起了但拒绝"（带状态码的 Error）。
   */
  async request<T>(path: string, init: RequestInit): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: this.headers(),
      signal: AbortSignal.timeout(this.timeoutMs),
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`LRC ${path} 返回 ${response.status}: ${text.slice(0, 300)}`);
    }
    return JSON.parse(text) as T;
  }

  /** 提炼对话文本（不写库）。 */
  async distill(text: string, project?: string, maxItems = 40): Promise<DistillResult> {
    return this.request<DistillResult>('/v1/context/distill', {
      method: 'POST',
      body: JSON.stringify({ text, project, max_items: maxItems }),
    });
  }

  /** 批量写入（幂等：同内容走相似合并）。 */
  async migrate(items: DistilledItem[], project?: string): Promise<MigrateResult> {
    return this.request<MigrateResult>('/v1/context/migrate', {
      method: 'POST',
      body: JSON.stringify({ items, project }),
    });
  }

  /** 取归档索引（`tags` 含 `archived` 的记忆）。 */
  async index(project?: string, limit = 20): Promise<IndexResult> {
    const query = new URLSearchParams();
    if (project) query.set('project', project);
    query.set('limit', String(limit));
    return this.request<IndexResult>(`/v1/context/index?${query.toString()}`, {
      method: 'GET',
    });
  }

  /**
   * 一步完成"提炼 + 迁移"，返回可直接展示给模型的索引行。
   *
   * 这是插件最常用的入口：拿到一段对话文本，落库，回一行索引。
   */
  async migrateText(
    text: string,
    project?: string,
    maxItems = 40,
  ): Promise<{ distilled: number; written: number; merged: number; indexLine: string }> {
    const distilled = await this.distill(text, project, maxItems);
    if (distilled.items.length === 0) {
      return { distilled: 0, written: 0, merged: 0, indexLine: distilled.index_line };
    }
    const migrated = await this.migrate(distilled.items, project);
    return {
      distilled: distilled.count,
      written: migrated.written,
      merged: migrated.merged,
      indexLine: distilled.index_line,
    };
  }
}

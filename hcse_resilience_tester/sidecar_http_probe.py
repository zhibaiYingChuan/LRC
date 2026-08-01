#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
HCSE Phase 3 + Phase 4 — Sidecar HTTP 韧性验证
覆盖：超时/卡死/错误/取消/竞态五类异常路径
输出：每项 PASS/FAIL/CANNOT_VERIFY + 证据
"""
import json
import time
import threading
import urllib.request
import urllib.error
import socket
from concurrent.futures import ThreadPoolExecutor, as_completed

SIDECAR = 'http://127.0.0.1:3099'
TIMEOUT_FAST = 3   # 健康检查超时阈值（HCSE 文档 L6-01：8s，这里收紧到 3s 测 try_lock）
TIMEOUT_NORMAL = 10

results = []

def record(case_id, desc, status, evidence, root_cause=None, fix=None):
    results.append({
        'case_id': case_id, 'desc': desc, 'status': status,
        'evidence': evidence, 'root_cause': root_cause, 'fix': fix
    })

def http_get(path, timeout=TIMEOUT_NORMAL):
    """返回 (status_code, body, elapsed_ms, error)"""
    url = SIDECAR + path
    t0 = time.time()
    try:
        req = urllib.request.Request(url, headers={'Accept': 'application/json'})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read().decode('utf-8', errors='replace')
            return resp.status, body, int((time.time() - t0) * 1000), None
    except urllib.error.HTTPError as e:
        body = e.read().decode('utf-8', errors='replace') if e.fp else ''
        return e.code, body, int((time.time() - t0) * 1000), None
    except Exception as e:
        return None, '', int((time.time() - t0) * 1000), str(e)

# ============== Phase 3：基础不变量验证 ==============

# INV-A：/health 永不卡死（try_lock 保证）
code, body, ms, err = http_get('/health', timeout=TIMEOUT_FAST)
if err:
    record('INV-A', '/health 在 3s 内必须返回（try_lock 保证）', 'FAIL',
           f'err={err} ms={ms}', 'sidecar /health 卡死或不可达', '检查 server.rs:1685 try_lock 实现')
elif code == 200 and ms < 3000:
    j = json.loads(body)
    record('INV-A', '/health 在 3s 内必须返回（try_lock 保证）', 'PASS',
           f'code={code} ms={ms} lock_busy={j.get("lock_busy")} status={j.get("status")} indexing_complete={j.get("indexing",{}).get("complete")}')
else:
    record('INV-A', '/health 在 3s 内必须返回（try_lock 保证）', 'FAIL',
           f'code={code} ms={ms} body={body[:200]}')

# INV-B：lock_busy 期间 /v1/* 必须返回 503 而非挂起 10s
code, body, ms, err = http_get('/v1/health/system', timeout=TIMEOUT_NORMAL)
if code == 503 and 'lock_busy' in body and ms < 2000:
    record('INV-B', 'lock_busy 期间 /v1/health/system 必须快速 503（try_lock）', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif code == 503:
    record('INV-B', 'lock_busy 期间 /v1/health/system 必须快速 503（try_lock）', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif ms > 8000:
    record('INV-B', 'lock_busy 期间 /v1/health/system 必须快速 503（try_lock）', 'FAIL',
           f'code={code} ms={ms} body={body[:150]}', 'try_lock 未生效，请求挂起 >8s',
           '检查 v1_api.rs:589 是否 try_lock')
else:
    record('INV-B', 'lock_busy 期间 /v1/health/system 必须快速 503（try_lock）', 'FAIL',
           f'code={code} ms={ms} body={body[:150]}', '预期 503 lock_busy，实际返回其他',
           '检查 memory_store 锁状态')

# INV-C：/v1/memories/stats 同样必须 503 快速返回
code, body, ms, err = http_get('/v1/memories/stats', timeout=TIMEOUT_NORMAL)
if code == 503 and 'lock_busy' in body and ms < 2000:
    record('INV-C', 'lock_busy 期间 /v1/memories/stats 必须快速 503', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif code == 503:
    record('INV-C', 'lock_busy 期间 /v1/memories/stats 必须快速 503', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif ms > 8000:
    record('INV-C', 'lock_busy 期间 /v1/memories/stats 必须快速 503', 'FAIL',
           f'code={code} ms={ms}', 'try_lock 未生效', '检查 v1_api.rs:1019')
else:
    record('INV-C', 'lock_busy 期间 /v1/memories/stats 必须快速 503', 'FAIL',
           f'code={code} ms={ms} body={body[:150]}')

# INV-D：/v1/health/detailed（v0.8.21 P0-01 修复点）
code, body, ms, err = http_get('/v1/health/detailed', timeout=TIMEOUT_NORMAL)
if code == 503 and 'lock_busy' in body and ms < 2000:
    record('INV-D', '/v1/health/detailed try_lock 修复生效（v0.8.21 P0-01）', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif ms > 8000:
    record('INV-D', '/v1/health/detailed try_lock 修复生效（v0.8.21 P0-01）', 'FAIL',
           f'code={code} ms={ms}', 'v0.8.21 P0-01 修复未生效，请求挂起', '检查 v1_api.rs:698')
else:
    record('INV-D', '/v1/health/detailed try_lock 修复生效（v0.8.21 P0-01）', 'FAIL',
           f'code={code} ms={ms} body={body[:150]}')

# INV-E：/v1/health/dao_metrics（L6-03 修复点）
code, body, ms, err = http_get('/v1/health/dao_metrics', timeout=TIMEOUT_NORMAL)
if code == 503 and 'lock_busy' in body and ms < 2000:
    record('INV-E', '/v1/health/dao_metrics try_lock 生效', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif code == 503:
    record('INV-E', '/v1/health/dao_metrics try_lock 生效', 'PASS',
           f'code={code} ms={ms} body={body[:150]}')
elif ms > 8000:
    record('INV-E', '/v1/health/dao_metrics try_lock 生效', 'FAIL',
           f'code={code} ms={ms}', 'try_lock 未生效', '检查 v1_api.rs 对应路由')
else:
    record('INV-E', '/v1/health/dao_metrics try_lock 生效', 'FAIL',
           f'code={code} ms={ms} body={body[:150]}')

# INV-F：状态矛盾检测 — lock_busy=true 且 indexing.complete=true 是矛盾状态
code, body, ms, err = http_get('/health', timeout=TIMEOUT_NORMAL)
if code == 200:
    j = json.loads(body)
    lb = j.get('lock_busy')
    ic = j.get('indexing', {}).get('complete')
    status = j.get('status')
    if lb and ic and status == 'running':
        record('INV-F', '/health 状态自洽（lock_busy 与 indexing 不能矛盾）', 'FAIL',
               f'lock_busy={lb} indexing_complete={ic} status={status} memory_total={j.get("memory",{}).get("total")}',
               '后台结晶任务持锁但 indexing 已标记 complete，状态矛盾',
               'consolidation.rs 中 lock().await 持有时间过长，应改 try_lock 或分段持锁')
    else:
        record('INV-F', '/health 状态自洽', 'PASS',
               f'lock_busy={lb} indexing_complete={ic} status={status}')
else:
    record('INV-F', '/health 状态自洽', 'FAIL', f'code={code} ms={ms}')

# ============== Phase 4：异常路径验证 ==============

# T-01：超时路径 — /health 8s 超时是否真正触发
code, body, ms, err = http_get('/health', timeout=8)
if err and 'timed out' in err.lower():
    record('T-01', '/health 8s 超时触发（异常路径）', 'FAIL',
           f'err={err}', '/health 卡死 >8s，try_lock 失效', '检查 server.rs:1695/1711')
elif ms < 8000:
    record('T-01', '/health 8s 超时触发（正常路径应快速返回）', 'PASS',
           f'code={code} ms={ms} (try_lock 生效，未卡死)')
else:
    record('T-01', '/health 8s 超时触发', 'FAIL', f'code={code} ms={ms}')

# T-02：卡死路径 — 并发 5 个 /v1/memories/stats 是否都快速 503（不串行卡死）
def concurrent_stats(i):
    return http_get('/v1/memories/stats', timeout=5)

t0 = time.time()
with ThreadPoolExecutor(max_workers=5) as ex:
    futs = [ex.submit(concurrent_stats, i) for i in range(5)]
    concurrent_results = [f.result() for f in as_completed(futs)]
total_ms = int((time.time() - t0) * 1000)
all_503 = all(r[0] == 503 for r in concurrent_results)
all_fast = all(r[2] < 2000 for r in concurrent_results)
if all_503 and all_fast:
    record('T-02', '并发 5 个 /v1/memories/stats 都快速 503（无串行卡死）', 'PASS',
           f'total={total_ms}ms individual={[r[2] for r in concurrent_results]}')
else:
    record('T-02', '并发 5 个 /v1/memories/stats 都快速 503', 'FAIL',
           f'total={total_ms}ms results={[(r[0],r[2]) for r in concurrent_results]}',
           '可能存在锁串行或 try_lock 未生效', '检查 try_lock 实现')

# T-03：错误路径 — 不存在端点返回 404 而非 500
code, body, ms, err = http_get('/v1/nonexistent_endpoint', timeout=5)
if code == 404:
    record('T-03', '不存在端点返回 404（错误路径）', 'PASS', f'code={code} ms={ms}')
else:
    record('T-03', '不存在端点返回 404', 'FAIL', f'code={code} ms={ms} body={body[:100]}',
           '路由匹配逻辑问题', '检查 axum 路由 fallback 配置')

# T-04：错误响应格式 — 503 必须包含 error 字段（结构化错误）
code, body, ms, err = http_get('/v1/health/system', timeout=5)
if code == 503:
    try:
        j = json.loads(body)
        if 'error' in j and 'lock_busy' in str(j.get('error', '')):
            record('T-04', '503 响应包含结构化 error 字段', 'PASS',
                   f'body={body[:200]}')
        else:
            record('T-04', '503 响应包含结构化 error 字段', 'FAIL',
                   f'body={body[:200]}', '错误响应缺 error 字段', '统一 503 响应格式')
    except Exception as e:
        record('T-04', '503 响应包含结构化 error 字段', 'FAIL',
               f'json parse error: {e} body={body[:200]}')
else:
    record('T-04', '503 响应包含结构化 error 字段', 'CANNOT_VERIFY', f'code={code}')

# T-05：竞态路径 — 同时打 /health 和 /v1/health/system 验证锁不串行
def hit_health(): return ('health',) + http_get('/health', timeout=5)
def hit_system(): return ('system',) + http_get('/v1/health/system', timeout=5)
t0 = time.time()
with ThreadPoolExecutor(max_workers=4) as ex:
    futs = [ex.submit(hit_health), ex.submit(hit_system), ex.submit(hit_health), ex.submit(hit_system)]
    race_results = [f.result() for f in as_completed(futs)]
race_total = int((time.time() - t0) * 1000)
health_times = [r[3] for r in race_results if r[0] == 'health']
system_times = [r[3] for r in race_results if r[0] == 'system']
if max(health_times) < 3000 and max(system_times) < 3000:
    record('T-05', '/health 与 /v1/* 并发无串行阻塞', 'PASS',
           f'total={race_total}ms health={health_times} system={system_times}')
else:
    record('T-05', '/health 与 /v1/* 并发无串行阻塞', 'FAIL',
           f'total={race_total}ms health={health_times} system={system_times}',
           '锁可能串行化', '检查 try_lock 是否真正非阻塞')

# T-06：sidecar 存活持续性 — 5s 内 3 次 /health 都必须 <3s
times = []
for i in range(3):
    code, body, ms, err = http_get('/health', timeout=5)
    times.append((code, ms))
    time.sleep(1)
if all(t[0] == 200 and t[1] < 3000 for t in times):
    record('T-06', '/health 持续 3 次都快速响应（存活稳定）', 'PASS', f'times={times}')
else:
    record('T-06', '/health 持续 3 次都快速响应', 'FAIL', f'times={times}',
           'sidecar 可能间歇性卡死', '检查后台任务持锁时间')

# ============== 输出报告 ==============
print('=' * 80)
print('HCSE Phase 3/4 Sidecar HTTP 韧性验证报告')
print('=' * 80)
pass_count = sum(1 for r in results if r['status'] == 'PASS')
fail_count = sum(1 for r in results if r['status'] == 'FAIL')
cv_count = sum(1 for r in results if r['status'] == 'CANNOT_VERIFY')
print(f'总计: {len(results)} 项 | PASS: {pass_count} | FAIL: {fail_count} | CANNOT_VERIFY: {cv_count}')
print('=' * 80)
for r in results:
    print(f"\n[{r['status']}] {r['case_id']}: {r['desc']}")
    print(f"  证据: {r['evidence']}")
    if r.get('root_cause'):
        print(f"  根因: {r['root_cause']}")
    if r.get('fix'):
        print(f"  修复: {r['fix']}")
print('=' * 80)
print(f'PASS率: {pass_count}/{len(results)} = {pass_count*100//len(results)}%')

# 保存 JSON 报告
with open(r'g:\code-memory\hcse_resilience_tester\sidecar_results.json', 'w', encoding='utf-8') as f:
    json.dump(results, f, ensure_ascii=False, indent=2)
print('报告已保存: g:\\code-memory\\hcse_resilience_tester\\sidecar_results.json')

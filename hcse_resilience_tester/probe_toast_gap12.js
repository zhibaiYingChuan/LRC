async () => {
  const tc = document.getElementById('toast-container');
  if (!tc) return { error: 'no toast container' };
  if (typeof showToast !== 'function') return { error: 'showToast 未定义' };
  const before = tc.querySelectorAll('.toast:not(.toast-leaving)').length;
  // 触发 5 个 error toast（GAP-12 期望最多保留 2 个 error）
  for (let i = 0; i < 5; i++) {
    showToast('GAP-12测试错误' + (i+1), 'error', 8000);
  }
  await new Promise(r => setTimeout(r, 600));
  const after = tc.querySelectorAll('.toast:not(.toast-leaving)').length;
  const errors = tc.querySelectorAll('.toast-error:not(.toast-leaving)').length;
  // 清理测试 toast
  tc.querySelectorAll('.toast').forEach(t => {
    if ((t.textContent||'').includes('GAP-12测试')) t.remove();
  });
  return {
    before, after, errorToasts: errors,
    gap12Enforced: errors <= 2,
    note: 'GAP-12 要求 error toast 独立计数上限 2'
  };
}()

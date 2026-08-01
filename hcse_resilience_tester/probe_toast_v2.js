(async () => {
  const tc = document.getElementById('toast-container');
  if (!tc) return { error: 'no tc' };
  if (typeof showToast !== 'function') return { error: 'no showToast' };
  const before = tc.querySelectorAll('.toast:not(.toast-leaving)').length;
  const beforeErr = tc.querySelectorAll('.toast-error:not(.toast-leaving)').length;
  const logs = [];
  for (let i = 0; i < 5; i++) {
    showToast('GAP12TauriErr' + (i+1), 'error', 8000);
    const total = tc.querySelectorAll('.toast:not(.toast-leaving)').length;
    const err = tc.querySelectorAll('.toast-error:not(.toast-leaving)').length;
    logs.push({ step: i+1, total: total, err: err });
  }
  await new Promise(r => setTimeout(r, 400));
  const final = tc.querySelectorAll('.toast:not(.toast-leaving)').length;
  const finalErr = tc.querySelectorAll('.toast-error:not(.toast-leaving)').length;
  tc.querySelectorAll('.toast').forEach(t => { if ((t.textContent||'').includes('GAP12TauriErr')) t.remove(); });
  return { before, beforeErr, perStep: logs, final, finalErr, gap12Enforced: finalErr <= 2 };
})()

(async () => {
  if (typeof manualRefreshDashboard !== 'function') return { error: 'no manualRefreshDashboard' };
  const before = (typeof _isManualRefreshing !== 'undefined') ? _isManualRefreshing : null;
  let calls = 0;
  for (let i = 0; i < 5; i++) {
    manualRefreshDashboard();
    calls++;
  }
  await new Promise(r => setTimeout(r, 300));
  const after = (typeof _isManualRefreshing !== 'undefined') ? _isManualRefreshing : null;
  const dashRetry = (typeof _dashboardRetryCount !== 'undefined') ? _dashboardRetryCount : null;
  return {
    before, callsMade: calls, afterRefreshing: after, dashboardRetryCount: dashRetry,
    note: 'GAP-04 防抖期望 _isManualRefreshing 拦截重复调用'
  };
})()

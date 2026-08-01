(() => {
  const banner = document.getElementById('sidecar-down-banner');
  if (!banner) return { exists: false };
  const cs = getComputedStyle(banner);
  return {
    exists: true,
    hiddenAttr: banner.hidden,
    hasHiddenClass: banner.classList.contains('hidden'),
    display: cs.display,
    visibility: cs.visibility,
    offsetParentNull: banner.offsetParent === null,
    offsetHeight: banner.offsetHeight,
    rectHeight: Math.round(banner.getBoundingClientRect().height)
  };
})()

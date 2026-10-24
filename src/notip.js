// No native hover tooltips: the `title` text moves to aria-label (screen readers keep it), the browser bubble never shows.
(() => {
  const strip = (el) => {
    if (el.nodeType !== 1 || !el.hasAttribute('title')) return;
    if (!el.hasAttribute('aria-label') && !el.textContent.trim()) el.setAttribute('aria-label', el.getAttribute('title'));
    el.removeAttribute('title');
  };
  const sweep = (root) => { strip(root); root.querySelectorAll && root.querySelectorAll('[title]').forEach(strip); };
  new MutationObserver((ms) => {
    for (const m of ms) {
      if (m.type === 'attributes') strip(m.target);
      else m.addedNodes.forEach((n) => n.nodeType === 1 && sweep(n));
    }
  }).observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ['title'] });
  sweep(document.documentElement);
})();

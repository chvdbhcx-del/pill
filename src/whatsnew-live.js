// "What's new" card after an update. Rust side: src-tauri/src/whatsnew.rs (whatsnew_check stores the running version and returns
// { version, notes } only once, right after an update; null on a fresh install, an unchanged version or with the switch off).
(() => {
  const P = window.Pill;
  if (!P || !P.invoke || !P.toast) return;
  const { t } = P;
  setTimeout(async () => { // let the island settle first; the check is idempotent, so a page reload cannot show the card twice
    const r = await P.invoke('whatsnew_check');
    if (!r || !r.version) return;
    P.toast({
      id: 'whatsnew', icon: 'sparkles', ttlMs: 14000,
      title: 'Pill ' + r.version,
      sub: (r.notes || []).join(' · ') || t({ ru: 'Приложение обновлено', en: 'Pill was updated' }),
      actions: [
        { label: t({ ru: 'Закрыть', en: 'Close' }), run: () => {} },
        { label: t({ ru: 'Все изменения', en: 'All changes' }), kind: 'white', run: () => P.invoke('settings_open', { page: 'updates' }) },
      ],
    });
  }, 4000);
})();

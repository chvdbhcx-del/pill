// Tiny ru/en helper for the separate settings page: t({ ru, en }) -> string in the current language.
// Language = settings key `language` ('auto' | 'ru' | 'en'); 'auto' follows navigator.language. The resolved value is cached in
// localStorage under the same key pill.js uses ('pill.lang'), so the very first paint is already in the right language.
(() => {
  const nav = () => (/^ru/i.test(navigator.language || '') ? 'ru' : 'en');
  const resolve = (v) => (v === 'ru' || v === 'en' ? v : nav());
  let lang = (() => {
    try { const c = localStorage.getItem('pill.lang'); if (c === 'ru' || c === 'en') return c; } catch (e) { /* storage blocked */ }
    return nav();
  })();
  window.I18n = {
    get lang() { return lang; },
    // Apply the `language` setting value; returns the resolved 'ru' | 'en'.
    set(v) {
      lang = resolve(v);
      try { localStorage.setItem('pill.lang', lang); } catch (e) { /* storage blocked */ }
      document.documentElement.lang = lang;
      return lang;
    },
  };
  window.t = (o) => (o == null ? '' : typeof o === 'string' ? o : o[lang] ?? o.en ?? o.ru ?? '');
  document.documentElement.lang = lang;
})();

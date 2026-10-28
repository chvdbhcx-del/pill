// Settings search: the pure matching used by the field above the sidebar (settings.js builds the entries, see its "search" block; tested in
// settings-search.test.mjs). An entry is { page, pageName, title, desc } in the language on screen. Every word of the query must occur in the title,
// the page name or the description; title matches rank first, then page name, then description. Case and ё / е are ignored.
(function (g) {
  const norm = (s) => String(s == null ? '' : s).toLowerCase().replace(/ё/g, 'е');
  function match(entries, query, max) {
    const toks = norm(query).split(/\s+/).filter(Boolean);
    if (!toks.length || !Array.isArray(entries)) return [];
    const out = [];
    entries.forEach((e, i) => {
      const title = norm(e.title), desc = norm(e.desc), page = norm(e.pageName);
      let score = 0, ok = true;
      for (const t of toks) {
        if (title.startsWith(t) || title.includes(' ' + t)) score += 0;
        else if (title.includes(t)) score += 1;
        else if (page.includes(t)) score += 3;
        else if (desc.includes(t)) score += 4;
        else { ok = false; break; }
      }
      if (ok) out.push({ e, score, i });
    });
    out.sort((a, b) => a.score - b.score || a.i - b.i);
    return out.slice(0, max > 0 ? max : 8).map((x) => x.e);
  }
  g.PillSettingsSearch = { match, norm };
})(typeof window !== 'undefined' ? window : globalThis);

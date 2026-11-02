// Mundo da página (Fase 5, `fase5-navegador`): só reemite `console.error` e
// `console.warn` como `CustomEvent("__tyba_console")`, com `detail` em
// string ("error:<texto>" / "warn:<texto>"), para o mundo isolado do TYBA
// guardar no buffer do `orbit_browser_console`. Não fala com o app: não há
// canal. A página pode forjar o evento — equivale a chamar `console.error`.
(() => {
  const c = window.console;
  if (!c) return;
  const fmt = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return v.stack || `${v.name}: ${v.message}`;
    try {
      return JSON.stringify(v);
    } catch {
      return String(v);
    }
  };
  for (const level of ["error", "warn"]) {
    const orig = c[level];
    if (typeof orig !== "function") continue;
    c[level] = function (...args) {
      try {
        const text = args.map(fmt).join(" ").slice(0, 2000);
        window.dispatchEvent(new CustomEvent("__tyba_console", { detail: `${level}:${text}` }));
      } catch {}
      return orig.apply(this, args);
    };
  }
})();

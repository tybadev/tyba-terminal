/**
 * As abas moram na barra (opção G, 2026-11-02), e a barra não rola: até
 * `limit` abas aparecem todas; passou disso, ficam `limit - 1` e as outras
 * vão para a lista do `+N`. A ativa nunca fica escondida — se ela cair na
 * lista, toma o último lugar visível.
 */
export function splitTabs<T>(
  tabs: T[],
  isActive: (tab: T) => boolean,
  limit = 6,
): { shown: T[]; hidden: T[] } {
  if (tabs.length <= limit) return { shown: tabs, hidden: [] };
  const keep = limit - 1;
  const shown = tabs.slice(0, keep);
  const activeAt = tabs.findIndex(isActive);
  if (activeAt >= keep) shown[keep - 1] = tabs[activeAt];
  const hidden = tabs.filter((tab) => !shown.includes(tab));
  return { shown, hidden };
}

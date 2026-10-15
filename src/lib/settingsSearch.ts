/**
 * Busca das Configurações: acha a opção pelo nome e pela descrição, em
 * qualquer seção. Sem acento e sem caixa — "notificacao" acha "Notificações".
 */

export interface SettingEntry<S extends string = string> {
  section: S;
  label: string;
  hint?: string;
}

const fold = (text: string) =>
  text
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase();

/**
 * Todas as palavras da busca têm de aparecer (no nome ou na descrição). Quem
 * casa pelo nome vem antes de quem casa só pela descrição.
 */
export function searchSettings<S extends string>(
  entries: SettingEntry<S>[],
  query: string,
): SettingEntry<S>[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const byLabel: SettingEntry<S>[] = [];
  const byHint: SettingEntry<S>[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    const key = `${entry.section}:${entry.label}`;
    if (seen.has(key)) continue;
    const label = fold(entry.label);
    const all = `${label} ${fold(entry.hint ?? "")}`;
    if (!words.every((w) => all.includes(w))) continue;
    seen.add(key);
    (words.every((w) => label.includes(w)) ? byLabel : byHint).push(entry);
  }
  return [...byLabel, ...byHint];
}

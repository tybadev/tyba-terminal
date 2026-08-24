/**
 * Formatação do painel de estatísticas de agente.
 *
 * Só apresentação: a conta inteira vem pronta do core (princípio #1). O que
 * mora aqui é a escolha de unidade e o separador decimal — que muda com o
 * idioma e é a única parte que o Rust não tem como decidir.
 */

/** Períodos do filtro. `null` é "tudo" e vai assim para o core. */
export const STATS_PERIODS = [7, 30, null] as const;

export type StatsPeriod = (typeof STATS_PERIODS)[number];

/** Valor do `<Select>` para "todos os repositórios". */
export const ALL_REPOS = "__all__";

function decimal(value: number, locale: string): string {
  return value.toLocaleString(locale, { maximumFractionDigits: 1 });
}

/**
 * Duração legível, com a unidade escolhida pela grandeza.
 *
 * O intervalo útil aqui vai de "aprovou na hora" a "esqueceu a tarde inteira":
 * uma unidade só faria `0,4 s` virar `0 min` ou `2 h` virar `7200000 ms`.
 *
 * `null` é ausência de dado — período sem nenhuma decisão humana — e vira o
 * travessão. Zero é um fato diferente (decidiu instantaneamente) e continua
 * sendo `0 ms`.
 */
export function formatDuration(ms: number | null, locale: string): string {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${decimal(ms / 1000, locale)} s`;
  if (ms < 3_600_000) return `${decimal(ms / 60_000, locale)} min`;
  return `${decimal(ms / 3_600_000, locale)} h`;
}

/**
 * Percentual já arredondado pelo core; aqui só entra o separador do idioma.
 *
 * Guarda contra não-número porque um `NaN` que escapasse do core viraria
 * "NaN%" na tela — o cartão precisa mostrar zero, não um defeito.
 */
export function formatPercent(pct: number, locale: string): string {
  if (!Number.isFinite(pct)) return `${decimal(0, locale)}%`;
  return `${decimal(pct, locale)}%`;
}

/** Contagem com separador de milhar do idioma. */
export function formatCount(value: number, locale: string): string {
  if (!Number.isFinite(value)) return decimal(0, locale);
  return Math.round(value).toLocaleString(locale);
}

/**
 * Regras puras da lista de blocos sem cartão: quando a linha do diretório
 * aparece e como uma duração é escrita.
 */

/**
 * O diretório a anunciar ANTES de um bloco, ou `null` para não anunciar.
 *
 * Só quando muda: repetir o mesmo caminho em todo bloco é ruído que empurra a
 * saída para baixo sem dizer nada novo. `prev` é o cwd do bloco anterior — ou,
 * no primeiro bloco, o da abertura da sessão, que a marca de início já mostra.
 * Diretório desconhecido não se anuncia: não há o que dizer.
 */
export function cwdChange(
  prev: string | null | undefined,
  cwd: string | null | undefined,
): string | null {
  if (!cwd) return null;
  if (prev && sameDir(prev, cwd)) return null;
  return cwd;
}

function sameDir(a: string, b: string): boolean {
  const trim = (p: string) => (p.length > 1 ? p.replace(/\/+$/, "") : p);
  return trim(a) === trim(b);
}

/**
 * Duração escrita para caber no canto do bloco.
 *
 * - abaixo de 10s, um decimal (`0,4s`): é onde a diferença ainda informa;
 * - até 1 min, segundos inteiros (`38s`);
 * - até 1 h, `1m02s`;
 * - acima, `1h05m`.
 *
 * `whole` força segundos inteiros também abaixo de 10s — é o contador ao vivo,
 * que andaria em décimos e faria o canto tremer.
 *
 * O separador decimal segue o idioma (`0,4s` em pt-BR, `0.4s` em inglês).
 * Valor inválido ou negativo volta `null`: não se mostra duração inventada.
 */
export function formatDuration(
  ms: number,
  locale?: string,
  { whole = false }: { whole?: boolean } = {},
): string | null {
  if (!Number.isFinite(ms) || ms < 0) return null;
  const seconds = ms / 1000;
  if (seconds < 10 && !whole) {
    const tenths = Math.max(0.1, Math.round(seconds * 10) / 10);
    const text = new Intl.NumberFormat(locale, {
      minimumFractionDigits: 1,
      maximumFractionDigits: 1,
    }).format(tenths);
    return `${text}s`;
  }
  const total = Math.floor(seconds);
  if (total < 60) return `${total}s`;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (total < 3600) {
    return `${Math.floor(total / 60)}m${pad(total % 60)}s`;
  }
  return `${Math.floor(total / 3600)}h${pad(Math.floor((total % 3600) / 60))}m`;
}

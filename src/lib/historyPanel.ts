/** Como a linha do painel Histórico mostra o fim de uma execução. */
export type RunOutcome =
  | { kind: "ok" }
  | { kind: "failed"; code: number }
  | { kind: "unknown" };

/** Sem exit code (bash/fish importados) não é falha nem sucesso: é desconhecido. */
export function runOutcome(exitCode: number | null | undefined): RunOutcome {
  if (exitCode == null) return { kind: "unknown" };
  return exitCode === 0 ? { kind: "ok" } : { kind: "failed", code: exitCode };
}

/** Hora da execução: só "HH:MM" se foi hoje; senão "DD/MM HH:MM". */
export function runTime(startedAtMs: number, nowMs: number, locale?: string): string {
  const at = new Date(startedAtMs);
  const now = new Date(nowMs);
  const time = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(at);
  const sameDay =
    at.getFullYear() === now.getFullYear() &&
    at.getMonth() === now.getMonth() &&
    at.getDate() === now.getDate();
  if (sameDay) return time;
  const day = new Intl.DateTimeFormat(locale, { day: "2-digit", month: "2-digit" }).format(at);
  return `${day} ${time}`;
}

/** O cwd curto: relativo à raiz do repo quando dentro dela, senão os dois últimos níveis. */
export function shortCwd(cwd: string | null | undefined, root?: string | null): string {
  if (!cwd) return "";
  if (root) {
    const base = root.replace(/\/+$/, "");
    if (cwd === base) return base.split("/").pop() ?? base;
    if (cwd.startsWith(`${base}/`)) {
      return `${base.split("/").pop()}/${cwd.slice(base.length + 1)}`;
    }
  }
  const parts = cwd.split("/").filter(Boolean);
  return parts.length <= 2 ? cwd : `…/${parts.slice(-2).join("/")}`;
}

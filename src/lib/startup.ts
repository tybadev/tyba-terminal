import type { StartupMode } from "../components/SettingsView";

/** Espelha `StartupMode::parse` no core: valor desconhecido ou ausente cai em
 * `resume`, que é o que um terminal faz — reabre onde você parou. */
export function parseStartupMode(raw: string | null | undefined): StartupMode {
  if (raw === "keep_layout") return "keep_layout";
  if (raw === "fresh") return "fresh";
  return "resume";
}

/**
 * O app terminou de carregar e o splash pode sair.
 *
 * Evento de DOM, e não de IPC: quem escuta é o `main.tsx`, que roda antes do
 * React e não fala com o core.
 */
export const SPLASH_DONE_EVENT = "tyba:ready";

/**
 * Teto de espera do splash, em ms.
 *
 * O splash sai quando o app está pronto — mas "pronto" depende do core, e o
 * core pode ficar parado num diálogo de permissão do macOS, que segura a
 * thread até alguém clicar. Sem teto, um boot que trava deixa o usuário preso
 * olhando um logo, que é pior do que a UI vazia que ele substituiu.
 */
export const SPLASH_CEILING_MS = 4000;

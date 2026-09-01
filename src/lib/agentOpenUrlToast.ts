import { openExternalUrl } from "./clipboard";
import type { AgentOpenUrlPayload } from "./ipc";
import type { ToastInput } from "./toast";

export type Translate = (key: string, options?: Record<string, unknown>) => string;

/**
 * Entrega B (§1.2/§5.3) -- monta o toast do 1-clique. Extraído de App.tsx pra
 * ser testável: `known_login` NUNCA esconde host/URL, só muda o título (fix
 * de phishing do review r1 -- um agente comprometido pode mandar
 * `https://claude.ai/oauth/authorize?client_id=<atacante>&redirect_uri=
 * https://evil.com/cb`; host=claude.ai, known_login=true, mas é OAuth de
 * terceiro). `openUrl` é injetável só pra teste -- em produção é sempre
 * `openExternalUrl`, que já revalida no cliente (defesa em profundidade).
 */
export function agentOpenUrlToastInput(
  payload: AgentOpenUrlPayload,
  t: Translate,
  openUrl: (url: string) => void = (url) => {
    void openExternalUrl(url);
  },
): ToastInput {
  return {
    tone: "info",
    title: payload.known_login
      ? t("agentOpenUrlKnownLoginTitle", { host: payload.host })
      : t("agentOpenUrlUnknownTitle", { host: payload.host }),
    detail: payload.url,
    action: {
      label: t("agentOpenUrlAction"),
      run: () => openUrl(payload.url),
    },
  };
}

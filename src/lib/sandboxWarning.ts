import type { AgentSandboxWarningPayload, SandboxWarningKind } from "./ipc";
import type { ToastInput } from "./toast";

/**
 * Entrega B (§6/§12 item 35): mapeia `SandboxWarningKind` (o core manda só o
 * kind + um `detail` cru -- ver `ipc.ts`) pra chave de i18n. Mesmo desenho de
 * `bootFailureTitleKey`: a chave mora aqui, o texto pt-BR/en mora em
 * `i18n/index.ts`, e é lá que fica o `{{detail}}` de interpolação.
 */
export function sandboxWarningTitleKey(kind: SandboxWarningKind): string {
  switch (kind) {
    case "CredencialPaiNaoEhRw":
      return "sandboxWarningCredencialPaiNaoEhRw";
    case "CredencialSombreadaDepois":
      return "sandboxWarningCredencialSombreadaDepois";
    case "CredencialHostNaoGrava":
      return "sandboxWarningCredencialHostNaoGrava";
    case "HomeRoClaudeJsonNaoPersiste":
      return "sandboxWarningHomeRoClaudeJsonNaoPersiste";
    case "FilhoDesconhecidoEmClaude":
      return "sandboxWarningFilhoDesconhecidoEmClaude";
  }
}

/**
 * Review de segurança r2 (v0.6.2), MAJOR: monta o `ToastInput` de um
 * `agent://sandbox-warning` -- extraído do handler em `App.tsx` pra ficar
 * testável sem i18n real nem IPC real (o título já vem resolvido, o ack é
 * injetado). Dois pontos de segurança, ambos travados aqui:
 *
 * - `sticky: true` SEMPRE, pra TODO `SandboxWarningKind` -- nenhum tem
 *   `action`, e sem isso o toast auto-fecharia em ~9s (`toastDuration`)
 *   silenciando um alarme de segurança que o dono nunca viu, num produto de
 *   agente sem supervisão.
 * - `onDismiss` só existe (e só chama `ackDrift`) pro
 *   `FilhoDesconhecidoEmClaude` com `names` não vazio -- é o ack durável do
 *   alarme de deriva (review r1, MAJOR), que só pode disparar quando o
 *   dono de fato dispensa ESTE toast.
 */
export function sandboxWarningToastInput(
  payload: AgentSandboxWarningPayload,
  title: string,
  ackDrift: (names: string[]) => void,
): ToastInput {
  const names =
    payload.kind === "FilhoDesconhecidoEmClaude" ? payload.names : null;
  return {
    tone: "warning",
    title,
    sticky: true,
    onDismiss:
      names && names.length > 0
        ? () => {
            ackDrift(names);
          }
        : undefined,
  };
}

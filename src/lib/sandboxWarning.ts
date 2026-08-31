import type { SandboxWarningKind } from "./ipc";

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

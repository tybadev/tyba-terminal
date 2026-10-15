import type {
  AgentRunner,
  DetectedAgent,
  NoJailReason,
  SessionKind,
  ShimRoute,
} from "./ipc";

export const noticeKey = (detected: DetectedAgent): string =>
  `${detected.pid}:${detected.start_ms}`;

export const agentBinaryName = (kind: AgentRunner): string => {
  if (kind === "claude_code") return "claude";
  if (kind === "codex") return "codex";
  return typeof kind === "object" ? kind.custom : String(kind);
};

/**
 * A faixa "sem gate" (v1, F2) — só pro agente cru, nunca hospedado. Shim v2
 * (tech-spec §7): um agente que o próprio TYBA já gateou pelo canal
 * shim↔core (`hosting`) já está dentro dos princípios 1/4/6/10, jaulado ou
 * não — mostrar "sem gate" por cima seria falso. O par jaulado/não-jaulado
 * é `showUnjailedNotice`, nunca este.
 */
export const showShellAgentNotice = (
  kind: SessionKind,
  detected: DetectedAgent | null | undefined,
  hosting: boolean,
  dismissedKey: string | undefined,
  shim: ShimRoute | null = null,
): boolean =>
  kind.type === "shell" &&
  detected != null &&
  !hosting &&
  // M6-A: um claude que passou pelo `tyba _jail` nunca ganha "Reabrir com
  // gate" — ou ele não abre sessão (passthrough), ou o vermelho
  // `ungatedReason` já diz por que está sem gate.
  shim == null &&
  dismissedKey !== noticeKey(detected);

/**
 * M6-A: o claude passou pelo TYBA mas roda SEM gate — sessão em segundo plano
 * (`ungated`) ou o core não conseguiu hospedar (`fallback`). Vermelho, sem
 * botão de dispensar: é o estado que o usuário precisa ver. `null` quando não
 * se aplica.
 */
export const ungatedReason = (
  kind: SessionKind,
  detected: DetectedAgent | null | undefined,
  hosting: boolean,
  shim: ShimRoute | null,
): "ungated" | "fallback" | "raw" | null =>
  kind.type === "shell" &&
  detected != null &&
  !hosting &&
  (shim === "ungated" || shim === "fallback" || shim === "raw")
    ? shim
    : null;

/** A frase da faixa vermelha, por motivo. */
export const ungatedMessageKey = (reason: "ungated" | "fallback" | "raw"): string =>
  reason === "fallback"
    ? "shellAgentUngatedFallback"
    : reason === "raw"
      ? "shellAgentUngatedRaw"
      : "shellAgentUngatedBackground";

/**
 * O sinal âmbar "sem jaula" (shim v2, tech-spec §7/§9, spec.md decisão 5):
 * hospedado (o gate ligou) mas fora da allowlist de jaula ou sem userns
 * disponível. Nunca junto com `showShellAgentNotice` — os dois exigem
 * `hosting` em polaridades opostas — e nunca sem `hosting`, porque "sem
 * jaula" pressupõe que o gate já está de pé; sem ele o sinal certo é o "sem
 * gate" de sempre.
 *
 * Agente livre (fatia 3): o claude digitado roda sem jaula por desenho, então
 * "sem sandbox" virou o estado normal e o sinal não aparece mais. Desligado
 * aqui; a fatia de limpeza tira o sinal, `noJailMessageKey` e as chaves
 * `shellUnjailed*`.
 */
export const showUnjailedNotice = (
  _kind: SessionKind,
  _detected: DetectedAgent | null | undefined,
  _hosting: boolean,
  _jailed: boolean,
): boolean => false;

/**
 * M7: a frase que explica por que o claude está sem jaula, uma por motivo.
 * Sem motivo conhecido (core antigo), a frase genérica.
 */
export const noJailMessageKey = (reason: NoJailReason | null): string => {
  switch (reason) {
    case "home":
      return "shellUnjailedHome";
    case "hidden_dir":
      return "shellUnjailedHiddenDir";
    case "outside_home":
      return "shellUnjailedOutsideHome";
    case "sandbox":
      return "shellUnjailedSandbox";
    default:
      return "shellUnjailedNotice";
  }
};

import type { AgentRunner, DetectedAgent, SessionKind } from "./ipc";

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
): boolean =>
  kind.type === "shell" &&
  detected != null &&
  !hosting &&
  dismissedKey !== noticeKey(detected);

/**
 * O sinal âmbar "sem jaula" (shim v2, tech-spec §7/§9, spec.md decisão 5):
 * hospedado (o gate ligou) mas fora da allowlist de jaula ou sem userns
 * disponível. Nunca junto com `showShellAgentNotice` — os dois exigem
 * `hosting` em polaridades opostas — e nunca sem `hosting`, porque "sem
 * jaula" pressupõe que o gate já está de pé; sem ele o sinal certo é o "sem
 * gate" de sempre.
 */
export const showUnjailedNotice = (
  kind: SessionKind,
  detected: DetectedAgent | null | undefined,
  hosting: boolean,
  jailed: boolean,
): boolean => kind.type === "shell" && detected != null && hosting && !jailed;

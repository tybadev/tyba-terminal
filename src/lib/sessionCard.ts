// Cartão de sessão da barra lateral (mockup Proposta/PropostaClaro): o ponto
// de estado à direita do nome. Os dados do cartão (caminho, agente, branch,
// contadores do git, abas) seguem como sempre foram.
// A decisão mora aqui, testada; o App só junta o estado e pinta.

/** O que o ponto à direita do nome afirma. */
export type SessionDot = "attention" | "running" | "idle";

export interface SessionDotInput {
  /** Pedidos de aprovação pendentes das sessões deste workspace. */
  approvals: number;
  /**
   * `rank` do `StatusVisual` do agente mais urgente do workspace (falhou 4,
   * esperando 3, concluído-não-visto 2, trabalhando 1), ou `null` sem agente.
   */
  agentRank: number | null;
  /** Algum painel do workspace tem um comando rodando agora. */
  commandRunning: boolean;
}

/**
 * Âmbar quando a sessão pede alguém (aprovação, agente esperando, falhou ou
 * terminou sem ser visto); verde pulsando quando algo roda; cinza parada.
 * Pedir alguém vence rodar: um agente esperando aprovação continua "vivo",
 * mas o que importa ao dono é que ele está parado por sua causa.
 */
export const sessionDot = ({
  approvals,
  agentRank,
  commandRunning,
}: SessionDotInput): SessionDot => {
  if (approvals > 0 || (agentRank ?? 0) >= 2) return "attention";
  if (agentRank === 1 || commandRunning) return "running";
  return "idle";
};

/** Classes do ponto, nos tokens do tema — valem nos dois temas. */
export const SESSION_DOT_CLASS: Record<SessionDot, string> = {
  attention: "bg-tyba-amber [box-shadow:var(--tyba-glow-amber)]",
  running:
    "bg-tyba-green [box-shadow:var(--tyba-glow-green)] tyba-pulse",
  idle: "bg-tyba-text-ghost", // mockup: #3a3a3f / #d4d4da
};

/** Chave i18n do rótulo do ponto (tooltip e leitor de tela). */
export const SESSION_DOT_LABEL_KEY: Record<SessionDot, string> = {
  attention: "sessionDotAttention",
  running: "sessionRunning",
  idle: "sessionIdle",
};

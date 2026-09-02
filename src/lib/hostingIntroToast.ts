import type { ToastInput } from "./toast";
import type { Translate } from "./agentOpenUrlToast";

/**
 * Shim v2, Track C (tech-spec §9): o toast único que explica a mudança de
 * comportamento na PRIMEIRA transição `hosting=false -> true` que o app vê
 * — nunca preso a abertura de sessão (não colide com a ADR
 * 2026-08-22 "o primeiro segundo é do usuário": não escreve no PTY, é
 * puramente informativo). "Uma vez por usuário" é durabilidade local
 * (`localStorage`, ver `hasSeenHostingIntroToast`/`markHostingIntroToastSeen`
 * em App.tsx) — o mesmo padrão de "já visto" que o app já usa para avisos
 * que não precisam da autoridade do core (não é uma decisão de segurança:
 * pior caso de perder o flag é mostrar o toast de novo, não um gate a menos).
 */
export function shouldShowHostingIntroToast(
  alreadySeen: boolean,
  anyHostingNow: boolean,
): boolean {
  return !alreadySeen && anyHostingNow;
}

export function hostingIntroToastInput(t: Translate): ToastInput {
  return {
    tone: "info",
    title: t("shimV2IntroToast"),
  };
}

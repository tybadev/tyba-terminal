export type ToastTone = "info" | "warning" | "error";

/** Entrega B (§5.3): a única ação de um toast é o clique que abre o
 * navegador -- run() nunca é chamado sozinho, só pela mão do dono. */
export interface ToastAction {
  label: string;
  run: () => void;
}

export interface ToastMessage {
  id: string;
  tone: ToastTone;
  title: string;
  detail?: string;
  action?: ToastAction;
  /** Review r1 (v0.6.2), MAJOR: roda quando ESTE toast é dispensado -- X,
   * clique na ação, ou auto-dismiss. Nunca roda se o toast nunca chegou a
   * ser criado (ex.: evento de core perdido) -- é exatamente a distinção
   * que o ack do alarme de deriva precisa: só marcar durável o que o dono
   * de fato viu. */
  onDismiss?: () => void;
  /** Review de segurança r2 (v0.6.2), MAJOR: toast de alarme de segurança
   * (`agent://sandbox-warning`) NÃO tem `action`, mas não pode sumir
   * sozinho -- um auto-dismiss de 9s dispararia `onDismiss` (e o ack
   * durável do alarme de deriva) sem o dono ter visto nada, num produto de
   * agente sem supervisão. `sticky` força duração infinita igual a
   * `action`, mas sem exigir um botão -- some só no X. */
  sticky?: boolean;
}

export interface ToastInput {
  tone?: ToastTone;
  title: string;
  detail?: string;
  action?: ToastAction;
  onDismiss?: () => void;
  sticky?: boolean;
}

type Listener = (toasts: ToastMessage[]) => void;

let toasts: ToastMessage[] = [];
const listeners = new Set<Listener>();

function emit() {
  const snapshot = toasts;
  for (const listener of listeners) listener(snapshot);
}

export function subscribeToasts(listener: Listener): () => void {
  listeners.add(listener);
  listener(toasts);
  return () => {
    listeners.delete(listener);
  };
}

export function pushToast(input: ToastInput): string {
  const id = crypto.randomUUID();
  toasts = [...toasts, { id, tone: input.tone ?? "info", ...input }];
  emit();
  return id;
}

export function dismissToast(id: string) {
  const dismissed = toasts.find((t) => t.id === id);
  const next = toasts.filter((t) => t.id !== id);
  if (next.length === toasts.length) return;
  toasts = next;
  emit();
  dismissed?.onDismiss?.();
}

export function clearToasts() {
  if (toasts.length === 0) return;
  toasts = [];
  emit();
}

/** Item 0 do contrato ("polir o alarme de deriva"): todo toast sem ação
 * precisa sumir sozinho depois de um tempo sensato, além do X — senão fica
 * preso na tela pra sempre (o bug que o dono reportou). ~8-10s dá tempo de
 * ler sem virar ruído acumulado. */
export const TOAST_AUTO_DISMISS_MS = 9000;

/** Toast com ação (ex.: login) OU `sticky` (ex.: alarme de segurança) mantém
 * duração infinita: só some no clique da ação ou no X — nunca pode sumir
 * sozinho antes de o dono decidir. Info benigna sem nenhum dos dois continua
 * com auto-dismiss finito. */
export function toastDuration(
  toast: Pick<ToastMessage, "action" | "sticky">,
): number {
  return toast.action || toast.sticky ? Infinity : TOAST_AUTO_DISMISS_MS;
}

export function toastError(title: string, detail?: unknown) {
  return pushToast({
    tone: "error",
    title,
    detail: detail === undefined ? undefined : String(detail),
  });
}

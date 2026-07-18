export type ToastTone = "info" | "warning" | "error";

export interface ToastMessage {
  id: string;
  tone: ToastTone;
  title: string;
  detail?: string;
}

export interface ToastInput {
  tone?: ToastTone;
  title: string;
  detail?: string;
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
  const next = toasts.filter((t) => t.id !== id);
  if (next.length === toasts.length) return;
  toasts = next;
  emit();
}

export function clearToasts() {
  if (toasts.length === 0) return;
  toasts = [];
  emit();
}

export function toastError(title: string, detail?: unknown) {
  return pushToast({
    tone: "error",
    title,
    detail: detail === undefined ? undefined : String(detail),
  });
}

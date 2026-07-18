export interface ConfirmInput {
  title: string;
  detail?: string;
  confirmLabel?: string;
  destructive?: boolean;
}

export interface ConfirmRequest extends ConfirmInput {
  id: string;
}

interface Pending extends ConfirmRequest {
  resolve: (answer: boolean) => void;
}

type Listener = (current: ConfirmRequest | null) => void;

let queue: Pending[] = [];
const listeners = new Set<Listener>();

function current(): ConfirmRequest | null {
  const head = queue[0];
  if (!head) return null;
  const { resolve: _resolve, ...request } = head;
  return request;
}

function emit() {
  const snapshot = current();
  for (const listener of listeners) listener(snapshot);
}

export function subscribeConfirm(listener: Listener): () => void {
  listeners.add(listener);
  listener(current());
  return () => {
    listeners.delete(listener);
  };
}

export function requestConfirm(input: ConfirmInput): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    queue = [...queue, { id: crypto.randomUUID(), ...input, resolve }];
    emit();
  });
}

export function answerConfirm(id: string, answer: boolean) {
  const head = queue[0];
  if (!head || head.id !== id) return;
  queue = queue.slice(1);
  head.resolve(answer);
  emit();
}

export function resetConfirms() {
  const pending = queue;
  queue = [];
  for (const item of pending) item.resolve(false);
  emit();
}

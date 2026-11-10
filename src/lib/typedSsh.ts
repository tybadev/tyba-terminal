import type { HostFormValues } from "./hostForm";
import { emptyHostForm } from "./hostForm";
import type { SessionCommand, TypedSshDetected, TypedSshTarget } from "./ipc";

/** `user@host:porta` — espelho de `TypedTarget::label` no core. */
export function targetLabel(t: TypedSshTarget): string {
  const host = t.host.includes(":") ? `[${t.host}]` : t.host;
  return `${t.user ? `${t.user}@` : ""}${host}:${t.port}`;
}

/** "09/11": o "desde" dos hosts negados. */
export function dayMonth(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getDate())}/${pad(d.getMonth() + 1)}`;
}

/**
 * O aviso só vale no bloco que o gerou: o comando ainda roda e é o mesmo
 * (`started_at_ms`). Saiu do `ssh` ou rodou outro comando, o aviso some.
 */
export function liveSshNote(
  note: TypedSshDetected | undefined,
  command: SessionCommand | undefined,
): TypedSshDetected | null {
  if (!note || !command?.running) return null;
  if (note.block_started_at_ms !== null && note.block_started_at_ms !== command.started_at_ms) {
    return null;
  }
  return note;
}

/** O cadastro pré-preenchido com o que o `ssh` digitado disse. */
export function hostFormFromDetected(e: TypedSshDetected): HostFormValues {
  const identity = e.destination.identity_file ?? "";
  return {
    ...emptyHostForm(),
    alias: e.destination.host,
    hostname: e.target.host,
    port: e.target.port === 22 ? "" : String(e.target.port),
    username: e.target.user ?? "",
    auth_method: identity ? "file" : "auto",
    identity_file: identity,
    proxy_jump: e.destination.proxy_jump ?? "",
  };
}

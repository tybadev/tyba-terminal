import type { TypedSshTarget } from "./ipc";

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

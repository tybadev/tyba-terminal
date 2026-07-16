import type { Host } from "./ipc";

interface SshTarget {
  user: string | null;
  target: string;
  port: number | null;
}

const FLAGS_WITH_VALUE = new Set([
  "-p",
  "-i",
  "-l",
  "-o",
  "-J",
  "-F",
  "-b",
  "-c",
  "-D",
  "-E",
  "-e",
  "-I",
  "-L",
  "-m",
  "-O",
  "-Q",
  "-R",
  "-S",
  "-W",
  "-w",
]);

/**
 * Extrai o destino de uma linha `ssh ...`. Retorna null se não for um ssh
 * interativo (scp/rsync/`ssh host cmd` não abrem sessão de trabalho no host).
 */
export function parseSshCommand(command: string): SshTarget | null {
  const parts = command.trim().split(/\s+/);
  if (parts[0] !== "ssh" || parts.length < 2) return null;

  let user: string | null = null;
  let port: number | null = null;
  let target: string | null = null;

  for (let i = 1; i < parts.length; i++) {
    const part = parts[i];
    if (FLAGS_WITH_VALUE.has(part)) {
      const value = parts[i + 1];
      if (value === undefined) return null;
      if (part === "-p") {
        const parsed = Number(value);
        port = Number.isInteger(parsed) ? parsed : null;
      }
      if (part === "-l") user = value;
      i++;
      continue;
    }
    if (part.startsWith("-")) continue;
    if (target !== null) return null;
    target = part;
  }

  if (target === null) return null;
  const at = target.lastIndexOf("@");
  if (at !== -1) {
    user = target.slice(0, at) || user;
    target = target.slice(at + 1);
  }
  if (!target) return null;
  return { user, target, port };
}

/**
 * Casa um `ssh` digitado à mão com um Host cadastrado — por alias ou hostname.
 * É o que deixa uma sessão de shell herdar o contexto da conexão enquanto o ssh
 * está vivo, sem inventar um Host que o usuário não cadastrou.
 */
export function matchSshHost(
  command: string | null,
  hosts: Host[],
): Host | null {
  if (!command) return null;
  const parsed = parseSshCommand(command);
  if (!parsed) return null;
  const byAlias = hosts.find((h) => h.alias === parsed.target);
  if (byAlias) return byAlias;
  return (
    hosts.find(
      (h) =>
        h.hostname === parsed.target &&
        (parsed.user === null ||
          h.username === null ||
          h.username === parsed.user),
    ) ?? null
  );
}

import type { SessionKind } from "@/lib/ipc"

/**
 * O PTY morrer significa que a sessão acabou?
 *
 * Para SSH, não: a sessão mora no host e o `ssh` é só o cano. Wifi caindo,
 * sleep e ⌘Q matam o cano, o core reata, e o trabalho continua do outro lado.
 * Escrever "[sessão encerrada]" aí diz ao dono o oposto do que aconteceu, no
 * exato instante em que ele teme ter perdido o trabalho.
 *
 * Nem quando o backoff desiste isso vira verdade: o estado é "conexão perdida",
 * com botão de reconectar — a sessão continua viva no host. E quando ela de
 * fato acaba (o dono deu `exit`), o core a descarta e o pane some da tela: não
 * sobra ninguém para ler o banner.
 */
export function ptyExitEndsSession(kind: SessionKind): boolean {
  return kind.type !== "ssh"
}

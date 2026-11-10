import { useTranslation } from "react-i18next";
import { HardDrives, Plug, Plus } from "@phosphor-icons/react";

import type { TypedSshDetected } from "../lib/ipc";
import { targetLabel } from "../lib/typedSsh";

export type SshNoteAction = "open" | "dismiss" | "never";

/**
 * O aviso do `ssh` digitado (quadros 1A/1B de SshVariaveisFila): mora no topo
 * do bloco vivo, entre o comando e a saída, e só oferece — quem abre a sessão
 * integrada é o gestor de SSH, e nada é escrito no PTY vivo.
 */
export function SshTypedNote({
  note,
  alias,
  onAction,
}: {
  note: TypedSshDetected;
  /** O nome do Host cadastrado; `null` quando não cadastrado. */
  alias: string | null;
  onAction: (action: SshNoteAction) => void;
}) {
  const { t } = useTranslation();
  const registered = note.host_id !== null;
  return (
    <div
      role="status"
      className="tyba-piece mt-1.5 mr-2 ml-[18px] grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 rounded-[8px] px-3.5 py-3"
    >
      <span className="row-span-3 flex size-7 items-center justify-center rounded-[7px] bg-tyba-blue-tint text-tyba-blue">
        <Plug size={16} />
      </span>
      <div className="flex min-w-0 flex-wrap items-center gap-2.5 text-[13px] font-medium text-tyba-text">
        {t("sshNoteTitle")}
        <span className="inline-flex h-[22px] items-center gap-1.5 rounded-[5px] px-2 font-mono text-[11.5px] font-normal text-tyba-text-muted shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
          <HardDrives size={12} />
          {targetLabel(note.target)}
        </span>
        <span className="text-[11.5px] font-normal text-tyba-text-faint">
          {registered && alias ? (
            <>
              {t("sshNoteRegisteredAs")} <span className="text-tyba-text-muted">{alias}</span>
            </>
          ) : (
            t("sshNoteUnregistered")
          )}
        </span>
      </div>
      <p className="m-0 text-[12px] leading-[18px] text-tyba-text-muted">
        {registered ? t("sshNoteRegisteredText") : t("sshNoteUnregisteredText")}
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <button
          onClick={() => onAction("open")}
          className="inline-flex h-7 items-center gap-1.5 rounded-[6px] bg-tyba-green px-3 text-[12px] font-medium text-tyba-text-inverse hover:bg-tyba-green/90"
        >
          {registered ? <HardDrives size={13} /> : <Plus size={13} />}
          {registered ? t("sshNoteOpen") : t("sshNoteRegister")}
        </button>
        <button
          onClick={() => onAction("dismiss")}
          className="inline-flex h-7 items-center rounded-[6px] px-3 text-[12px] text-tyba-text-muted hover:text-tyba-text"
        >
          {t("sshNoteNotNow")}
        </button>
        <button
          onClick={() => onAction("never")}
          className="inline-flex h-7 items-center rounded-[6px] px-3 text-[12px] text-tyba-text-faint hover:text-tyba-text"
        >
          {t("sshNoteNever")}
        </button>
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ClockCounterClockwise, FolderOpen, House } from "@phosphor-icons/react";

import {
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { getPref, setPref } from "@/lib/ipc";
import { lastDir as orbitLastDir } from "@/lib/orbit";

const LAST_DIR_KEY = "pref.last_session_dir";

/**
 * "Onde esse terminal vai rodar?" (quadro OrbitTerminal 2): o seletor de
 * pasta da Nova sessão, com a última pasta, Escolher pasta… e a pasta
 * pessoal. Escolher entrega a pasta; fechar sem escolher não cria nada.
 */
export function TerminalFolderPrompt({
  open,
  home,
  onChoose,
  onClose,
}: {
  open: boolean;
  /** A pasta pessoal (`~`); vazia enquanto não se sabe. */
  home: string;
  onChoose: (dir: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [last, setLast] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    void getPref(LAST_DIR_KEY)
      .then((v) => setLast(v || orbitLastDir()))
      .catch(() => setLast(orbitLastDir()));
  }, [open]);

  const choose = (dir: string) => {
    if (dir) void setPref(LAST_DIR_KEY, dir).catch(() => {});
    onChoose(dir);
  };

  const pickFolder = async () => {
    onClose();
    const dir = await openFileDialog({ directory: true, multiple: false }).catch(() => null);
    if (typeof dir === "string") choose(dir);
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={(v) => !v && onClose()}
      title={t("orbitTerminalWhere")}
      description={t("orbitTerminalWhere")}
      showCloseButton={false}
      className="top-28 max-w-[480px] translate-y-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface shadow-2xl"
    >
      <CommandInput placeholder={t("orbitTerminalWhere")} />
      <CommandList>
        <CommandGroup>
          {last && (
            <CommandItem onSelect={() => choose(last)}>
              <ClockCounterClockwise size={15} />
              <span className="min-w-0 flex-1 truncate">{last}</span>
              <span className="ml-auto font-mono text-[10px] text-tyba-text-faint">
                {t("lastFolder")}
              </span>
            </CommandItem>
          )}
          <CommandItem onSelect={() => void pickFolder()}>
            <FolderOpen size={15} />
            {t("chooseFolder")}
          </CommandItem>
          <CommandItem onSelect={() => choose(home)}>
            <House size={15} />
            {t("homeFolder")}
          </CommandItem>
        </CommandGroup>
      </CommandList>
      <div className="flex items-center gap-4 tyba-divide-t px-3 py-1.5 font-mono text-[10px] text-tyba-text-faint">
        <span>↑↓ {t("hintNavigate")}</span>
        <span>↵ {t("hintOpen")}</span>
        <span className="ml-auto">esc {t("hintClose")}</span>
      </div>
    </CommandDialog>
  );
}

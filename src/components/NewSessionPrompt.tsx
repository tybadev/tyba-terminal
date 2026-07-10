import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ClockCounterClockwise,
  FolderOpen,
  GitBranch,
  House,
} from "@phosphor-icons/react";

import {
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { getPref, setPref } from "../lib/ipc";
import { basename } from "@/lib/utils";

const LAST_DIR_KEY = "pref.last_session_dir";
const DEFAULT_DIR_KEY = "pref.default_session_dir";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (cwd: string | null, name: string, isolate: boolean) => void;
  isolate: boolean;
  onIsolateChange: (value: boolean) => void;
}

export function NewSessionPrompt({
  open,
  onOpenChange,
  onCreate,
  isolate,
  onIsolateChange,
}: Props) {
  const { t } = useTranslation();
  const [lastDir, setLastDir] = useState<string | null>(null);
  const [defaultDir, setDefaultDir] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    void getPref(LAST_DIR_KEY)
      .then(setLastDir)
      .catch(() => setLastDir(null));
    void getPref(DEFAULT_DIR_KEY)
      .then((v) => setDefaultDir(v || null))
      .catch(() => setDefaultDir(null));
  }, [open]);

  const create = (dir: string | null) => {
    if (dir) void setPref(LAST_DIR_KEY, dir).catch(() => {});
    onCreate(dir, dir ? basename(dir) : "shell", isolate);
  };

  const chooseFolder = async () => {
    onOpenChange(false);
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir === "string") create(dir);
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("newSession")}
      description={t("newSessionWhere")}
      showCloseButton={false}
      className="top-28 max-w-[480px] translate-y-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface shadow-2xl"
    >
      <CommandInput placeholder={t("newSessionWhere")} />
      <CommandList>
        <CommandGroup>
          {defaultDir && (
            <CommandItem
              onSelect={() => {
                onOpenChange(false);
                create(defaultDir);
              }}
            >
              <FolderOpen size={15} className="text-tyba-green" />
              <span className="min-w-0 flex-1 truncate">{defaultDir}</span>
              <span className="ml-auto font-mono text-[10px] text-tyba-text-faint">
                {t("defaultFolder")}
              </span>
            </CommandItem>
          )}
          {lastDir && lastDir !== defaultDir && (
            <CommandItem
              onSelect={() => {
                onOpenChange(false);
                create(lastDir);
              }}
            >
              <ClockCounterClockwise size={15} />
              <span className="min-w-0 flex-1 truncate">{lastDir}</span>
              <span className="ml-auto font-mono text-[10px] text-tyba-text-faint">
                {t("lastFolder")}
              </span>
            </CommandItem>
          )}
          <CommandItem onSelect={() => void chooseFolder()}>
            <FolderOpen size={15} />
            {t("chooseFolder")}
          </CommandItem>
          <CommandItem onSelect={() => onIsolateChange(!isolate)}>
            <GitBranch
              size={15}
              className={isolate ? "text-tyba-green" : undefined}
            />
            <span className="min-w-0 flex-1">{t("worktreeIsolate")}</span>
            {isolate && <Check size={14} className="ml-auto text-tyba-green" />}
          </CommandItem>
          <CommandItem
            onSelect={() => {
              onOpenChange(false);
              create(null);
            }}
          >
            <House size={15} />
            {t("homeFolder")}
          </CommandItem>
        </CommandGroup>
      </CommandList>
      <div className="flex items-center gap-4 border-t border-tyba-border px-3 py-1.5 font-mono text-[10px] text-tyba-text-faint">
        <span>↑↓ {t("hintNavigate")}</span>
        <span>↵ {t("hintRun")}</span>
        <span className="ml-auto">esc {t("hintClose")}</span>
      </div>
    </CommandDialog>
  );
}

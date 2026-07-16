import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ClockCounterClockwise,
  FolderOpen,
  GitBranch,
  HardDrives,
  House,
  TerminalWindow,
} from "@phosphor-icons/react";

import {
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import {
  getPref,
  listHosts,
  listShells,
  setPref,
  type Host,
  type ShellOption,
} from "../lib/ipc";
import { basename, cn } from "@/lib/utils";

const LAST_DIR_KEY = "pref.last_session_dir";
const DEFAULT_DIR_KEY = "pref.default_session_dir";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (
    cwd: string | null,
    name: string,
    isolate: boolean,
    shell?: string,
  ) => void;
  isolate: boolean;
  onIsolateChange: (value: boolean) => void;
  /** "Onde a sessão vai trabalhar" inclui os servidores, não só pastas daqui. */
  onConnectHost?: (host: Host) => void;
}

export function NewSessionPrompt({
  open,
  onOpenChange,
  onCreate,
  isolate,
  onIsolateChange,
  onConnectHost,
}: Props) {
  const { t } = useTranslation();
  const [hosts, setHosts] = useState<Host[]>([]);
  const [lastDir, setLastDir] = useState<string | null>(null);
  const [defaultDir, setDefaultDir] = useState<string | null>(null);
  const [shells, setShells] = useState<ShellOption[]>([]);
  const [shellId, setShellId] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    void listHosts()
      .then(setHosts)
      .catch(() => setHosts([]));
    void getPref(LAST_DIR_KEY)
      .then(setLastDir)
      .catch(() => setLastDir(null));
    void getPref(DEFAULT_DIR_KEY)
      .then((v) => setDefaultDir(v || null))
      .catch(() => setDefaultDir(null));
    void listShells()
      .then((list) => {
        setShells(list);
        setShellId((prev) => prev ?? list[0]?.id ?? null);
      })
      .catch(() => setShells([]));
  }, [open]);

  const create = (dir: string | null) => {
    if (dir) void setPref(LAST_DIR_KEY, dir).catch(() => {});
    onCreate(dir, dir ? basename(dir) : "shell", isolate, shellId ?? undefined);
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
      {shells.length > 1 && (
        <div className="flex items-center gap-1.5 border-b border-tyba-border px-3 py-1.5">
          <TerminalWindow size={13} className="shrink-0 text-tyba-text-faint" />
          <div className="flex min-w-0 flex-wrap gap-1">
            {shells.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => setShellId(s.id)}
                className={cn(
                  "rounded-[4px] px-2 py-0.5 text-[11px] transition-colors",
                  s.id === shellId
                    ? "bg-tyba-green/15 text-tyba-green"
                    : "text-tyba-text-faint hover:text-tyba-text",
                )}
              >
                {s.label}
              </button>
            ))}
          </div>
        </div>
      )}
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
        {hosts.length > 0 && onConnectHost && (
          <CommandGroup heading={t("connectionsTitle")}>
            {hosts.map((host) => (
              <CommandItem
                key={host.id}
                value={`ssh ${host.alias} ${host.hostname}`}
                onSelect={() => {
                  onOpenChange(false);
                  onConnectHost(host);
                }}
              >
                <span
                  className="size-1.5 shrink-0 rounded-full"
                  style={{
                    background: host.color
                      ? `var(--tyba-${host.color})`
                      : "var(--tyba-text-faint)",
                  }}
                />
                <HardDrives size={15} className="shrink-0 opacity-60" />
                <span className="min-w-0 flex-1 truncate">{host.alias}</span>
                <span className="ml-auto shrink-0 truncate font-mono text-[10px] text-tyba-text-faint">
                  {host.username ? `${host.username}@` : ""}
                  {host.hostname}
                </span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
      </CommandList>
      <div className="flex items-center gap-4 border-t border-tyba-border px-3 py-1.5 font-mono text-[10px] text-tyba-text-faint">
        <span>↑↓ {t("hintNavigate")}</span>
        <span>↵ {t("hintRun")}</span>
        <span className="ml-auto">esc {t("hintClose")}</span>
      </div>
    </CommandDialog>
  );
}

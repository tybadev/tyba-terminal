import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { HardDrives } from "@phosphor-icons/react";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { listHostGroups, listHosts, type Host, type HostGroup } from "@/lib/ipc";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onPick: (host: Host) => void;
}

export function HostPicker({ open, onOpenChange, onPick }: Props) {
  const { t } = useTranslation();
  const [hosts, setHosts] = useState<Host[]>([]);
  const [groups, setGroups] = useState<HostGroup[]>([]);

  useEffect(() => {
    if (!open) return;
    void Promise.all([listHosts(), listHostGroups()])
      .then(([h, g]) => {
        setHosts(h);
        setGroups(g);
      })
      .catch(() => {});
  }, [open]);

  const groupName = (id: string | null): string =>
    groups.find((g) => g.id === id)?.name ?? t("connectionsNoGroup");

  const sections = [...new Set(hosts.map((h) => groupName(h.group_id)))];

  const pick = (host: Host) => {
    onOpenChange(false);
    onPick(host);
  };

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange}>
      <CommandInput placeholder={t("hostPickerPlaceholder")} />
      <CommandList>
        <CommandEmpty>{t("hostPickerEmpty")}</CommandEmpty>
        {sections.map((section) => (
          <CommandGroup key={section} heading={section}>
            {hosts
              .filter((h) => groupName(h.group_id) === section)
              .map((host) => (
                <CommandItem
                  key={host.id}
                  value={`${host.alias} ${host.hostname} ${host.username ?? ""}`}
                  onSelect={() => pick(host)}
                  className="gap-2"
                >
                  <span
                    className="size-1.5 shrink-0 rounded-full"
                    style={{
                      background: host.color
                        ? `var(--tyba-${host.color})`
                        : "var(--tyba-text-faint)",
                    }}
                  />
                  <HardDrives size={14} className="shrink-0 opacity-60" />
                  <span className="truncate">{host.alias}</span>
                  <span className="ml-auto truncate font-mono text-[10px] text-tyba-text-faint">
                    {host.username ? `${host.username}@` : ""}
                    {host.hostname}
                  </span>
                </CommandItem>
              ))}
          </CommandGroup>
        ))}
      </CommandList>
    </CommandDialog>
  );
}

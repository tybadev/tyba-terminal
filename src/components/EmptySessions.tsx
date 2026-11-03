import type { ReactNode } from "react";
import {
  HardDrives,
  MagnifyingGlass,
  Plus,
  SidebarSimple,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import { TybaMark } from "@/components/icons/TybaMark";
import { PLATE, PLATE_ACRYLIC, PLATE_LINE } from "@/components/plate";
import { Shortcut } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { primaryAction, recentHosts } from "@/lib/emptySessions";
import type { Host } from "@/lib/ipc";
import type { Bindings } from "@/lib/keys";

export interface EmptySessionsProps {
  /** Há workspace aberto (só faltam abas) ou a janela está vazia de vez. */
  hasWorkspaces: boolean;
  /** Os atalhos do usuário; a tela nunca mostra tecla fixa. */
  bindings: Pick<Bindings, "newSession" | "newTab" | "paletteActions" | "panel">;
  /** Hosts SSH salvos: decidem o rótulo do SSH e alimentam "Recentes". */
  hosts: Host[];
  onNewSession: () => void;
  onNewTab: () => void;
  /** Abre o seletor de host, ou a tela de Conexões SSH se não houver host. */
  onConnectSsh: () => void;
  onPalette: () => void;
  onTogglePanel: () => void;
  onPickHost: (host: Host) => void;
}

const ROW =
  "flex h-9 w-full items-center gap-2.5 rounded-[6px] px-2.5 text-left text-[13px] transition-colors";
const ROW_IDLE =
  "text-tyba-text-muted hover:bg-tyba-text/[.04] hover:text-tyba-text";

function Row({
  icon,
  label,
  detail,
  combo,
  primary,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  detail?: string;
  combo?: string;
  primary?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(ROW, primary ? "tyba-pill-on text-tyba-text" : ROW_IDLE)}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">
        {label}
        {detail && (
          <span className="ml-2 font-mono text-[11px] text-tyba-text-faint">
            {detail}
          </span>
        )}
      </span>
      {combo && <Shortcut combo={combo} />}
    </button>
  );
}

/** `user@host:porta`, só com o que o host tem. */
function hostDetail(h: Host): string {
  const user = h.username ? `${h.username}@` : "";
  const port = h.port ? `:${h.port}` : "";
  return `${user}${h.hostname}${port}`;
}

/**
 * A folha sem sessão (ou sem aba): a marca, o título, as ações numa placa de
 * acrílico com a principal acesa e, se houver, os hosts SSH usados por último.
 */
export function EmptySessions({
  hasWorkspaces,
  bindings,
  hosts,
  onNewSession,
  onNewTab,
  onConnectSsh,
  onPalette,
  onTogglePanel,
  onPickHost,
}: EmptySessionsProps) {
  const { t } = useTranslation();
  const primary = primaryAction(hasWorkspaces, bindings);
  const recent = recentHosts(hosts);

  return (
    <div className="flex w-[320px] max-w-[calc(100%-32px)] flex-col items-center gap-5">
      <div className="flex flex-col items-center gap-2 text-center">
        <TybaMark size={28} />
        <p className="mt-1 text-[15px] font-medium text-tyba-text">
          {hasWorkspaces ? t("noTabs") : t("noSessions")}
        </p>
        <p className="text-[12px] text-tyba-text-faint">
          {hasWorkspaces ? t("emptyTabsHint") : t("emptySessionsHint")}
        </p>
      </div>

      <div
        className={cn(
          PLATE,
          PLATE_LINE,
          PLATE_ACRYLIC,
          "flex w-full flex-col gap-px p-1",
        )}
      >
        <Row
          primary
          icon={<Plus size={14} className="text-tyba-green" />}
          label={primary.kind === "newTab" ? t("newTab") : t("newSession")}
          combo={primary.combo}
          onClick={primary.kind === "newTab" ? onNewTab : onNewSession}
        />
        <Row
          icon={<HardDrives size={14} className="text-tyba-cyan" />}
          label={hosts.length > 0 ? t("connectSsh") : t("connectionsTitle")}
          onClick={onConnectSsh}
        />
        <Row
          icon={<MagnifyingGlass size={14} />}
          label={t("commandPalette")}
          combo={bindings.paletteActions}
          onClick={onPalette}
        />
        <Row
          icon={<SidebarSimple size={14} />}
          label={t("togglePanel")}
          combo={bindings.panel}
          onClick={onTogglePanel}
        />
      </div>

      {recent.length > 0 && (
        <div className="flex w-full flex-col gap-px">
          <p className="px-2.5 pb-1 font-mono text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase">
            {t("emptyRecent")}
          </p>
          {recent.map((h) => (
            <Row
              key={h.id}
              icon={<HardDrives size={14} className="text-tyba-text-faint" />}
              label={h.alias}
              detail={hostDetail(h)}
              onClick={() => onPickHost(h)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

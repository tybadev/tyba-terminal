import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import {
  ArrowBendUpLeft,
  ArrowRight,
  Circle,
  Command,
  FolderSimple,
  HardDrives,
  Monitor,
  Plus,
  Rows,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import { ClaudeIcon } from "@/components/icons/ClaudeIcon";
import { OpenAIIcon } from "@/components/icons/OpenAIIcon";
import { ShellGlyph } from "@/components/icons/ShellGlyph";
import { TybaMark } from "@/components/icons/TybaMark";
import { PLATE, PLATE_ACRYLIC, PLATE_LINE } from "@/components/plate";
import { Button } from "@/components/ui/button";
import { Shortcut } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import {
  INITIAL_CHOICE,
  agentsAllowed,
  chosenHost,
  effectiveWhat,
  hostAddress,
  launchPlan,
  launchSummary,
  pickFolder,
  pickHost,
  recentHosts,
  RECENT_MAX,
  type LaunchChoice,
  type LaunchPlan,
  type LaunchWhat,
} from "@/lib/emptySessions";
import { listRecentFolders, type Host, type RecentFolder } from "@/lib/ipc";
import { comboOf, formatCombo, SEND_PROMPT_COMBO, type Bindings } from "@/lib/keys";
import { formatLastConnected } from "@/lib/relativeTime";
import { compactPath, dirLabel } from "@/lib/workspaceCwd";

export interface EmptySessionsProps {
  /** Há workspace aberto (só faltam abas) ou a janela está vazia de vez. */
  hasWorkspaces: boolean;
  /** Os atalhos do usuário; a tela nunca mostra tecla fixa. */
  bindings: Pick<Bindings, "newTab" | "openFolder" | "paletteActions" | "toggleOrbit">;
  /** Hosts SSH salvos: o host do cartão e a coluna de recentes. */
  hosts: Host[];
  /** Abre o que o cartão escolheu, pelas intenções que o App já tem. */
  onLaunch: (plan: LaunchPlan) => void;
  /** O diálogo de pasta do sistema; `null` se a pessoa cancelou. */
  onChooseFolder: () => Promise<string | null>;
  onNewTab: () => void;
  /** Abre o seletor de host, ou a tela de Conexões SSH se não houver host. */
  onConnectSsh: () => void;
  onOrbit: () => void;
  onPalette: () => void;
}

// Peça do cartão (Onde / Com quê): vidro .5 com o fio fino em repouso, que
// clareia para .8 no hover; escolhida, a placa acesa com o contorno inteiro na
// cor da marca (azul no claro, verde no escuro).
const TILE =
  "flex min-w-0 flex-col gap-1 rounded-[10px] px-3.5 py-3 text-left text-tyba-text outline-none transition-colors disabled:cursor-not-allowed disabled:opacity-40";
const TILE_IDLE =
  "bg-[var(--tyba-mk-glass-tile,transparent)] shadow-[inset_0_0_0_1px_var(--tyba-line)] enabled:hover:bg-[var(--tyba-mk-glass-strong,var(--tyba-hover))] focus-visible:[box-shadow:inset_0_0_0_1px_var(--tyba-line),var(--tyba-focus-ring)]";
const TILE_ON =
  "tyba-pill-on [box-shadow:inset_0_0_0_1.5px_var(--primary),var(--tyba-pill-on)]! focus-visible:[box-shadow:inset_0_0_0_1.5px_var(--primary),var(--tyba-focus-ring)]!";

// Linha da coluna de recentes: mesma regra da peça, em 36px; no hover, o
// branco .96 com a sombra do knob (quadro `.rw`).
const ROW =
  "flex h-9 w-full items-center gap-2.5 rounded-[10px] px-3 text-left text-tyba-text outline-none transition-colors";
const ROW_IDLE =
  "hover:bg-[var(--tyba-mk-raised,var(--tyba-hover))] hover:[box-shadow:var(--tyba-mk-hover-shadow,none)] focus-visible:[box-shadow:var(--tyba-focus-ring)]";

// Botão da linha compacta (Conexões SSH, Orbit, Paleta), quadro `.mini`:
// branco .6 com o fio; no hover, branco .96 com a sombra do knob.
const MINI =
  "inline-flex h-10 items-center gap-2 rounded-[10px] bg-[var(--tyba-mk-glass-mini,var(--tyba-control-bg))] px-3.5 text-[13px] font-semibold text-tyba-text outline-none transition-colors [box-shadow:var(--tyba-mk-mini-shadow,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] hover:bg-[var(--tyba-mk-raised,color-mix(in_srgb,var(--tyba-text)_4%,var(--tyba-control-bg)))] hover:[box-shadow:var(--tyba-mk-hover-shadow,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] focus-visible:[box-shadow:var(--tyba-focus-ring)]";

/** Setas andam o foco dentro do grupo, como num radiogroup nativo. */
function moveFocus(e: KeyboardEvent<HTMLDivElement>) {
  const step =
    e.key === "ArrowRight" || e.key === "ArrowDown"
      ? 1
      : e.key === "ArrowLeft" || e.key === "ArrowUp"
        ? -1
        : 0;
  if (step === 0) return;
  const radios = Array.from(
    e.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]:not(:disabled)'),
  );
  const at = radios.indexOf(document.activeElement as HTMLButtonElement);
  if (at < 0 || radios.length === 0) return;
  e.preventDefault();
  radios[(at + step + radios.length) % radios.length].focus();
}

function Tile({
  on,
  disabled,
  icon,
  corner,
  name,
  detail,
  onClick,
}: {
  on: boolean;
  disabled?: boolean;
  icon: ReactNode;
  corner?: ReactNode;
  name: string;
  detail: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={on}
      disabled={disabled}
      onClick={onClick}
      className={cn(TILE, on ? TILE_ON : TILE_IDLE)}
    >
      <span
        className={cn(
          "flex h-[22px] items-center",
          on ? "text-primary" : "text-tyba-text-muted",
        )}
      >
        {icon}
        {corner && <span className="ml-auto">{corner}</span>}
      </span>
      <span className="truncate text-[13.5px] font-bold">{name}</span>
      <span className="truncate font-mono text-[11px] text-tyba-text-muted">
        {detail}
      </span>
    </button>
  );
}

function RecentColumn({
  title,
  empty,
  children,
}: {
  title: string;
  empty: string;
  children: ReactNode[];
}) {
  const { t } = useTranslation();
  return (
    <div className="min-w-0">
      <div className="tyba-label flex items-center gap-2 px-1 pb-1.5">
        {title}
        {children.length > 0 && (
          <span className="ml-auto inline-flex items-center gap-1 text-[10.5px] font-medium tracking-normal text-primary normal-case">
            <ArrowBendUpLeft size={12} />
            {t("recentPickTip")}
          </span>
        )}
      </div>
      {/* Quadro `.box`: raio 14, branco .55 com o fio, padding 4, sem sombra;
          as linhas encostadas. */}
      <div className="flex flex-col rounded-[14px] bg-[var(--tyba-mk-glass-soft,var(--tyba-glass-section))] p-1 shadow-[inset_0_0_0_1px_var(--tyba-line)] backdrop-blur-[28px] backdrop-saturate-150">
        {children.length > 0 ? (
          children
        ) : (
          <p className="flex h-9 items-center px-3 text-[12px] text-tyba-text-faint">
            {empty}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * A tela inicial sem sessão (quadro "Tela inicial · A+B", 2026-11-19): a marca
 * e o título, o cartão de lançamento (Onde × Com quê, o resumo e "Abrir
 * sessão"), a linha compacta e os recentes em duas colunas. Com workspace e
 * sem aba, a mesma tela com "Nova tab" à frente da linha compacta.
 */
export function EmptySessions({
  hasWorkspaces,
  bindings,
  hosts,
  onLaunch,
  onChooseFolder,
  onNewTab,
  onConnectSsh,
  onOrbit,
  onPalette,
}: EmptySessionsProps) {
  const { t, i18n } = useTranslation();
  const [choice, setChoice] = useState<LaunchChoice>(INITIAL_CHOICE);
  const [folders, setFolders] = useState<RecentFolder[]>([]);

  useEffect(() => {
    let alive = true;
    // Sem o command no core (ainda), a coluna fica vazia.
    listRecentFolders()
      .then((list) => alive && setFolders(list.slice(0, RECENT_MAX)))
      .catch(() => alive && setFolders([]));
    return () => {
      alive = false;
    };
  }, []);

  const recent = recentHosts(hosts);
  const host = chosenHost(choice, hosts);
  const what = effectiveWhat(choice);
  const remote = !agentsAllowed(choice);
  const plan = launchPlan(choice, hosts);
  const summary = launchSummary(choice, hosts);
  const openCombo = formatCombo(SEND_PROMPT_COMBO);

  const launch = () => {
    if (plan) onLaunch(plan);
  };

  // ⌘⏎ abre de qualquer ponto da tela; dentro de um diálogo é dele.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (comboOf(e) !== SEND_PROMPT_COMBO) return;
      if ((e.target as Element | null)?.closest?.('[role="dialog"]')) return;
      if (!plan) return;
      e.preventDefault();
      onLaunch(plan);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [plan, onLaunch]);

  const chooseFolder = async () => {
    const dir = await onChooseFolder();
    if (dir) setChoice((c) => pickFolder(c, dir));
  };

  // Pasta: sem pasta (ou já escolhida) abre o diálogo; com pasta, só escolhe.
  const onFolderTile = () => {
    if (choice.where === "folder" || !choice.folder) void chooseFolder();
    else setChoice((c) => ({ ...c, where: "folder" }));
  };

  // Host: o último escolhido, senão o mais recente, senão o primeiro salvo.
  // Sem host salvo, a tela de Conexões SSH.
  const onHostTile = () => {
    const target = host ?? recent[0] ?? hosts[0] ?? null;
    if (target) setChoice((c) => pickHost(c, target.id));
    else onConnectSsh();
  };

  const pickWhat = (w: LaunchWhat) => setChoice((c) => ({ ...c, what: w }));

  const placeMissing =
    choice.where === "folder" ? t("startFolderPick") : t("startHostPick");

  return (
    <div className="flex w-[720px] max-w-[calc(100%-32px)] flex-col items-center gap-4">
      <div className="flex flex-col items-center gap-1.5 text-center">
        <TybaMark size={56} />
        <h1 className="mt-1 text-[28px] leading-[34px] font-bold tracking-[-0.02em] text-tyba-text">
          {hasWorkspaces ? t("noTabs") : t("noSessions")}
        </h1>
        <p className="text-[14px] leading-[21px] text-tyba-text-muted">
          {t("startHint", { combo: openCombo })}
        </p>
      </div>

      <section
        aria-label={t("startCard")}
        className={cn(
          PLATE,
          PLATE_LINE,
          PLATE_ACRYLIC,
          // Quadro: `mk-glass is-strong is-float`, raio 18.
          "is-strong flex w-full flex-col gap-4 rounded-[18px] px-[22px] py-5",
        )}
      >
        <div className="flex flex-col gap-2">
          <div id="launch-where" className="tyba-label">
            {t("startWhere")}
          </div>
          <div
            role="radiogroup"
            aria-labelledby="launch-where"
            onKeyDown={moveFocus}
            className="grid grid-cols-3 gap-2.5"
          >
            <Tile
              on={choice.where === "local"}
              icon={<Monitor size={20} />}
              name={t("startLocal")}
              detail="~"
              onClick={() => setChoice((c) => ({ ...c, where: "local" }))}
            />
            <Tile
              on={choice.where === "folder"}
              icon={<FolderSimple size={20} />}
              corner={<Shortcut combo={bindings.openFolder} joined />}
              name={t("startFolder")}
              detail={choice.folder ? compactPath(choice.folder) : t("startFolderPick")}
              onClick={onFolderTile}
            />
            <Tile
              on={remote}
              icon={<HardDrives size={20} />}
              name={host ? `${t("startHost")} · ${host.alias}` : t("startHost")}
              detail={
                host
                  ? hostAddress(host)
                  : hosts.length > 0
                    ? t("startHostPick")
                    : t("startHostNone")
              }
              onClick={onHostTile}
            />
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <div id="launch-what" className="tyba-label">
            {t("startWhat")}
          </div>
          <div
            role="radiogroup"
            aria-labelledby="launch-what"
            onKeyDown={moveFocus}
            className="grid grid-cols-3 gap-2.5"
          >
            <Tile
              on={what === "shell"}
              icon={<ShellGlyph size={20} />}
              name={t("startShell")}
              detail={t("startShellDetail")}
              onClick={() => pickWhat("shell")}
            />
            <Tile
              on={what === "claude"}
              disabled={remote}
              icon={<ClaudeIcon size={20} style={{ color: "#d97757" }} />}
              name="Claude Code"
              detail={t("startClaudeDetail")}
              onClick={() => pickWhat("claude")}
            />
            <Tile
              on={what === "codex"}
              disabled={remote}
              icon={<OpenAIIcon size={20} className="text-tyba-text" />}
              name="Codex"
              detail={t("startCodexDetail")}
              onClick={() => pickWhat("codex")}
            />
          </div>
          {remote && (
            <span className="text-[12px] text-tyba-text-muted">
              {t("startRemoteNote")}
            </span>
          )}
        </div>

        <div className="flex items-center gap-3 border-t border-[var(--tyba-line)] pt-3.5">
          <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text-muted">
            {summary.place
              ? t("startSummary", { what: summary.what, place: summary.place })
              : placeMissing}
          </span>
          <Button type="button" disabled={!plan} onClick={launch}>
            <ArrowRight />
            {t("startOpen")}
            <Shortcut combo={SEND_PROMPT_COMBO} joined />
          </Button>
        </div>
      </section>

      <nav aria-label={t("startShortcuts")} className="flex flex-wrap justify-center gap-2.5">
        {hasWorkspaces && (
          <button type="button" className={MINI} onClick={onNewTab}>
            <Plus size={16} className="text-primary" />
            {t("newTab")}
            <Shortcut combo={bindings.newTab} joined />
          </button>
        )}
        <button type="button" className={MINI} onClick={onConnectSsh}>
          <Rows size={16} className="text-primary" />
          {t("connectionsTitle")}
        </button>
        <button type="button" className={MINI} onClick={onOrbit}>
          <Circle size={16} className="text-primary" />
          {t("orbitView")}
          <Shortcut combo={bindings.toggleOrbit} joined />
        </button>
        <button type="button" className={MINI} onClick={onPalette}>
          <Command size={16} className="text-primary" />
          {t("commandPalette")}
          <Shortcut combo={bindings.paletteActions} joined />
        </button>
      </nav>

      <div className="grid w-full grid-cols-2 gap-3">
        <RecentColumn title={t("recentHostsTitle")} empty={t("recentHostsEmpty")}>
          {recent.map((h) => (
            <button
              key={h.id}
              type="button"
              aria-pressed={remote && choice.hostId === h.id}
              onClick={() => setChoice((c) => pickHost(c, h.id))}
              className={cn(
                ROW,
                remote && choice.hostId === h.id ? TILE_ON : ROW_IDLE,
              )}
            >
              <HardDrives size={16} className="shrink-0 text-primary" />
              <span className="shrink-0 text-[13px] font-semibold">{h.alias}</span>
              <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-tyba-text-muted">
                {hostAddress(h)}
              </span>
              <span className="shrink-0 text-[11.5px] font-medium text-tyba-text-faint">
                {formatLastConnected(h.last_connected_at ?? "", i18n.language)}
              </span>
            </button>
          ))}
        </RecentColumn>
        <RecentColumn title={t("recentFoldersTitle")} empty={t("recentFoldersEmpty")}>
          {folders.map((f) => {
            const on = choice.where === "folder" && choice.folder === f.path;
            return (
              <button
                key={f.path}
                type="button"
                aria-pressed={on}
                onClick={() => setChoice((c) => pickFolder(c, f.path))}
                className={cn(ROW, on ? TILE_ON : ROW_IDLE)}
              >
                <FolderSimple size={16} className="shrink-0 text-tyba-text-muted" />
                <span className="shrink-0 text-[13px] font-semibold">{dirLabel(f.path)}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-tyba-text-muted">
                  {compactPath(f.path)}
                </span>
                <span className="shrink-0 text-[11.5px] font-medium text-tyba-text-faint">
                  {formatLastConnected(f.last_opened_at_ms, i18n.language)}
                </span>
              </button>
            );
          })}
        </RecentColumn>
      </div>
    </div>
  );
}

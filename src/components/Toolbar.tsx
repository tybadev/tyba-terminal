import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowDown,
  ArrowUp,
  Folder,
  GitBranch,
  GitDiff,
  PuzzlePiece,
  TextAlignLeft,
} from "@phosphor-icons/react";

import { Shortcut } from "@/components/ui/kbd";
import { installClaudePlugin, type RepoSnapshot } from "../lib/ipc";
import { translateError } from "../lib/errors";
import { toastError } from "../lib/toast";
import type { BranchChip, ChipId, ToolbarPref } from "../lib/repoSnapshots";
import {
  FOOTER_STEP,
  FOOTER_TOP_STEP,
  HINT_LABEL_STEP,
  footerKeeps,
} from "../lib/appFooter";
import { pickStripLevel } from "../lib/orbitStrip";
import { BranchPicker } from "./BranchPicker";

interface Props {
  pref: ToolbarPref;
  cwd: string | null;
  /**
   * O chip de branch, já resolvido: o rótulo e a sessão que o checkout usa.
   *
   * Vem pronto, e não derivado de `snapshot`, porque os dois têm de falar do
   * MESMO repositório. `snapshot` é do workspace; o checkout do core resolve o
   * worktree da sessão. Enquanto o chip lia um e agia sobre o outro, dava para
   * trocar de branch num repositório vendo a branch de outro na tela.
   *
   * `sessionId` nulo deixa o chip como texto — é o que sobra quando não há
   * sessão, ou quando o core não teria como localizar o repositório dela.
   * `state: "unknown"` é o chip que fica na barra sem rótulo nem ação; `null`
   * é o único caso em que ele não existe (não há repositório nenhum em jogo).
   */
  branch: BranchChip | null;
  snapshot: RepoSnapshot | undefined;
  /**
   * Arquivos mudados no SERVIDOR, numa sessão SSH (regra 23).
   *
   * Vem à parte de `snapshot` porque o canal do servidor devolve a contagem e
   * nada mais — não há `+n/−n` do outro lado. `null` é "não mostrar": sessão
   * local, servidor limpo, ou sem canal para perguntar.
   */
  remoteChanged?: number | null;
  hasWorktree: boolean;
  onOpenDiff: () => void;
  showRichInput: boolean;
  richInputCombo: string;
  onOpenRichInput: () => void;
  /** Agente livre, fatia 4: o core achou um claude neste pane sem o plugin
   * do TYBA. O chip instala; quando o core confirma, o payload volta sem a
   * sugestão e o chip some. */
  suggestPlugin: boolean;
  /** O pane do chip: o core instala no config do Claude daquele claude. */
  sessionId: string | null;
  /**
   * Dicas de acesso no canto esquerdo (como as do Warp): tecla + o que faz.
   * Clicáveis — a mesma ação do atalho. `combo` vem dos bindings do usuário,
   * nunca fixo, para a dica não ensinar uma tecla que ele trocou.
   */
  hints?: {
    key: string;
    label: string;
    combo?: string;
    /** Dica sem tecla (atalhos): o ícone fica quando o rótulo sai. */
    icon?: ReactNode;
    onClick: () => void;
  }[];
}

/** gap-4 entre os grupos e px-3.5 dos dois lados. */
const FOOTER_CHROME = 16 + 28;

/**
 * Degrau da escada do rodapé (quadro GavetaPaneEstreito, Rodapé): mede a
 * largura natural dos dois grupos no degrau em que está, guarda por degrau e
 * escolhe o menor que cabe, com folga para voltar (o mesmo de
 * `useStripLevel` da faixa do Orbit). Sem breakpoint fixo.
 */
function useFooterStep(
  row: RefObject<HTMLElement | null>,
  left: RefObject<HTMLElement | null>,
  right: RefObject<HTMLElement | null>,
) {
  const [step, setStep] = useState(0);
  const current = useRef(0);
  const needs = useRef<(number | undefined)[]>([]);

  const measure = useCallback(() => {
    const r = row.current;
    const l = left.current;
    const rt = right.current;
    if (!r || !l || !rt) return;
    const at = current.current;
    const need = l.offsetWidth + rt.offsetWidth + FOOTER_CHROME;
    if (needs.current[at] !== need) {
      if (needs.current[at] !== undefined) needs.current = [];
      needs.current[at] = need;
    }
    setStep(pickStripLevel(needs.current, r.clientWidth, at, FOOTER_TOP_STEP));
  }, [row, left, right]);

  // A cada render: o conteúdo muda (estado da pílula, branch) sem o rodapé
  // mudar de tamanho.
  useLayoutEffect(() => {
    current.current = step;
    measure();
  });

  useEffect(() => {
    const ro = new ResizeObserver(() => measure());
    for (const el of [row.current, left.current, right.current]) if (el) ro.observe(el);
    return () => ro.disconnect();
  }, [measure, row, left, right]);

  return step;
}

/** O chip de instalar o plugin: só emite a intenção. Quem decide se ainda
 * falta é o core, que reemite a detecção depois de instalar. */
function PluginChip({ sessionId }: { sessionId: string | null }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  return (
    <button
      onClick={() => {
        setBusy(true);
        installClaudePlugin(sessionId ?? undefined)
          .catch((e) => toastError(t("claudePluginFailed"), translateError(e, t)))
          .finally(() => setBusy(false));
      }}
      disabled={busy}
      title={t("pluginChipHint")}
      aria-label={t("pluginChipHint")}
      className="flex shrink-0 items-center gap-1 rounded-[4px] px-1 py-0.5 text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text disabled:opacity-60"
    >
      <PuzzlePiece size={12} />
      <span>{busy ? t("pluginChipBusy") : t("pluginChip")}</span>
    </button>
  );
}

function ClockChip() {
  const { i18n } = useTranslation();
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 30_000);
    return () => clearInterval(id);
  }, []);
  const time = new Intl.DateTimeFormat(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(now);
  return <span className="font-mono tabular-nums">{time}</span>;
}

function basename(path: string): string {
  const trimmed = path.endsWith("/") ? path.slice(0, -1) : path;
  const last = trimmed.split("/").pop();
  return last || trimmed;
}

export function Toolbar({
  pref,
  cwd,
  branch,
  snapshot,
  remoteChanged,
  hasWorktree,
  onOpenDiff,
  showRichInput,
  richInputCombo,
  onOpenRichInput,
  suggestPlugin,
  hints = [],
  sessionId,
}: Props) {
  const { t } = useTranslation();
  const rootRef = useRef<HTMLDivElement>(null);
  const leftRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const step = useFooterStep(rootRef, leftRef, rightRef);
  const keeps = (at: number) => footerKeeps(step, at);
  if (!pref.enabled) return null;

  const chip = (id: ChipId) => {
    if (pref.hidden.includes(id)) return null;
    switch (id) {
      case "cwd":
        return cwd ? (
          <span
            key={id}
            title={cwd}
            className="flex min-w-0 shrink items-center gap-1"
            aria-label={t("toolbarCwd")}
          >
            <Folder size={12} className="shrink-0" />
            {keeps(FOOTER_STEP.cwdLabel) && (
              <span className="truncate">{basename(cwd)}</span>
            )}
          </span>
        ) : null;
      case "branch":
        if (!branch) return null;
        // A sessão tem repositório, mas o TYBA não acompanha a branch dele: o
        // chip fica no lugar dizendo isso. Sumir era o que fazia uma sessão de
        // agente encerrada parecer defeito da barra.
        if (branch.state === "unknown") {
          return (
            <span
              key={id}
              title={t("toolbarBranchUnknownHint")}
              className="flex min-w-0 shrink items-center gap-1 text-tyba-text-muted"
              aria-label={t("toolbarBranchUnknown")}
            >
              <GitBranch size={12} className="shrink-0" />
              {/* Travessão em vez de palavra: o ícone já diz "branch", e o que
                  falta dizer é só que o valor não está disponível. */}
              <span className="truncate">—</span>
            </span>
          );
        }
        // Com uma sessão que o core sabe localizar, o chip troca de branch; sem
        // ela, continua sendo o texto que sempre foi. O seletor faz o checkout
        // em dois passos — armar e confirmar —, que é a rede para uma ação que
        // mexe no working tree.
        // Azul, a cor do git (mockup Proposta): o chip colorido da barra.
        return branch.sessionId ? (
          <span key={id} className="flex min-w-0 shrink text-tyba-blue">
            <BranchPicker
              sessionId={branch.sessionId}
              browsing={null}
              label={branch.label}
              onBrowse={() => {}}
              variant="chip"
              browsable={false}
            />
          </span>
        ) : (
          <span
            key={id}
            title={branch.label}
            className="flex min-w-0 shrink items-center gap-1 text-tyba-blue"
            aria-label={t("toolbarBranch")}
          >
            <span className="truncate">{branch.label}</span>
          </span>
        );
      case "diffCount": {
        // O servidor responde quantos arquivos mudaram, e só. Sem botão: o
        // diff local abriria o repositório da máquina de cá.
        if (remoteChanged != null) {
          return (
            <span
              key={id}
              title={t("toolbarRemoteDiff")}
              aria-label={t("toolbarRemoteDiff")}
              className="flex items-center gap-1 font-mono"
            >
              <GitDiff size={12} className="shrink-0" />
              {remoteChanged}
            </span>
          );
        }
        const status = snapshot?.status;
        if (!status?.dirty) return null;
        return (
          <button
            key={id}
            onClick={onOpenDiff}
            title={t("gitStatusIconLabel")}
            aria-label={t("toolbarDiff")}
            className="rounded-[4px] px-1 py-0.5 font-mono transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
          >
            {t("toolbarChanges", { count: status.changed })}{" "}
            <span className="text-tyba-green">+{status.insertions}</span>{" "}
            <span className="text-tyba-red">−{status.deletions}</span>
          </button>
        );
      }
      case "reviewDiff":
        return hasWorktree ? (
          <button
            key={id}
            onClick={onOpenDiff}
            title={t("toolbarReviewDiff")}
            aria-label={t("toolbarReviewDiff")}
            className="flex items-center gap-1 rounded-[4px] px-1 py-0.5 text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
          >
            <GitDiff size={12} />
            <span>diff</span>
          </button>
        ) : null;
      case "aheadBehind": {
        const ahead = snapshot?.ahead ?? 0;
        const behind = snapshot?.behind ?? 0;
        if (snapshot?.ahead == null || (ahead === 0 && behind === 0)) {
          return null;
        }
        return (
          <span
            key={id}
            className="flex items-center gap-0.5 font-mono"
            aria-label={t("toolbarAheadBehind")}
          >
            {ahead > 0 && (
              <>
                <ArrowUp size={11} />
                {ahead}
              </>
            )}
            {behind > 0 && (
              <>
                <ArrowDown size={11} />
                {behind}
              </>
            )}
          </span>
        );
      }
      case "clock":
        return keeps(FOOTER_STEP.clock) ? <ClockChip key={id} /> : null;
    }
  };

  const left = pref.left.map(chip).filter(Boolean);
  const right = pref.right.map(chip).filter(Boolean);
  if (
    !showRichInput &&
    !suggestPlugin &&
    left.length === 0 &&
    right.length === 0
  ) {
    return null;
  }

  return (
    // Opção G: o rodapé é cromo — o mesmo acrílico da barra e da lateral, com
    // o fio em cima. Transparente, ele mostrava a aurora crua por trás.
    // A escada mede a largura natural do grupo da esquerda (o `w-max` de
    // dentro); o de fora só recorta se nem o último degrau couber.
    <div
      ref={rootRef}
      className="tyba-section flex h-[26px] min-w-0 shrink-0 items-center justify-between gap-4 px-3.5 text-[11px] text-tyba-text-faint shadow-[inset_0_1px_0_var(--tyba-line)]"
    >
      <div className="flex min-w-0 overflow-hidden">
        <div ref={leftRef} className="flex w-max items-center gap-4">
          {showRichInput && (
            <button
              onClick={onOpenRichInput}
              title={t("richInputHint")}
              aria-label={t("richInputToggle")}
              className="flex shrink-0 items-center gap-1.5 rounded-[4px] px-1.5 py-0.5 text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
            >
              <TextAlignLeft size={12} />
              {keeps(FOOTER_STEP.richInputLabel) && <span>{t("richInputToggle")}</span>}
              <Shortcut combo={richInputCombo} />
            </button>
          )}
          {suggestPlugin && <PluginChip sessionId={sessionId} />}
          {hints.map((hint) => {
            const at = HINT_LABEL_STEP[hint.key];
            const labeled = at === undefined || keeps(at);
            return (
              <button
                key={hint.key}
                onClick={hint.onClick}
                aria-label={hint.label}
                title={labeled ? undefined : hint.label}
                className="flex shrink-0 items-center gap-1.5 rounded-[4px] px-1 py-0.5 text-tyba-text-faint transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
              >
                {hint.combo ? <Shortcut combo={hint.combo} /> : hint.icon}
                {labeled && <span>{hint.label}</span>}
              </button>
            );
          })}
          {left}
        </div>
      </div>
      <div ref={rightRef} className="flex shrink-0 items-center gap-4">
        {right}
      </div>
    </div>
  );
}

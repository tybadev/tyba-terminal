import { useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowsLeftRight,
  CaretDown,
  FolderSimple,
  ShieldCheck,
  Stop,
  Warning,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  EDGE_KINDS,
  OUTPUT_KINDS,
  integrationProblem,
  type EdgeKind,
  type Integration,
  type OutputKind,
  type Trigger,
} from "@/lib/maestro";

const label = "text-[10px] tracking-[0.08em] text-tyba-text-faint uppercase";
const field =
  "w-full rounded-[6px] border border-tyba-border-strong bg-tyba-sunken px-2.5 py-[7px] font-mono text-[12.5px] text-tyba-text outline-none focus:border-tyba-text-faint";

const triggerValue = (trigger: Trigger) =>
  trigger.type === "manual" ? "manual" : `onInput:${trigger.kind}`;

const parseTrigger = (value: string): Trigger =>
  value === "manual" ? { type: "manual" } : { type: "onInput", kind: value.slice(8) as EdgeKind };

/**
 * Editor de uma integração (quadro MaestroIntegracao): nome, gatilho, script,
 * tipo de saída, a caixa de segurança e a prévia do nó. O core valida no
 * Salvar; o aviso daqui só antecipa o que ele recusaria.
 */
export function IntegrationEditor({
  draft,
  onChange,
  saving,
  dirty,
  testNodeTitle,
  error,
  onSave,
  onTest,
}: {
  draft: Integration;
  onChange: (draft: Integration) => void;
  saving: boolean;
  dirty: boolean;
  /** Nó do canvas cujo `in/` o "Testar" usa; `null` se a integração não está no canvas. */
  testNodeTitle: string | null;
  error: string | null;
  onSave: () => void;
  onTest: () => void;
}) {
  const { t } = useTranslation();
  const gutter = useRef<HTMLDivElement>(null);
  const problem = integrationProblem(draft);
  const lines = Math.max(1, draft.script.split("\n").length);
  const input = draft.trigger.type === "manual" ? null : draft.trigger.kind;

  return (
    <form
      aria-label={t("maestroIntEdit", { name: draft.name })}
      onSubmit={(e) => {
        e.preventDefault();
        if (!problem && !saving) onSave();
      }}
      className="flex min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-[18px] py-4 font-mono"
    >
      <div className="flex flex-col gap-1.5">
        <label className={label} htmlFor="maestro-int-name">
          {t("maestroIntName")}
        </label>
        <input
          id="maestro-int-name"
          type="text"
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={draft.name}
          onChange={(e) => onChange({ ...draft, name: e.target.value })}
          className={field}
        />
      </div>

      <div className="flex flex-col gap-1.5">
        <label className={label} htmlFor="maestro-int-trigger">
          {t("maestroIntTrigger")}
        </label>
        <div className="relative">
          <select
            id="maestro-int-trigger"
            value={triggerValue(draft.trigger)}
            onChange={(e) => onChange({ ...draft, trigger: parseTrigger(e.target.value) })}
            className={`${field} appearance-none pr-8`}
          >
            {EDGE_KINDS.map((kind) => (
              <option key={kind} value={`onInput:${kind}`}>
                {t("maestroIntOnInput", { kind: t(`maestroEdge_${kind}`) })}
              </option>
            ))}
            <option value="manual">{t("maestroIntManual")}</option>
          </select>
          <CaretDown
            size={12}
            className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 text-tyba-text-faint"
          />
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="flex items-baseline justify-between gap-2">
          <label className={label} htmlFor="maestro-int-script">
            {t("maestroIntScript")}
          </label>
          <span className="truncate text-[10.5px] text-tyba-text-faint">
            {t("maestroIntRunsIn")}
          </span>
        </span>
        <div className="flex min-h-[150px] overflow-hidden rounded-[6px] border border-tyba-border-strong bg-tyba-sunken focus-within:border-tyba-text-faint">
          <div
            ref={gutter}
            aria-hidden
            className="w-7 shrink-0 overflow-hidden py-2 pr-2.5 text-right text-[11px] leading-[18px] text-tyba-text-ghost select-none"
          >
            {Array.from({ length: lines }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>
          <textarea
            id="maestro-int-script"
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            wrap="off"
            rows={Math.min(Math.max(lines, 6), 16)}
            placeholder={"# $TYBA_IN  = entradas\n# $TYBA_OUT = saída deste nó"}
            value={draft.script}
            onChange={(e) => onChange({ ...draft, script: e.target.value })}
            onScroll={(e) => {
              if (gutter.current) gutter.current.scrollTop = e.currentTarget.scrollTop;
            }}
            className="min-w-0 flex-1 resize-y bg-transparent py-2 pr-2 text-[11px] leading-[18px] text-tyba-text-muted outline-none placeholder:text-tyba-text-ghost"
          />
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className={label} id="maestro-int-out">
          {t("maestroIntOutput")}
        </span>
        <div className="flex flex-wrap items-center gap-2.5">
          <div
            role="radiogroup"
            aria-labelledby="maestro-int-out"
            className="flex w-max overflow-hidden rounded-[6px] border border-tyba-border-strong"
          >
            {OUTPUT_KINDS.map((kind) => (
              <label
                key={kind}
                className="relative cursor-pointer border-r border-tyba-border px-3 py-[5px] text-[11.5px] text-tyba-text-muted last:border-r-0 has-[:checked]:bg-tyba-blue-tint has-[:checked]:text-tyba-text has-[:focus-visible]:outline-2 has-[:focus-visible]:-outline-offset-2 has-[:focus-visible]:outline-tyba-cyan"
              >
                <input
                  type="radio"
                  name="maestro-int-out"
                  value={kind}
                  checked={draft.output === kind}
                  onChange={() =>
                    onChange({
                      ...draft,
                      output: kind as OutputKind,
                      file: kind === "file" ? draft.file : null,
                    })
                  }
                  className="absolute m-0 size-px opacity-0"
                />
                {t(`maestroEdge_${kind}`)}
              </label>
            ))}
          </div>
          {draft.output === "file" && (
            <input
              aria-label={t("maestroOutputFile")}
              placeholder="result.json"
              type="text"
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              value={draft.file ?? ""}
              onChange={(e) => onChange({ ...draft, file: e.target.value || null })}
              className={`${field} w-40 py-[5px] text-[11.5px]`}
            />
          )}
        </div>
      </div>

      <div
        role="group"
        aria-label={t("maestroIntSecurity")}
        className="flex flex-col gap-[7px] rounded-[8px] border border-tyba-cyan/30 bg-tyba-cyan-tint px-3 py-[11px] text-[11.5px] text-tyba-text-muted"
      >
        <span className="text-tyba-cyan">{t("maestroIntSecurity")}</span>
        <span className="flex items-center gap-2.5">
          <FolderSimple size={13} className="shrink-0 text-tyba-cyan" />
          {t("maestroIntSecRunsAs")}
        </span>
        <span className="flex items-center gap-2.5">
          <ArrowsLeftRight size={13} className="shrink-0 text-tyba-cyan" />
          <span>
            {t("maestroIntSecIn")} <span className="text-tyba-text">$TYBA_IN</span> ·{" "}
            {t("maestroIntSecOut")} <span className="text-tyba-text">$TYBA_OUT</span>
          </span>
        </span>
        <span className="flex items-center gap-2.5">
          <ShieldCheck size={13} className="shrink-0 text-tyba-cyan" />
          <span>
            {t("maestroIntSecGuard")}{" "}
            <span className="text-tyba-text">{t("maestroIntSecGuardRule")}</span>
          </span>
        </span>
        <span className="flex items-center gap-2.5">
          <Stop size={13} className="shrink-0 text-tyba-cyan" />
          {t("maestroIntSecStop")}
        </span>
        <span className="text-tyba-text-faint">{t("maestroIntSecFixed")}</span>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className={label}>{t("maestroIntOnCanvas")}</span>
        <div
          aria-label={t("maestroIntPreview", {
            input: input ? t(`maestroEdge_${input}`) : t("maestroIntAnyInput"),
            output: t(`maestroEdge_${draft.output}`),
          })}
          className="flex items-center text-[11px] text-tyba-text-muted"
        >
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
            <i className="size-[9px] shrink-0 rounded-full border-2 border-tyba-text-faint" />
            {t("maestroInput")}: {input ? t(`maestroEdge_${input}`) : t("maestroIntAnyInput")}
          </span>
          <span className="mx-2 h-0 min-w-3.5 flex-1 border-t-[1.5px] border-dashed border-tyba-text-faint" />
          <span className="max-w-[160px] truncate rounded-[7px] border border-tyba-border-strong bg-tyba-raised px-2.5 py-[5px] text-tyba-text">
            {draft.name || t("maestroKindIntegration")}
          </span>
          <span className="mx-2 h-0 min-w-3.5 flex-1 border-t-[1.5px] border-dashed border-tyba-text-faint" />
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap">
            {t("maestroOutput")}: {t(`maestroEdge_${draft.output}`)}
            <i className="size-[9px] shrink-0 rounded-full bg-tyba-text-muted" />
          </span>
        </div>
      </div>

      {(problem || error) && (
        <p
          role="alert"
          className="flex items-start gap-2 rounded-[6px] border border-tyba-red/50 bg-tyba-red-tint px-3 py-2 text-[11.5px] text-tyba-red"
        >
          <Warning size={13} className="mt-px shrink-0" />
          {problem ? t(`maestroIntProblem_${problem}`) : error}
        </p>
      )}

      <div className="mt-auto flex items-center gap-2 pt-1">
        <button
          type="button"
          disabled={!testNodeTitle || !!problem || saving}
          onClick={onTest}
          className="inline-flex items-center gap-[7px] rounded-[6px] border border-tyba-border-strong px-3 py-[7px] text-[12px] whitespace-nowrap text-tyba-text hover:border-tyba-text-muted disabled:cursor-not-allowed disabled:text-tyba-text-ghost disabled:hover:border-tyba-border-strong"
        >
          {t("maestroIntTest")}
        </button>
        <span className="flex-1" />
        <Button type="submit" size="sm" disabled={!!problem || saving || !dirty}>
          {t("maestroIntSave")}
        </Button>
      </div>
      <span className="text-[10.5px] text-tyba-text-faint">
        {testNodeTitle
          ? t("maestroIntTestWith", { node: testNodeTitle })
          : t("maestroIntTestNeedsNode")}
      </span>
    </form>
  );
}

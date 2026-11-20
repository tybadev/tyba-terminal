import { LockSimple } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented";

/**
 * As peças das Configurações, no desenho do quadro "Configurações (aprovada)":
 * grupo = título curto em caixa normal, sem caixa em volta; linha = nome e
 * explicação à esquerda, controle sempre à vista à direita, divisória fina
 * embaixo (a última do grupo não tem).
 */

export function SettingGroup({
  title,
  note,
  children,
}: {
  title: string;
  note?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    // Quadro: 36px entre grupos, 32px no primeiro (logo abaixo do título ou do seletor de tema).
    <section className="mt-9 first-of-type:mt-8">
      <div className="mb-1 flex items-baseline gap-3">
        <h2 className="m-0 text-[15px] leading-[22px] font-bold tracking-[-.01em] text-tyba-text">
          {title}
        </h2>
        {note && <span className="ml-auto shrink-0 text-[12.5px] text-tyba-text-faint">{note}</span>}
      </div>
      {children !== undefined && <div className="[&>*:last-child]:border-b-0">{children}</div>}
    </section>
  );
}

export function SettingLine({
  label,
  hint,
  children,
  below,
}: {
  label?: React.ReactNode;
  hint?: React.ReactNode;
  children?: React.ReactNode;
  /** Controle largo demais para a coluna da direita: vai embaixo, na largura toda. */
  below?: React.ReactNode;
}) {
  return (
    <div
      data-setting-row
      className="flex flex-wrap items-center gap-x-8 gap-y-3 border-b border-[var(--tyba-line)] py-[13px]"
    >
      <div className="flex min-w-[240px] flex-1 basis-[240px] flex-col gap-0.5">
        {label && <span className="text-[14px] leading-5 font-semibold text-tyba-text">{label}</span>}
        {hint && (
          <span className="max-w-[420px] break-words text-[13px] leading-[19px] text-tyba-text-muted">
            {hint}
          </span>
        )}
      </div>
      {children && <div className="ml-auto flex shrink-0 items-center justify-end gap-2">{children}</div>}
      {below && <div className="w-full">{below}</div>}
    </div>
  );
}

/** Texto solto dentro de um grupo, na mesma margem das linhas. */
export function SettingNote({
  children,
  tone = "faint",
}: {
  children: React.ReactNode;
  tone?: "faint" | "muted" | "amber";
}) {
  const color =
    tone === "amber" ? "text-tyba-amber" : tone === "muted" ? "text-tyba-text-muted" : "text-tyba-text-faint";
  return (
    <div className={`border-b border-[var(--tyba-line)] py-2.5 text-[12.5px] leading-[18px] ${color}`}>
      {children}
    </div>
  );
}

/** "● Conectado" ao lado de um controle: o estado de agora, não a escolha. */
export function SettingStatus({ tone, children }: { tone: "green" | "faint" | "amber" | "red"; children: React.ReactNode }) {
  const color = {
    green: "text-tyba-green",
    faint: "text-tyba-text-faint",
    amber: "text-tyba-amber",
    red: "text-tyba-red",
  }[tone];
  return (
    <span className={`inline-flex items-center gap-[7px] text-[13px] font-semibold ${color}`}>
      <i className="block size-2 rounded-full bg-current" aria-hidden />
      {children}
    </span>
  );
}

/** Regra que nenhuma configuração desliga: cadeado + "Fixo", sem controle. */
export function SettingFixed({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-tyba-text-faint">
      <LockSimple size={14} />
      {children}
    </span>
  );
}

/** Controle segmentado do quadro: o `SegmentedControl` do Muriki. */
export function SettingSegment<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { value: T; label: React.ReactNode }[];
  value: T | null;
  onChange: (value: T) => void;
}) {
  return (
    <SegmentedControl<T>
      aria-label={label}
      options={options}
      // Sem escolha ainda (preferência lendo), nenhuma opção acende.
      value={value as T}
      onChange={onChange}
    />
  );
}

const BUTTON_TONE = {
  glass: "outline",
  ghost: "ghost",
  danger: "destructive",
} as const;

/** Botão pequeno do quadro (glass, ghost ou danger), com ícone opcional: o
 *  `Button` sm do Muriki. */
export function SettingButton({
  tone = "glass",
  icon,
  children,
  ...props
}: React.ComponentProps<"button"> & {
  tone?: keyof typeof BUTTON_TONE;
  icon?: React.ReactNode;
}) {
  return (
    <Button type="button" size="sm" variant={BUTTON_TONE[tone]} {...props}>
      {icon}
      {children}
    </Button>
  );
}

/** Campo de texto e select do quadro: o `Input`/`Select` do Muriki com
 *  230px. No claro do TYBA é o campo de 40px; nos outros temas o fallback
 *  mantém o campo de 36px com o fio da linha de antes. */
export const SETTING_FIELD =
  "h-[var(--tyba-mk-field-h,2.25rem)] w-[230px] rounded-[10px] border-[var(--tyba-mk-glass-edge,transparent)] bg-[var(--tyba-mk-glass-strong,var(--tyba-glass-piece))] shadow-[0_0_0_1px_var(--tyba-line)] hover:border-[var(--tyba-mk-glass-edge,transparent)]";

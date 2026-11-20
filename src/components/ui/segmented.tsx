import * as React from "react";

import { cn } from "@/lib/utils";

export interface SegmentOption<T extends string> {
  value: T;
  label: React.ReactNode;
  icon?: React.ReactNode;
  disabled?: boolean;
}

// Segmentado do Muriki (`SegmentedControl`, `mk-glass mk-seg`): trilho de
// vidro em pílula com a aresta branca e 4px de folga, opções de 30px em
// 13px/600 e o knob branco que desliza para a opção acesa com a mola de
// 320ms. O vidro, a aresta e a mola são do claro do TYBA (`--tyba-mk-*`);
// nos outros temas o trilho fica na tinta de 6%, sem aresta, e a mola em
// 300ms. O knob é a placa (`.tyba-pill-on`), então cada tema ganha a sua.
// `tone="tint"`: o trilho de tinta das telas de Git e Docker (o quadro tira
// o vidro dele ali), com a mesma medida.
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  className,
  tone = "glass",
  "aria-label": ariaLabel,
}: {
  tone?: "glass" | "tint";
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
  className?: string;
  "aria-label"?: string;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const [knob, setKnob] = React.useState<{ x: number; w: number } | null>(null);
  const idx = options.findIndex((o) => o.value === value);
  // A cada render, e não só quando muda a opção: rótulo que cresce (a
  // contagem que chega depois) muda a largura sem mudar o índice.
  React.useLayoutEffect(() => {
    const el = refs.current[idx];
    const next = el ? { x: el.offsetLeft, w: el.offsetWidth } : null;
    setKnob((prev) =>
      prev?.x === next?.x && prev?.w === next?.w ? prev : next,
    );
  });
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      className={cn(
        "relative inline-flex shrink-0 rounded-full border-[length:var(--tyba-mk-edge-w,0px)] bg-tyba-text/[.06] p-1",
        tone === "glass"
          ? "tyba-mk-glass border-[var(--tyba-mk-glass-edge,transparent)]"
          : "border-transparent",
        className,
      )}
    >
      {knob && (
        <span
          aria-hidden
          className="tyba-pill-on absolute top-1 bottom-1 left-0 rounded-full transition-[transform,width] duration-[var(--tyba-mk-spring,300ms)] ease-[cubic-bezier(.3,1.4,.5,1)] motion-reduce:transition-none"
          style={{ width: knob.w, transform: `translateX(${knob.x}px)` }}
        />
      )}
      {options.map((o, i) => (
        <button
          key={o.value}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={cn(
            "relative z-[1] inline-flex h-[30px] items-center gap-2 rounded-full px-4 text-[13px] font-semibold whitespace-nowrap transition-colors duration-200 outline-none focus-visible:outline-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-4 [&_svg]:shrink-0",
            o.value === value ? "text-tyba-text" : "text-tyba-text-muted hover:text-tyba-text",
          )}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}

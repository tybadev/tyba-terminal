import * as React from "react";

import { cn } from "@/lib/utils";

export interface SegmentOption<T extends string> {
  value: T;
  label: React.ReactNode;
  icon?: React.ReactNode;
  disabled?: boolean;
}

// Segmentado do Muriki (`SegmentedControl`): trilho em pílula com 4px de
// folga, opções de 30px em 13px/600 e o knob branco que desliza para a
// opção acesa com a mola de 320ms. O knob é a placa (`.tyba-pill-on`), então
// os outros temas ganham a deles.
export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  className,
  "aria-label": ariaLabel,
}: {
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
  className?: string;
  "aria-label"?: string;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const [knob, setKnob] = React.useState<{ x: number; w: number } | null>(null);
  const idx = options.findIndex((o) => o.value === value);
  React.useLayoutEffect(() => {
    const el = refs.current[idx];
    setKnob(el ? { x: el.offsetLeft, w: el.offsetWidth } : null);
  }, [idx, options.length]);
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      className={cn(
        "relative inline-flex shrink-0 rounded-full bg-tyba-text/[.06] p-1",
        className,
      )}
    >
      {knob && (
        <span
          aria-hidden
          className="tyba-pill-on absolute top-1 bottom-1 left-0 rounded-full transition-[transform,width] duration-300 ease-[cubic-bezier(.3,1.4,.5,1)] motion-reduce:transition-none"
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

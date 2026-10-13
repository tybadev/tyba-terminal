import { useId } from "react";

// Desenho copiado da marca canônica do design system
// (ds-bundle/assets/tyba-mark.svg). SVG inline, não <img>: como arquivo, o
// Vite inlinava como data URI (bloqueado pela CSP em `img-src`) e o
// `?no-inline` dependia da URL do servidor de dev chegar ao webview — no app
// em dev a imagem aparecia quebrada. Inline não depende de URL nem de CSP.
// Os ids dos gradientes levam o `useId` para não colidir entre instâncias.

/** A marca em cor de marca — os gradientes são constantes da marca, não tokens. */
export function TybaMark({ size = 16 }: { size?: number }) {
  const id = useId().replace(/:/g, "");
  const g = (name: string) => `tyba-${name}-${id}`;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      aria-hidden="true"
      className="shrink-0 select-none"
    >
      <defs>
        <linearGradient id={g("arc-top")} x1="15.3" y1="4.5" x2="48.7" y2="4.5" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#7dc242" />
          <stop offset="1" stopColor="#f2bb0f" />
        </linearGradient>
        <linearGradient id={g("arc-right")} x1="58.1" y1="12.3" x2="58.1" y2="46.8" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#f8bb10" />
          <stop offset="1" stopColor="#6c9d1a" />
        </linearGradient>
        <linearGradient id={g("arc-left")} x1="5.8" y1="12.3" x2="5.8" y2="46.8" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#1fab4e" />
          <stop offset="1" stopColor="#008d84" />
        </linearGradient>
        <linearGradient id={g("wave-hi")} x1="11.8" y1="53.6" x2="52.2" y2="53.6" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#0179d4" />
          <stop offset="0.5" stopColor="#00a2c4" />
          <stop offset="1" stopColor="#0179d4" />
        </linearGradient>
        <linearGradient id={g("wave-lo")} x1="18.7" y1="59.6" x2="45.3" y2="59.6" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#04408e" />
          <stop offset="0.5" stopColor="#0658a8" />
          <stop offset="1" stopColor="#04408e" />
        </linearGradient>
        <linearGradient id={g("core")} x1="32" y1="25.4" x2="32" y2="33.4" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#c3d95f" />
          <stop offset="1" stopColor="#46a175" />
        </linearGradient>
      </defs>
      <g strokeWidth={3.8} strokeLinecap="round" fill="none">
        <path d="M15.3 4.5 Q32 31.6 48.7 4.5" stroke={`url(#${g("arc-top")})`} />
        <path d="M58.1 12.3 Q36.9 29.3 58.1 46.8" stroke={`url(#${g("arc-right")})`} />
        <path d="M5.8 12.3 Q27.1 29.3 5.8 46.8" stroke={`url(#${g("arc-left")})`} />
        <path d="M11.8 53.6 Q32 25.8 52.2 53.6" stroke={`url(#${g("wave-hi")})`} />
        <path d="M18.7 59.6 Q32 38.7 45.3 59.6" stroke={`url(#${g("wave-lo")})`} />
      </g>
      <circle cx="32" cy="29.4" r="4.05" fill={`url(#${g("core")})`} />
    </svg>
  );
}

/**
 * A marca de uma cor só, na cor do texto em volta (`currentColor`).
 *
 * Versão reduzida para tamanhos pequenos: os quatro rios e o ponto, sem a
 * segunda onda de baixo, traço mais grosso — em 16px a onda dupla vira borrão.
 */
export function TybaMarkMono({
  size = 16,
  className,
}: {
  size?: number;
  className?: string;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 64 64"
      fill="none"
      aria-hidden="true"
      className={className}
    >
      <g
        strokeWidth={4.5}
        strokeLinecap="round"
        stroke="currentColor"
        fill="none"
      >
        <path d="M15.3 4.5 Q32 31.6 48.7 4.5" />
        <path d="M58.1 12.3 Q36.9 29.3 58.1 46.8" />
        <path d="M5.8 12.3 Q27.1 29.3 5.8 46.8" />
        <path d="M11.8 53.6 Q32 25.8 52.2 53.6" />
      </g>
      <circle cx="32" cy="29.4" r="5" fill="currentColor" />
    </svg>
  );
}

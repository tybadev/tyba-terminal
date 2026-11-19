/** Ícone de sessão de shell (dono, 2026-11-15): o `Terminal` do Phosphor
 *  (>_) dentro de um círculo, no desenho de `ds/terminal-shell-icon.svg`. */
export function ShellGlyph({
  size = 16,
  className,
  style,
}: {
  size?: number;
  className?: string;
  style?: React.CSSProperties;
}) {
  return (
    <svg
      viewBox="0 0 256 256"
      width={size}
      height={size}
      aria-hidden
      className={`shrink-0 ${className ?? ""}`}
      style={style}
    >
      <circle cx="128" cy="128" r="114" fill="none" stroke="currentColor" strokeWidth="16" />
      <path
        fill="currentColor"
        transform="translate(128 128) scale(0.52) translate(-125 -129)"
        d="M117.31,134l-72,64a8,8,0,1,1-10.63-12L100,128,34.69,70A8,8,0,1,1,45.32,58l72,64a8,8,0,0,1,0,12ZM216,184H120a8,8,0,0,0,0,16h96a8,8,0,0,0,0-16Z"
      />
    </svg>
  );
}

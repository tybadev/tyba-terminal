interface Props {
  size?: number;
  className?: string;
}

/** O círculo do Orbit (dono, 2026-11-02: "só um círculo, não vários"), no
 *  traço dos ícones da lateral. */
export function OrbitIcon({ size = 16, className }: Props) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      className={className}
      aria-hidden
    >
      <circle cx="8" cy="8" r="5.6" />
    </svg>
  );
}

interface Props {
  size?: number;
  className?: string;
  style?: React.CSSProperties;
}

export function AgentIcon({ size = 16, className, style }: Props) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.35}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={style}
    >
      <path d="M10.887 15H8.5a.5.5 0 0 1 0-1h2.387a1 1 0 0 0 .865-.5l2.888-5a1 1 0 0 0 0-1l-2.935-5.084A.84.84 0 0 0 10.983 2a.83.83 0 0 0-.795.587l-3.422 11.12A1.82 1.82 0 0 1 5.016 15a1.84 1.84 0 0 1-1.587-.916L.495 9a2 2 0 0 1 0-2l2.887-5c.355-.617 1.019-1 1.731-1H7.5a.5.5 0 0 1 0 1H5.113a1 1 0 0 0-.865.5l-2.888 5a1 1 0 0 0 0 1l2.935 5.084a.84.84 0 0 0 .722.416a.83.83 0 0 0 .795-.588L9.234 2.293A1.82 1.82 0 0 1 10.984 1c.652 0 1.261.351 1.587.916L15.506 7a2 2 0 0 1 0 2.001L12.619 14c-.355.617-1.02 1-1.732 1" />
    </svg>
  );
}

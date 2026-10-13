import { OpenaigymIcon } from "./simple-icons-openaigym";

interface Props {
  size?: number;
  className?: string;
  style?: React.CSSProperties;
}

/** O glifo genérico de AGENTE — um só, em todo lugar que o conceito aparece.
 *  O path é do Simple Icons e foi desenhado para ser PREENCHIDO: com traço ele
 *  vira um borrão em 11–16px. Logos de runner (Claude, Codex) não passam aqui. */
export function AgentIcon({ size = 16, className, style }: Props) {
  return (
    <OpenaigymIcon
      size={size}
      fill="currentColor"
      stroke="none"
      className={className}
      style={style}
    />
  );
}

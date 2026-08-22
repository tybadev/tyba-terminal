import { useTranslation } from "react-i18next";

import { nextPromptMode } from "../lib/promptModePicker";

interface Props {
  value: boolean;
  onChange: (next: boolean) => void;
}

/**
 * A mini-tela de cada cartão.
 *
 * Desenho estático, não um terminal de verdade: montar PTY dentro do painel de
 * configuração seria custo sem retorno, e o que precisa ser respondido aqui é
 * "onde eu digito?" — que se responde com a forma.
 *
 * Ela usa a MESMA gramática da tela real, porque é disso que a comparação vive:
 * chevron verde à esquerda, mono, e a linha de digitar separada da saída por um
 * filete. No modo clássico não há filete nenhum, porque não há faixa: o prompt
 * do shell é só mais uma linha da grade.
 */
function MiniScreen({ tybaLine }: { tybaLine: boolean }) {
  const { t } = useTranslation();
  return (
    <div
      aria-hidden
      className="mt-2 overflow-hidden rounded-[4px] border border-tyba-border bg-tyba-bg font-mono text-[9px] leading-[14px]"
    >
      {tybaLine ? (
        <>
          <div className="px-1.5 pt-1.5">
            <span className="text-tyba-green">❯</span>{" "}
            <span className="text-tyba-text">ls</span>
          </div>
          <div className="px-1.5 pb-1.5 text-tyba-text-muted">arquivo.ts</div>
          {/* O filete é a peça que diz "aqui a leitura acaba e a escrita
              começa". É ele, e não a cor, que distingue os dois modos. */}
          <div className="border-t border-tyba-border bg-tyba-sunken px-1.5 py-1.5">
            <span className="text-tyba-green">❯</span>{" "}
            <span className="text-tyba-text-faint">
              {t("promptModeCardTypeHere")}
            </span>
            <span className="ml-0.5 inline-block h-[9px] w-[4px] translate-y-[1px] bg-tyba-green/70" />
          </div>
        </>
      ) : (
        <div className="px-1.5 py-1.5">
          <div>
            <span className="text-tyba-text-muted">~/proj $</span>{" "}
            <span className="text-tyba-text">ls</span>
          </div>
          <div className="text-tyba-text-muted">arquivo.ts</div>
          <div>
            <span className="text-tyba-text-muted">~/proj $</span>
            <span className="ml-0.5 inline-block h-[9px] w-[4px] translate-y-[1px] bg-tyba-text/60" />
          </div>
          {/* Uma linha em branco no lugar da faixa: sem ela os dois cartões
              teriam alturas diferentes e a comparação viraria "um é maior". */}
          <div>&nbsp;</div>
        </div>
      )}
    </div>
  );
}

/**
 * Escolher entre a linha do TYBA e o prompt do shell, vendo o que cada uma faz.
 *
 * Era um `Switch` com um parágrafo de prosa. Quem nunca viu os dois modos não
 * tinha como saber o que estava escolhendo antes de escolher — e a escolha muda
 * onde o cursor mora, que é a coisa mais básica de um terminal.
 *
 * Dois cartões, e não um switch com prévia única: assim as duas mini-telas
 * ficam visíveis ao MESMO tempo. Com uma prévia só, comparar exige alternar o
 * controle e guardar a primeira de memória.
 */
export function PromptModePicker({ value, onChange }: Props) {
  const { t } = useTranslation();

  const options: Array<{ tybaLine: boolean; title: string; hint: string }> = [
    {
      tybaLine: true,
      title: t("promptModeCardTybaTitle"),
      hint: t("promptModeCardTybaHint"),
    },
    {
      tybaLine: false,
      title: t("promptModeCardShellTitle"),
      hint: t("promptModeCardShellHint"),
    },
  ];

  return (
    <div
      role="radiogroup"
      aria-label={t("promptModeTitle")}
      className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2"
    >
      {options.map((option) => {
        const selected = value === option.tybaLine;
        return (
          <button
            key={option.title}
            type="button"
            role="radio"
            aria-checked={selected}
            // Roving tabindex: o grupo inteiro é UMA parada de Tab, e dentro
            // dele quem anda é a seta. Dois botões tabuláveis fariam o seletor
            // custar duas paradas e ainda assim não responder às setas.
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option.tybaLine)}
            onKeyDown={(event) => {
              const next = nextPromptMode(value, event.key);
              if (next === null) return;
              event.preventDefault();
              onChange(next);
            }}
            className={`rounded-[6px] border p-3 text-left transition-colors ${
              selected
                ? "border-tyba-green/60 bg-tyba-green/[.06]"
                : "border-tyba-border hover:border-tyba-border-strong"
            }`}
          >
            <span className="flex items-center gap-2">
              <span
                aria-hidden
                className={`grid h-3 w-3 shrink-0 place-items-center rounded-full border ${
                  selected ? "border-tyba-green" : "border-tyba-text-faint"
                }`}
              >
                {selected && (
                  <span className="h-1.5 w-1.5 rounded-full bg-tyba-green" />
                )}
              </span>
              <span className="text-[13px] text-tyba-text">{option.title}</span>
            </span>
            <MiniScreen tybaLine={option.tybaLine} />
            <span className="mt-2 block text-[11px] leading-relaxed text-tyba-text-faint">
              {option.hint}
            </span>
          </button>
        );
      })}
    </div>
  );
}

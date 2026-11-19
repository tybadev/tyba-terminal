import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import blackout from "../../src-tauri/icons/app/thumb/blackout.png";
import aurora from "../../src-tauri/icons/app/thumb/aurora.png";
import filamento from "../../src-tauri/icons/app/thumb/filamento.png";
import assinatura from "../../src-tauri/icons/app/thumb/assinatura.png";
import fosforo from "../../src-tauri/icons/app/thumb/fosforo.png";
import violeta from "../../src-tauri/icons/app/thumb/violeta.png";
import grafite from "../../src-tauri/icons/app/thumb/grafite.png";
import papel from "../../src-tauri/icons/app/thumb/papel.png";

import {
  APP_ICONS,
  APP_ICON_NAMES,
  isAppIconId,
  radioStep,
  type AppIconId,
} from "@/lib/appIcon";
import { getAppIcon, onAppIconChanged, setAppIcon } from "@/lib/ipc";
import { IS_MAC } from "@/lib/platform";
import { SettingLine } from "./settings/rows";

const THUMBS: Record<AppIconId, string> = {
  blackout,
  aurora,
  filamento,
  assinatura,
  fosforo,
  violeta,
  grafite,
  papel,
};

/**
 * "Ícone no Dock" (quadro "Configurações (aprovada)"): uma linha com o nome
 * do escolhido e os ícones pequenos à direita, como grupo de rádios. Quem decide e aplica é o core; a seleção
 * só muda quando ele confirma (resposta do comando ou o evento, que também
 * acompanha a escolha feita em outra janela).
 */
export function AppIconPicker() {
  const { t } = useTranslation();
  const [current, setCurrent] = useState<AppIconId | null>(null);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  // Último pedido ainda sem confirmação. `choose` compara com ele, não com o
  // confirmado: quem volta atrás antes de o core responder precisa que a
  // volta também seja enviada. Toda confirmação do core zera.
  const requested = useRef<AppIconId | null>(null);

  useEffect(() => {
    const accept = (id: string) => {
      if (!isAppIconId(id)) return;
      requested.current = null;
      setCurrent(id);
    };
    void getAppIcon().then(accept).catch(() => {});
    const off = onAppIconChanged(accept);
    return () => void off.then((fn) => fn());
  }, []);

  const choose = (id: AppIconId) => {
    if (id === (requested.current ?? current)) return;
    requested.current = id;
    void setAppIcon(id)
      .then(() => {
        if (requested.current === id) requested.current = null;
        setCurrent(id);
      })
      .catch(() => {
        if (requested.current === id) requested.current = null;
      });
  };

  const onKeyDown = (e: React.KeyboardEvent, index: number) => {
    const next = radioStep(index, e.key, APP_ICONS.length);
    if (next === null) return;
    e.preventDefault();
    refs.current[next]?.focus();
    choose(APP_ICONS[next]);
  };

  // Antes da resposta do core nenhum está marcado, mas o grupo precisa de uma
  // parada de Tab: o primeiro.
  const tabStop = current ? APP_ICONS.indexOf(current) : 0;

  return (
    <SettingLine
      label={<span id="app-icon-label">{t("cfgIconName", { name: APP_ICON_NAMES[current ?? APP_ICONS[0]] })}</span>}
      hint={t(IS_MAC ? "cfgIconHintMac" : "appIconHint")}
    >
      <div role="radiogroup" aria-labelledby="app-icon-label" className="flex gap-1.5">
        {APP_ICONS.map((id, index) => {
          const selected = id === current;
          return (
            <button
              key={id}
              ref={(el) => {
                refs.current[index] = el;
              }}
              type="button"
              role="radio"
              aria-checked={selected}
              aria-label={APP_ICON_NAMES[id]}
              title={APP_ICON_NAMES[id]}
              tabIndex={index === tabStop ? 0 : -1}
              onClick={() => choose(id)}
              onKeyDown={(e) => onKeyDown(e, index)}
              className={`grid size-10 place-items-center overflow-hidden rounded-[11px] p-0 transition-shadow focus-visible:outline-none focus-visible:[box-shadow:var(--tyba-focus-ring)] ${
                selected
                  ? "[box-shadow:0_0_0_2px_var(--tyba-bg),0_0_0_4px_var(--tyba-primary)]"
                  : "[box-shadow:inset_0_0_0_1px_var(--tyba-line)]"
              }`}
            >
              <img src={THUMBS[id]} alt="" width={40} height={40} draggable={false} className="size-10" />
            </button>
          );
        })}
      </div>
    </SettingLine>
  );
}

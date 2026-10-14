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
 * "Ícone do app": grade de rádios. Quem decide e aplica é o core; a seleção
 * só muda quando ele confirma (resposta do comando ou o evento, que também
 * acompanha a escolha feita em outra janela).
 */
export function AppIconPicker() {
  const { t } = useTranslation();
  const [current, setCurrent] = useState<AppIconId | null>(null);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    const accept = (id: string) => {
      if (isAppIconId(id)) setCurrent(id);
    };
    void getAppIcon().then(accept).catch(() => {});
    const off = onAppIconChanged(accept);
    return () => void off.then((fn) => fn());
  }, []);

  const choose = (id: AppIconId) => {
    if (id === current) return;
    void setAppIcon(id)
      .then(() => setCurrent(id))
      .catch(() => {});
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
    <>
      <span id="app-icon-label" className="tyba-label">
        {t("appIconSection")}
      </span>
      <p className="pt-1 pb-3 text-[11px] text-tyba-text-faint">
        {t(IS_MAC ? "appIconHintMac" : "appIconHint")}
      </p>
      <div
        role="radiogroup"
        aria-labelledby="app-icon-label"
        className="grid grid-cols-4 gap-2"
      >
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
              tabIndex={index === tabStop ? 0 : -1}
              onClick={() => choose(id)}
              onKeyDown={(e) => onKeyDown(e, index)}
              className={`flex flex-col items-center gap-1.5 rounded-[6px] border p-2 transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-tyba-border-strong ${
                selected
                  ? "border-tyba-green [box-shadow:0_0_0_1px_var(--tyba-green)]"
                  : "border-tyba-border hover:border-tyba-border-strong"
              }`}
            >
              <img
                src={THUMBS[id]}
                alt=""
                width={56}
                height={56}
                draggable={false}
                className="size-14"
              />
              <span className="text-[12px] text-tyba-text">
                {APP_ICON_NAMES[id]}
              </span>
            </button>
          );
        })}
      </div>
    </>
  );
}

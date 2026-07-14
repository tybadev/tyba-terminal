import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "@phosphor-icons/react";
import { IS_MAC } from "@/lib/platform";

type ResizeDirection =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

const EDGES: { dir: ResizeDirection; className: string }[] = [
  { dir: "North", className: "inset-x-0 top-0 h-[3px] cursor-ns-resize" },
  { dir: "South", className: "inset-x-0 bottom-0 h-[3px] cursor-ns-resize" },
  { dir: "West", className: "inset-y-0 left-0 w-[3px] cursor-ew-resize" },
  { dir: "East", className: "inset-y-0 right-0 w-[3px] cursor-ew-resize" },
  {
    dir: "NorthWest",
    className: "left-0 top-0 h-3 w-3 cursor-nwse-resize",
  },
  {
    dir: "NorthEast",
    className: "right-0 top-0 h-3 w-3 cursor-nesw-resize",
  },
  {
    dir: "SouthWest",
    className: "bottom-0 left-0 h-3 w-3 cursor-nesw-resize",
  },
  {
    dir: "SouthEast",
    className: "bottom-0 right-0 h-3 w-3 cursor-nwse-resize",
  },
];

export function WindowResizeEdges() {
  if (IS_MAC) return null;
  return (
    <div className="pointer-events-none fixed inset-0 z-50">
      {EDGES.map(({ dir, className }) => (
        <div
          key={dir}
          role="presentation"
          className={`pointer-events-auto absolute ${className}`}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            void getCurrentWindow()
              .startResizeDragging(dir)
              .catch(() => {});
          }}
        />
      ))}
    </div>
  );
}

export function WindowControls() {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (IS_MAC) return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void win
      .isMaximized()
      .then((v) => {
        if (!cancelled) setMaximized(v);
      })
      .catch(() => {});
    void win
      .onResized(() => {
        void win
          .isMaximized()
          .then((v) => {
            if (!cancelled) setMaximized(v);
          })
          .catch(() => {});
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  if (IS_MAC) return null;

  const win = getCurrentWindow();
  const buttonClass =
    "flex h-9 w-11 items-center justify-center text-tyba-text-muted transition-colors hover:bg-tyba-text/[.08] hover:text-tyba-text";

  return (
    <div className="-mr-2.5 ml-1 flex h-9 shrink-0 items-stretch">
      <button
        type="button"
        aria-label={t("windowMinimize")}
        title={t("windowMinimize")}
        className={buttonClass}
        onClick={() => void win.minimize().catch(() => {})}
      >
        <Minus size={14} weight="bold" />
      </button>
      <button
        type="button"
        aria-label={maximized ? t("windowRestore") : t("windowMaximize")}
        title={maximized ? t("windowRestore") : t("windowMaximize")}
        className={buttonClass}
        onClick={() => void win.toggleMaximize().catch(() => {})}
      >
        {maximized ? (
          <Copy size={13} weight="bold" />
        ) : (
          <Square size={12} weight="bold" />
        )}
      </button>
      <button
        type="button"
        aria-label={t("windowClose")}
        title={t("windowClose")}
        className="flex h-9 w-11 items-center justify-center text-tyba-text-muted transition-colors hover:bg-tyba-red hover:text-tyba-text"
        onClick={() => void win.close().catch(() => {})}
      >
        <X size={14} weight="bold" />
      </button>
    </div>
  );
}

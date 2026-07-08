import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Clock as ClockIcon } from "@phosphor-icons/react";

export function Clock() {
  const { i18n } = useTranslation();
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 10_000);
    return () => clearInterval(id);
  }, []);

  const time = new Intl.DateTimeFormat(i18n.language, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(now);
  const date = new Intl.DateTimeFormat(i18n.language, {
    weekday: "short",
    day: "2-digit",
    month: "short",
  }).format(now);

  return (
    <div
      data-tauri-drag-region
      className="flex select-none items-center gap-1.5 px-1.5 font-mono text-[11px] text-tyba-text-faint tabular-nums"
      title={date}
    >
      <ClockIcon size={13} className="opacity-70" />
      <span className="text-tyba-text-muted">{time}</span>
      <span className="hidden sm:inline">{date}</span>
    </div>
  );
}

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Clock as ClockIcon } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

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
    weekday: "long",
    day: "2-digit",
    month: "long",
    year: "numeric",
  }).format(now);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={time}
          className="size-6 rounded-[4px] text-tyba-text-muted hover:text-tyba-text"
        >
          <ClockIcon size={16} />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-40 px-3 py-2.5">
        <div className="text-center">
          <div className="font-mono text-xl tabular-nums text-tyba-text">
            {time}
          </div>
          <div className="pt-0.5 text-[11px] text-tyba-text-faint first-letter:uppercase">
            {date}
          </div>
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

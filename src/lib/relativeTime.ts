// "Há 2 horas", "ontem": quando algo foi usado por último, relativo a agora.
// Aceita ISO (host SSH) ou epoch em ms (pasta recente).

export function formatLastConnected(
  when: string | number,
  locale: string,
): string {
  const date = new Date(when);
  if (Number.isNaN(date.getTime())) return String(when);
  const diffMs = date.getTime() - Date.now();
  const diffMinutes = Math.round(diffMs / 60000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  if (Math.abs(diffMinutes) < 60) return rtf.format(diffMinutes, "minute");
  const diffHours = Math.round(diffMinutes / 60);
  if (Math.abs(diffHours) < 24) return rtf.format(diffHours, "hour");
  const diffDays = Math.round(diffHours / 24);
  if (Math.abs(diffDays) < 30) return rtf.format(diffDays, "day");
  const diffMonths = Math.round(diffDays / 30);
  if (Math.abs(diffMonths) < 12) return rtf.format(diffMonths, "month");
  const diffYears = Math.round(diffMonths / 12);
  return rtf.format(diffYears, "year");
}

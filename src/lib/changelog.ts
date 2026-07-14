const SITE_URL = "https://tyba.dev";

const SITE_LOCALES = ["pt-br", "en"] as const;

export function siteLocale(language: string): string {
  const normalized = language.toLowerCase();
  const exact = SITE_LOCALES.find((l) => l === normalized);
  if (exact) return exact;
  const base = normalized.split("-")[0];
  const prefixed = SITE_LOCALES.find((l) => l.split("-")[0] === base);
  return prefixed ?? "en";
}

export function changelogUrl(language: string): string {
  return `${SITE_URL}/${siteLocale(language)}/changelog`;
}

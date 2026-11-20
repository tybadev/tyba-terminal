import { siteLocale } from "./changelog";

export const REPO_URL = "https://github.com/tybadev/tyba-terminal";
export const LICENSE_URL = `${REPO_URL}/blob/main/LICENSE`;
export const LICENSE_NAME = "Apache-2.0";

const DOCS_URL = "https://docs.tyba.dev";

export function docsUrl(language: string): string {
  return `${DOCS_URL}/${siteLocale(language)}`;
}

/** A página da doc sobre fazer e importar um tema, no idioma do app. */
export function themeDocsUrl(language: string): string {
  const locale = siteLocale(language);
  return `${DOCS_URL}/${locale}/${locale === "pt-br" ? "temas/importar-tema" : "themes/import-a-theme"}`;
}

export function commitUrl(commit: string): string {
  return `${REPO_URL}/commit/${commit}`;
}

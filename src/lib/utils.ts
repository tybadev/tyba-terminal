import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Remove o prefixo verbatim do Windows (`\\?\`, `\\?\UNC\`) para exibição. */
export const stripVerbatim = (dir: string): string =>
  dir.replace(/^\\\\\?\\UNC\\/, "\\\\").replace(/^\\\\\?\\/, "");

export const basename = (dir: string): string => {
  const clean = stripVerbatim(dir);
  return clean.split(/[/\\]+/).filter(Boolean).pop() ?? clean;
};

import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export const basename = (dir: string): string =>
  dir.split("/").filter(Boolean).pop() ?? dir;

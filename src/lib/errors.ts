export interface AppError {
  code: string;
  params: Record<string, string>;
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.params === "object" &&
    candidate.params !== null
  );
}

function coerce(err: unknown): AppError | string {
  if (typeof err === "string") {
    const trimmed = err.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        const parsed = JSON.parse(trimmed);
        if (isAppError(parsed)) return parsed;
      } catch {
        return err;
      }
    }
    return err;
  }
  if (isAppError(err)) return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

export function translateError(err: unknown, t: Translate): string {
  const value = coerce(err);
  if (typeof value === "string") return value;

  const key = `error.${value.code}`;
  const translated = t(key, value.params);
  if (translated !== key) return translated;

  const detail = value.params.detail ?? value.code;
  return t("error.unknown", { detail });
}

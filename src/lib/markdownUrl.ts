export function isRemoteUrl(url: string): boolean {
  const u = url.trim().toLowerCase();
  return (
    u.startsWith("http://") || u.startsWith("https://") || u.startsWith("//")
  );
}

export function isDangerousUrl(url: string): boolean {
  const u = url.trim().toLowerCase();
  return (
    u.startsWith("javascript:") ||
    u.startsWith("vbscript:") ||
    u.startsWith("data:text/html")
  );
}

export function safeMarkdownUrl(url: string): string {
  if (isRemoteUrl(url) || isDangerousUrl(url)) return "";
  return url;
}

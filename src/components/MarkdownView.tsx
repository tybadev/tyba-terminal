import { useEffect, useMemo, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import { highlightBlock, langForFence, langForFile, type TokenSpan } from "@/lib/highlight";
import { isRemoteUrl, safeMarkdownUrl } from "@/lib/markdownUrl";

function useTokens(lines: string[], lang: string | null, dark: boolean) {
  const [tokens, setTokens] = useState<TokenSpan[][] | null>(null);
  useEffect(() => {
    setTokens(null);
    if (!lang) return;
    let alive = true;
    void highlightBlock(lines, lang, dark ? "mono-dark" : "vitesse-dark")
      .then((r) => {
        if (alive) setTokens(r);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [lines, lang, dark]);
  return tokens;
}

function MarkdownCode({ code, lang, dark }: { code: string; lang: string | null; dark: boolean }) {
  const body = code.replace(/\n+$/, "");
  const lines = useMemo(() => body.split("\n"), [body]);
  const tokens = useTokens(lines, lang, dark);
  return (
    <pre className="files-md-pre">
      <code>
        {tokens
          ? tokens.map((line, i) => (
              <div key={i}>
                {line.length === 0
                  ? "\n"
                  : line.map((tk, j) => (
                      <span key={j} style={{ color: tk.color }}>
                        {tk.text}
                      </span>
                    ))}
              </div>
            ))
          : body}
      </code>
    </pre>
  );
}

/** Markdown renderizado (GFM). Imagem remota não carrega; link não navega. */
export function MarkdownView({ text, dark }: { text: string; dark: boolean }) {
  const components = useMemo<Components>(
    () => ({
      img(props) {
        const { src, alt } = props;
        return typeof src === "string" && src.length > 0 && !isRemoteUrl(src) ? (
          <img src={src} alt={alt ?? ""} className="max-w-full" />
        ) : (
          <span className="text-tyba-text-faint">{alt ?? ""}</span>
        );
      },
      a(props) {
        const { href, children } = props;
        return (
          <a href={href || undefined} onClick={(e) => e.preventDefault()}>
            {children}
          </a>
        );
      },
      pre(props) {
        return <>{props.children}</>;
      },
      code(props) {
        const { className, children } = props;
        const code = String(children ?? "");
        const match = /language-([\w-]+)/.exec(className || "");
        if (!match && !code.includes("\n")) {
          return <code className="files-md-inline">{children}</code>;
        }
        return (
          <MarkdownCode code={code} lang={match ? langForFence(match[1]) : null} dark={dark} />
        );
      },
    }),
    [dark],
  );
  return (
    <div className="files-markdown">
      <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={safeMarkdownUrl} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

/** Fonte só leitura com realce (Shiki) — a leitura do arquivo sem editor. */
export function HighlightedSource({ text, path, dark }: { text: string; path: string; dark: boolean }) {
  const lines = useMemo(() => text.split("\n"), [text]);
  const tokens = useTokens(lines, langForFile(path), dark);
  return (
    <div className="min-w-0 py-3 font-mono text-[12px] leading-[1.5] text-tyba-text">
      {lines.map((raw, i) => {
        const line = tokens?.[i];
        return (
          <div key={i} className="whitespace-pre px-4">
            {line
              ? line.length === 0
                ? " "
                : line.map((tk, j) => (
                    <span key={j} style={{ color: tk.color }}>
                      {tk.text}
                    </span>
                  ))
              : raw === ""
                ? " "
                : raw}
          </div>
        );
      })}
    </div>
  );
}

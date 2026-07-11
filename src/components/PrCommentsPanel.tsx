import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PaperPlaneTilt, WarningCircle } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  forgePrComments,
  type ForgeReviewComment,
  type PullRequest,
} from "../lib/ipc";
import { buildForgeCommentPrompt } from "../lib/forge";

interface Props {
  sessionId: string;
  pr: PullRequest;
  onSendToAgent: (prompt: string) => Promise<void>;
}

export function PrCommentsPanel({ sessionId, pr, onSendToAgent }: Props) {
  const { t } = useTranslation();
  const [comments, setComments] = useState<ForgeReviewComment[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [sendState, setSendState] = useState<
    "idle" | "sending" | "sent" | "error"
  >("idle");
  const [sendError, setSendError] = useState<string | null>(null);

  useEffect(() => {
    setComments(null);
    setLoadError(null);
    setSelected(new Set());
    setSendState("idle");
    void forgePrComments(sessionId, pr.number)
      .then(setComments)
      .catch((e) => setLoadError(String(e)));
  }, [sessionId, pr.number]);

  const selectedComments = useMemo(
    () => (comments ?? []).filter((c) => selected.has(c.id)),
    [comments, selected],
  );

  const preview = useMemo(
    () =>
      selectedComments.length > 0
        ? buildForgeCommentPrompt(pr, selectedComments)
        : "",
    [pr, selectedComments],
  );

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    if (!comments) return;
    setSelected((prev) =>
      prev.size === comments.length ? new Set() : new Set(comments.map((c) => c.id)),
    );
  };

  const send = async () => {
    if (selectedComments.length === 0 || sendState === "sending") return;
    setSendState("sending");
    setSendError(null);
    try {
      await onSendToAgent(preview);
      setSendState("sent");
      setSelected(new Set());
    } catch (e) {
      setSendState("error");
      setSendError(String(e));
    }
  };

  return (
    <div className="flex flex-col gap-2 border-t border-tyba-border p-3">
      <div className="flex items-center justify-between">
        <span className="tyba-label">{t("prCommentsTitle")}</span>
        {comments && comments.length > 0 && (
          <button
            onClick={toggleAll}
            className="text-[11px] text-tyba-text-faint hover:text-tyba-text"
          >
            {selected.size === comments.length
              ? t("prCommentsClearSelection")
              : t("prCommentsSelectAll")}
          </button>
        )}
      </div>

      {loadError && (
        <div className="text-[12px] text-tyba-red">{loadError}</div>
      )}
      {!comments && !loadError && (
        <div className="text-[12px] text-tyba-text-faint">
          {t("prCommentsLoading")}
        </div>
      )}
      {comments && comments.length === 0 && (
        <div className="text-[12px] text-tyba-text-faint">
          {t("prCommentsEmpty")}
        </div>
      )}

      {comments && comments.length > 0 && (
        <div className="flex flex-col gap-1.5">
          {comments.map((c) => (
            <label
              key={c.id}
              className="flex items-start gap-2 rounded-[6px] border border-tyba-border p-2 text-[12px]"
            >
              <input
                type="checkbox"
                checked={selected.has(c.id)}
                onChange={() => toggle(c.id)}
                className="mt-0.5 shrink-0 accent-tyba-green"
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-[11px] text-tyba-text-faint">
                  <span className="font-medium text-tyba-text">
                    @{c.author}
                  </span>
                  {c.path && (
                    <span className="font-mono">
                      {c.path}
                      {c.line !== null ? `:${c.line}` : ""}
                    </span>
                  )}
                </div>
                <div className="whitespace-pre-wrap pt-0.5 text-tyba-text">
                  {c.body}
                </div>
              </div>
            </label>
          ))}
        </div>
      )}

      {selectedComments.length > 0 && (
        <div className="flex flex-col gap-1.5 rounded-[6px] border border-tyba-yellow/30 bg-tyba-yellow/[.05] p-2">
          <div className="flex items-center gap-1.5 text-[11px] text-tyba-yellow">
            <WarningCircle size={12} />
            {t("prCommentsUntrustedHint")}
          </div>
          <div className="tyba-label">{t("prCommentsPreviewTitle")}</div>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap font-mono text-[11px] text-tyba-text-muted">
            {preview}
          </pre>
        </div>
      )}

      <div className="flex items-center gap-2">
        {sendState === "sent" && selectedComments.length === 0 && (
          <span className="text-[11px] text-tyba-green">
            {t("prCommentsSent")}
          </span>
        )}
        {sendError && (
          <span className="text-[11px] text-tyba-red">{sendError}</span>
        )}
        <div className="flex-1" />
        {selectedComments.length > 0 && (
          <Button
            size="sm"
            className="h-6 gap-1.5 px-2.5 text-[11px]"
            disabled={sendState === "sending"}
            onClick={() => void send()}
          >
            <PaperPlaneTilt size={12} />
            {sendState === "sending"
              ? t("prCommentsSending")
              : t("prCommentsSend", { count: selectedComments.length })}
          </Button>
        )}
      </div>
    </div>
  );
}

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { translateError } from "../lib/errors";
import {
  CaretDown,
  CaretRight,
  PaperPlaneTilt,
  WarningCircle,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
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
  const [previewOpen, setPreviewOpen] = useState(true);

  useEffect(() => {
    setComments(null);
    setLoadError(null);
    setSelected(new Set());
    setSendState("idle");
    void forgePrComments(sessionId, pr.number)
      .then(setComments)
      .catch((e) => setLoadError(translateError(e, t)));
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
      setSendError(translateError(e, t));
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
        <div className="flex flex-col divide-y divide-tyba-border/60">
          {comments.map((c) => (
            <label
              key={c.id}
              className="flex items-start gap-2 px-1 py-1.5 text-[12px] hover:bg-white/[.02]"
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
        <div className="flex flex-col gap-1 pt-1">
          <div className="flex items-center gap-1.5 text-[10px] text-tyba-text-faint">
            <WarningCircle size={11} />
            {t("prCommentsUntrustedHint")}
          </div>
          <Collapsible open={previewOpen} onOpenChange={setPreviewOpen}>
            <CollapsibleTrigger className="flex items-center gap-1 text-[10px] font-medium uppercase tracking-wide text-tyba-text-faint hover:text-tyba-text-muted">
              {previewOpen ? (
                <CaretDown size={9} className="shrink-0" />
              ) : (
                <CaretRight size={9} className="shrink-0" />
              )}
              {t("prCommentsPreviewTitle")}
            </CollapsibleTrigger>
            <CollapsibleContent>
              <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-[4px] bg-black/10 p-2 font-mono text-[11px] text-tyba-text-muted">
                {preview}
              </pre>
            </CollapsibleContent>
          </Collapsible>
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

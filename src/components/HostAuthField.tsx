import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  CheckCircle,
  CircleNotch,
  FolderOpen,
  Key,
} from "@phosphor-icons/react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { homeDir, join } from "@tauri-apps/api/path";

import { Input } from "@/components/ui/input";
import { translateError } from "@/lib/errors";
import { agentKeyType, agentListingState } from "@/lib/hostForm";
import {
  listAgentKeys,
  type AgentKey,
  type AgentKeyListing,
  type AuthMethod,
} from "@/lib/ipc";

const METHODS: Array<{ value: AuthMethod; labelKey: string }> = [
  { value: "auto", labelKey: "hostAuthAuto" },
  { value: "agent", labelKey: "hostAuthAgent" },
  { value: "file", labelKey: "hostAuthFile" },
  { value: "password", labelKey: "hostAuthPassword" },
];

interface Props {
  method: AuthMethod;
  identityFile: string;
  agentKey: AgentKey | null;
  /** Alias já salvo: o agente é o que o `ssh -G` resolve para ele. `null` em Host novo. */
  savedAlias: string | null;
  onMethodChange: (method: AuthMethod) => void;
  onIdentityFileChange: (path: string) => void;
  onAgentKeyChange: (key: AgentKey) => void;
}

type Listing =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "loaded"; listing: AgentKeyListing }
  | { state: "error"; message: string };

async function sshDirectory(): Promise<string | undefined> {
  try {
    return await join(await homeDir(), ".ssh");
  } catch {
    return undefined;
  }
}

function AgentKeyPicker({
  savedAlias,
  selected,
  onSelect,
}: {
  savedAlias: string | null;
  selected: AgentKey | null;
  onSelect: (key: AgentKey) => void;
}) {
  const { t } = useTranslation();
  const [listing, setListing] = useState<Listing>({ state: "idle" });

  const load = useCallback(() => {
    setListing({ state: "loading" });
    listAgentKeys(savedAlias)
      .then((result) => setListing({ state: "loaded", listing: result }))
      .catch((e) => setListing({ state: "error", message: translateError(e, t) }));
  }, [savedAlias, t]);

  useEffect(() => {
    load();
  }, [load]);

  const keys = listing.state === "loaded" ? listing.listing.keys : [];
  const selectedListed =
    selected !== null && keys.some((k) => k.fingerprint === selected.fingerprint);

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate font-mono text-[10px] text-tyba-text-faint">
          {listing.state === "loaded" && listing.listing.socket
            ? t("hostAuthAgentSocket", { socket: listing.listing.socket })
            : t("hostAuthAgentHint")}
        </span>
        <button
          type="button"
          onClick={load}
          disabled={listing.state === "loading"}
          className="flex shrink-0 items-center gap-1 text-[11px] text-tyba-text-faint hover:text-tyba-text disabled:opacity-50"
        >
          <ArrowClockwise size={11} />
          {t("hostAuthAgentRefresh")}
        </button>
      </div>

      {listing.state === "loading" && (
        <p className="flex items-center gap-1.5 text-[11px] text-tyba-text-faint">
          <CircleNotch size={11} className="animate-spin" weight="bold" />
          {t("hostAuthAgentLoading")}
        </p>
      )}
      {listing.state === "error" && (
        <p className="text-[11px] text-tyba-red">{listing.message}</p>
      )}
      {listing.state === "loaded" &&
        agentListingState(listing.listing) === "no_agent" && (
          <p className="text-[11px] text-tyba-amber">{t("hostAuthAgentNoAgent")}</p>
        )}
      {listing.state === "loaded" &&
        agentListingState(listing.listing) === "empty" && (
          <p className="text-[11px] text-tyba-amber">{t("hostAuthAgentEmpty")}</p>
        )}

      {keys.length > 0 && (
        <div
          role="listbox"
          aria-label={t("hostAuthAgent")}
          className="flex max-h-40 flex-col overflow-y-auto rounded-[4px] border border-tyba-border"
        >
          {keys.map((key) => {
            const active = selected?.fingerprint === key.fingerprint;
            return (
              <button
                key={key.fingerprint}
                type="button"
                role="option"
                aria-selected={active}
                onClick={() => onSelect(key)}
                className={`flex items-center gap-2 px-2 py-1.5 text-left transition-colors ${
                  active ? "bg-tyba-text/[.06]" : "hover:bg-tyba-text/[.03]"
                }`}
              >
                {active ? (
                  <CheckCircle size={13} weight="fill" className="shrink-0 text-tyba-green" />
                ) : (
                  <Key size={13} className="shrink-0 text-tyba-text-faint" />
                )}
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12px] text-tyba-text">
                    {key.name || agentKeyType(key.public_key)}
                  </span>
                  <span className="block truncate font-mono text-[10px] text-tyba-text-faint">
                    {agentKeyType(key.public_key)} · {key.fingerprint}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      )}

      {selected !== null && listing.state === "loaded" && !selectedListed && (
        <p className="text-[11px] text-tyba-amber">
          {t("hostAuthAgentKeyMissing")}
          <span className="block truncate font-mono text-[10px] text-tyba-text-faint">
            {selected.name} · {selected.fingerprint}
          </span>
        </p>
      )}
    </div>
  );
}

export function HostAuthField({
  method,
  identityFile,
  agentKey,
  savedAlias,
  onMethodChange,
  onIdentityFileChange,
  onAgentKeyChange,
}: Props) {
  const { t } = useTranslation();

  const browse = async () => {
    const picked = await openFileDialog({
      multiple: false,
      directory: false,
      defaultPath: await sshDirectory(),
    });
    if (typeof picked === "string") onIdentityFileChange(picked);
  };

  return (
    <div className="flex flex-col gap-2">
      <div
        role="radiogroup"
        aria-label={t("hostAuthMethod")}
        className="grid grid-cols-4 gap-1 rounded-[4px] border border-tyba-border p-0.5"
      >
        {METHODS.map((m) => (
          <button
            key={m.value}
            type="button"
            role="radio"
            aria-checked={method === m.value}
            onClick={() => onMethodChange(m.value)}
            className={`truncate rounded-[3px] px-1.5 py-1 text-[11px] transition-colors ${
              method === m.value
                ? "bg-tyba-text/[.08] text-tyba-text"
                : "text-tyba-text-faint hover:text-tyba-text"
            }`}
          >
            {t(m.labelKey)}
          </button>
        ))}
      </div>

      {method === "auto" && (
        <p className="text-[11px] text-tyba-text-faint">{t("hostAuthAutoHint")}</p>
      )}

      {method === "agent" && (
        <AgentKeyPicker
          savedAlias={savedAlias}
          selected={agentKey}
          onSelect={onAgentKeyChange}
        />
      )}

      {method === "file" && (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-1.5">
            <Input
              id="host-identity-file"
              aria-label={t("hostFieldIdentityFile")}
              className="font-mono"
              value={identityFile}
              placeholder={t("hostFieldIdentityFilePlaceholder")}
              onChange={(e) => onIdentityFileChange(e.target.value)}
            />
            <button
              type="button"
              onClick={() => void browse()}
              className="flex h-9 shrink-0 items-center gap-1.5 rounded-md border border-tyba-border px-2.5 text-[11px] text-tyba-text-muted hover:text-tyba-text"
            >
              <FolderOpen size={13} />
              {t("hostAuthFileBrowse")}
            </button>
          </div>
          <span className="text-[11px] text-tyba-text-faint">{t("hostAuthFileHint")}</span>
        </div>
      )}

      {method === "password" && (
        <p className="text-[11px] text-tyba-text-faint">{t("hostAuthPasswordHint")}</p>
      )}
    </div>
  );
}

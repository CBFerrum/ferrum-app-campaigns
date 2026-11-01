// An email as people will get it: the server builds it (its templates, its footer, its preview text)
// for a stand-in contact or a real one and for each version of an A/B test, and it is shown in a
// frame the width of a computer or a phone, under the inbox line that leads to it. It shows the
// last saved version, and says so while there is something unsaved.

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, docPath, errorMessage } from "@ferrum/client";
import { Alert, Icon, Spinner } from "@ferrum/kit";
import { LinkInput } from "@ferrum/records";
import type { Campaign } from "./campaign";
import { list, previewOf } from "./data";
import { clip, PREHEADER_ROOM, preheaderOf, previewDoc, SUBJECT_ROOM } from "./preview";

export type Device = "desktop" | "phone";

/** The saved email, built by the server; it follows the saved version, and keeps the last one on screen while the next loads. */
export function usePreview(id: string, version: number, o: { variant: string | null; contact: string | null; enabled?: boolean }) {
  return useQuery({
    queryKey: ["campaigns", "preview", id, version, o.variant, o.contact],
    queryFn: () => previewOf(id, { variant: o.variant, contact: o.contact }),
    enabled: o.enabled ?? true,
    placeholderData: keepPreviousData,
  });
}

interface Account {
  account_name: string;
  email_address: string;
  display_name?: string | null;
}

/** Who the email comes from: the chosen account, or else the workspace's default one. */
export function useSender(account: string | null | undefined) {
  return useQuery({
    queryKey: ["campaigns", "sender", account ?? "default"],
    staleTime: 5 * 60_000,
    queryFn: async () => {
      const a = account
        ? await api.get<Account>(docPath("EmailAccount", account))
        : (await list<Account>("EmailAccount", { filters: JSON.stringify({ default_outgoing: true, enable_outgoing: true }), fields: "account_name,email_address,display_name", limit: 1 })).rows[0];
      return a ? { name: a.display_name || a.account_name, address: a.email_address } : null;
    },
  });
}

export function DeviceSwitch({ value, onChange }: { value: Device; onChange: (d: Device) => void }) {
  return (
    <div className="segmented" role="group" aria-label="Show it on">
      <button type="button" aria-pressed={value === "desktop"} onClick={() => onChange("desktop")}>
        Computer
      </button>
      <button type="button" aria-pressed={value === "phone"} onClick={() => onChange("phone")}>
        Phone
      </button>
    </div>
  );
}

/** The line an inbox shows for the email: who from, the subject, the first words. */
export function InboxLine({ sender, subject, preheader, device }: { sender?: { name: string; address: string } | null; subject: string; preheader: string; device: Device }) {
  const cut = clip(subject, SUBJECT_ROOM[device]);
  return (
    <div className={`cm-inbox cm-inbox-${device}`} role="group" aria-label="How it looks in an inbox">
      <span className="cm-inbox-mark" aria-hidden="true">
        {(sender?.name ?? "?").slice(0, 1).toUpperCase()}
      </span>
      <div className="cm-inbox-text">
        <div className="cm-inbox-from">
          <strong>{sender?.name ?? "Your sender"}</strong>
          {sender && <span className="faint"> {sender.address}</span>}
        </div>
        <div className="cm-inbox-subject">{cut || <span className="faint">No subject yet</span>}</div>
        <div className="cm-inbox-preheader muted">{clip(preheader, PREHEADER_ROOM[device]) || " "}</div>
      </div>
      {cut !== subject.trim() && <span className="cm-inbox-cut faint">Cut off</span>}
    </div>
  );
}

export function PreviewPane({ doc, dirty, editable }: { doc: Campaign; dirty: boolean; editable: boolean }) {
  const [device, setDevice] = useState<Device>("desktop");
  const [chosen, setChosen] = useState<string | null>(null);
  const [contact, setContact] = useState<{ id: string; title?: string } | null>(null);
  const versions = doc.ab_test ? doc.variants.map((v) => v.variant).filter((v): v is string => !!v) : [];
  const variant = versions.includes(chosen ?? "") ? chosen : (versions[0] ?? null);
  const preview = usePreview(doc.id, doc.version, { variant, contact: contact?.id ?? null });
  const sender = useSender(doc.email_account);
  const p = preview.data;

  return (
    <div className="cm-preview">
      <div className="cm-preview-bar">
        <DeviceSwitch value={device} onChange={setDevice} />
        {versions.length > 1 && (
          <div className="segmented" role="group" aria-label="Version">
            {versions.map((v) => (
              <button key={v} type="button" aria-pressed={variant === v} onClick={() => setChosen(v)}>
                {v}
              </button>
            ))}
          </div>
        )}
        <span className="spacer" />
        {preview.isFetching && <Spinner small label="Updating the preview" />}
      </div>
      <div className="cm-preview-as">
        <span className="faint">As</span>
        <div className="cm-preview-contact">
          <LinkInput
            to="MarketingContact"
            id="preview-contact"
            label="Preview as this contact"
            placeholder="A sample contact (Jane Doe)"
            value={contact?.id ?? null}
            title={contact?.title}
            onChange={(id, title) => setContact(id ? { id, title } : null)}
          />
        </div>
      </div>
      {dirty && editable && (
        <p className="cm-preview-note faint" role="status">
          <Icon name="refresh" size={12} /> Showing your last save. It updates in a moment.
        </p>
      )}
      {preview.error && !p ? (
        <Alert tone="error">
          <strong>The preview could not be built.</strong> {errorMessage(preview.error)}
        </Alert>
      ) : (
        <>
          <InboxLine sender={sender.data} subject={p?.subject ?? ""} preheader={p ? preheaderOf(p.html) : ""} device={device} />
          <div className={`cm-device cm-device-${device}`} data-stale={preview.isPlaceholderData || undefined}>
            {p ? (
              <iframe title={`Preview of the email on a ${device === "phone" ? "phone" : "computer"}`} sandbox="allow-popups allow-popups-to-escape-sandbox" referrerPolicy="no-referrer" srcDoc={previewDoc(p.html)} />
            ) : (
              <div className="cm-device-wait">
                <Spinner label="Building the preview" />
              </div>
            )}
          </div>
          <p className="cm-preview-note faint">Links open in a new tab. The unsubscribe link works only in a real email.</p>
        </>
      )}
    </div>
  );
}

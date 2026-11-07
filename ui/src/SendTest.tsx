// A test of the email, sent to people who can check it: up to ten addresses, the subject marked
// [Test], nothing tracked and nothing counted in the results. What goes out is the saved campaign,
// so it is saved first; a version of an A/B test, or a real contact's details, can be chosen.

import { type FormEvent, useState } from "react";
import { errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, Dialog, Field, toast } from "@ferrum/kit";
import { LinkInput, useMe } from "@ferrum/records";
import { sendTest } from "./data";
import { noun } from "./format";
import { splitAddresses } from "./importer";
import type { Draft } from "./useDraft";

export function SendTest({ draft, onClose }: { draft: Draft; onClose: () => void }) {
  const doc = draft.doc!;
  const me = useMe().data;
  const [to, setTo] = useState(me?.user.email ?? "");
  const versions = doc.ab_test ? doc.variants.map((v) => v.variant).filter((v): v is string => !!v) : [];
  const [variant, setVariant] = useState<string | null>(versions[0] ?? null);
  const [contact, setContact] = useState<{ id: string; title?: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { valid, invalid } = splitAddresses(to);
  const problem = invalid.length ? `${invalid.join(", ")} ${invalid.length === 1 ? "is" : "are"} not ${invalid.length === 1 ? "an email address" : "email addresses"}.` : valid.length > 10 ? "Send a test to 10 addresses at most." : null;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid.length || problem) return;
    setBusy(true);
    setError(null);
    try {
      // What is sent is what is saved: make sure the last changes are.
      const saved = await draft.save();
      if (!saved) throw new Error("The campaign could not be saved yet. Fix what is marked first.");
      const r = await sendTest(saved.id, { emails: valid, variant, contact: contact?.id });
      toast(t("Test sent to {value}", { value: noun(r.sent, "address", "addresses") }));
      onClose();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={t("Send a test")}
      footer={
        <>
          <Button onClick={onClose}>{t("Cancel")}</Button>
          <Button type="submit" form="cm-test-form" variant="primary" icon="mail" loading={busy} disabled={!valid.length || !!problem}>
            {t("Send")} {valid.length > 1 ? t("to {length} addresses", { length: valid.length }) : t("test")}
          </Button>
        </>
      }
    >
      <form id="cm-test-form" className="stack" onSubmit={submit}>
        {error && <Alert tone="error">{isolate(error)}</Alert>}
        <Field label={t("Send it to")} htmlFor="cm-test-to" required error={problem ?? undefined} hint={t("Up to 10 addresses, separated by commas or lines.")}>
          <textarea id="cm-test-to" className="textarea" rows={2} value={to} data-autofocus onChange={(e) => setTo(e.target.value)} />
        </Field>
        {versions.length > 1 && (
          <Field label={t("Which version")} htmlFor="cm-test-version">
            <select id="cm-test-version" className="select" value={variant ?? ""} onChange={(e) => setVariant(e.target.value)}>
              {versions.map((v) => (
                <option key={v} value={v}>{t("Version {v}", { v })}</option>
              ))}
            </select>
          </Field>
        )}
        <Field label={t("Fill in the details of")} htmlFor="cm-test-contact" hint={t("Optional. Without a contact, the details of a sample person (Jane Doe) are used.")}>
          <LinkInput to="MarketingContact" id="cm-test-contact" value={contact?.id ?? null} title={contact?.title} placeholder={t("A sample contact")} onChange={(id, title) => setContact(id ? { id, title } : null)} />
        </Field>
        <p className="faint">{t("The subject starts with [Test]. Nothing in it is tracked, and it does not count in the results. It shows the campaign as saved a moment ago.")}</p>
      </form>
    </Dialog>
  );
}

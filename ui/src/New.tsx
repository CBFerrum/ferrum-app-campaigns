// Starting a campaign: a name, a subject and who it is for. The server wants all three (and some
// content) before it keeps a draft, so this asks for what is needed to begin and no more; the
// rest is written on the campaign's own page, where a draft saves by itself.

import { useMutation } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { api, ApiError, errorMessage, t, isolate } from "@ferrum/client";
import { Alert, Button, EmptyState, ErrorState, Field, PageLoading, useShellTitle } from "@ferrum/kit";
import { can, LinkInput, useEntityMeta } from "@ferrum/records";
import type { AudienceRow, Campaign } from "./campaign";
import { AudienceRows } from "./Setup";
import { STARTER } from "./snippets";

export function NewCampaign() {
  useShellTitle(t("New campaign"));
  const navigate = useNavigate();
  const [sp] = useSearchParams();
  const meta = useEntityMeta("EmailCampaign");
  const [name, setName] = useState("");
  const [subject, setSubject] = useState("");
  const [template, setTemplate] = useState<{ id: string; title?: string } | null>(null);
  // A segment can send a campaign to itself: its page links here with `?segment=`.
  const [audience, setAudience] = useState<AudienceRow[]>(() => [sp.get("segment") ? { segment: sp.get("segment"), exclude: false } : { segment: null, email_group: null, exclude: false }]);
  const create = useMutation({
    mutationFn: () =>
      api.post<Campaign>("/api/v1/EmailCampaign", {
        campaign_name: name.trim(),
        subject: subject.trim() || undefined,
        audience: audience.filter((r) => r.segment || r.email_group).map((r) => ({ segment: r.segment ?? null, email_group: r.email_group ?? null, exclude: !!r.exclude })),
        // A template fills what is blank; without one the email starts from a layout to write over.
        ...(template ? { email_template: template.id } : { content: STARTER }),
      }),
    onSuccess: (made) => navigate(`/c/${made.id}?tab=content`, { replace: true }),
  });

  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <PageLoading />;
  if (!can(meta.data.permissions, "create")) {
    return (
      <EmptyState title={t("You cannot create campaigns")} icon="mail">
        {t("Ask a Marketing Manager for the Marketing User role.")}
      </EmptyState>
    );
  }

  const fields = create.error instanceof ApiError ? Object.fromEntries(create.error.fields.filter((f) => !f.row).map((f) => [f.field, f.message])) : {};
  const chosen = audience.some((r) => (r.segment || r.email_group) && !r.exclude);
  const ready = name.trim() && (subject.trim() || template) && chosen;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ready) create.mutate();
  };

  return (
    <div className="cm-page cm-narrow">
      <header className="cm-head">
        <div>
          <h1 className="page-title">{t("New campaign")}</h1>
          <p className="page-sub">{t("Start with the basics. You write the email next, and a draft saves as you go.")}</p>
        </div>
      </header>
      <form className="card card-pad stack" onSubmit={submit}>
        {create.error && !Object.keys(fields).length && <Alert tone="error">{isolate(errorMessage(create.error))}</Alert>}
        <Field label={t("Name")} htmlFor="new-name" required error={fields.campaign_name} hint={t("For you and your team. Recipients never see it.")}>
          <input id="new-name" className="input" value={name} maxLength={140} autoFocus onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={t("Subject")} htmlFor="new-subject" required={!template} error={fields.subject} hint="What people see in their inbox. You can use {{ contact.first_name }} here too.">
          <input id="new-subject" className="input" value={subject} maxLength={998} onChange={(e) => setSubject(e.target.value)} />
        </Field>
        <fieldset className="cm-fieldset">
          <legend className="field-label">{t("Who is it for?")}</legend>
          <AudienceRows rows={audience} onChange={setAudience} error={fields.audience} />
          {/* The button below waits for this, so say what it waits for. */}
          {!chosen && (
            <p className="field-hint">
              {t("Choose at least one segment or email group to go on. Nobody to choose yet?")} <Link to="/s/new">{t("Make a segment")}</Link>.
            </p>
          )}
        </fieldset>
        <Field label={t("Start from a Mail template")} htmlFor="new-template" hint={t("Optional. It fills the subject (if empty) and the content. Without one you start from a simple layout.")}>
          <LinkInput to="EmailTemplate" id="new-template" value={template?.id ?? null} title={template?.title} onChange={(id, title) => setTemplate(id ? { id, title } : null)} />
        </Field>
        <div className="row cm-actions">
          <Button type="button" onClick={() => navigate("/")}>
            {t("Cancel")}
          </Button>
          <Button type="submit" variant="primary" loading={create.isPending} disabled={!ready}>
            {t("Create and write the email")}
          </Button>
        </div>
      </form>
    </div>
  );
}

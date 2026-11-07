// One campaign, whole: set it up, write it, check who it will reach and send it, and then judge how
// it did, on one page whose tab is in the address. A draft saves by itself, every change against
// the version on screen; once it is scheduled it is fixed, and what can still be done is to stop it,
// copy it or, if it was cancelled, amend it.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { api, docPath, errorMessage, humanize, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Dialog, ErrorState, Icon, Menu, MenuItem, PageLoading, TabPanel, Tabs, toast, useShellTitle } from "@ferrum/kit";
import { can, CollabPanel, type EntityMeta, useEntityMeta } from "@ferrum/records";
import { audienceForSave, type Campaign, defaultTab, isTab, type Tab, tone, whenLine } from "./campaign";
import { Content } from "./Content";
import { duplicateCampaign, keys, whole } from "./data";
import { ResultsTab } from "./ResultsTab";
import { ReviewTab } from "./ReviewTab";
import { SendTest } from "./SendTest";
import { Setup } from "./Setup";
import { useAudienceCheck } from "./useCheck";
import { useDraft } from "./useDraft";
import { SaveLine } from "./ui";

/** Which tab a field is on, for the count of what could not be saved. */
const TAB_OF: Record<string, Tab> = { content: "content", text_content: "content", schedule_at: "review" };

export function CampaignRoute() {
  const { id = "" } = useParams();
  const meta = useEntityMeta("EmailCampaign");
  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <PageLoading />;
  return <CampaignPage key={id} id={id} meta={meta.data} />;
}

function CampaignPage({ id, meta }: { id: string; meta: EntityMeta }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [sp, setSp] = useSearchParams();
  const draft = useDraft(id, can(meta.permissions, "write"));
  const doc = draft.doc;
  useShellTitle(doc?.campaign_name ?? null);
  const [dialog, setDialog] = useState<"test" | "talk" | null>(null);
  // The audience numbers the Setup tab shows are the server's count of the saved audience, taken again only when that changes.
  const saved = draft.saved;
  const audienceKey = JSON.stringify(audienceForSave(saved?.audience ?? []).map((r) => [r.segment, r.email_group, r.exclude]));
  const asked = sp.get("tab");
  const tab: Tab = isTab(asked) ? asked : doc ? defaultTab(doc) : "setup";
  const counted = useAudienceCheck(id, audienceKey, tab === "setup" && !!saved && (saved.status === "Draft" || saved.status === "Scheduled") && audienceKey !== "[]");

  if (draft.query.error && !doc) return <ErrorState error={draft.query.error} onRetry={() => void draft.query.refetch()} title={tr("This campaign could not be opened")} />;
  if (!doc) return <PageLoading label={tr("Opening the campaign")} />;

  const editable = draft.editable;
  const canSubmit = can(meta.permissions, "submit");
  const canCancel = can(meta.permissions, "cancel");
  const canCreate = can(meta.permissions, "create");
  const go = (t: Tab) =>
    setSp(
      (p) => {
        const next = new URLSearchParams(p);
        next.set("tab", t);
        return next;
      },
      { replace: true },
    );

  const refresh = (c: Campaign) => {
    qc.setQueryData<Campaign>(keys.campaign(id), whole({ ...c, _titles: { ...doc._titles, ...c._titles } }));
    void qc.invalidateQueries({ queryKey: keys.campaign(id) });
    void qc.invalidateQueries({ queryKey: keys.list });
  };

  const stop = async () => {
    const sending = doc.status === "Sending";
    const ok = await confirmDialog({
      title: sending ? tr("Stop sending?") : tr("Cancel this send?"),
      message: sending ? tr("Emails still waiting will not go out. Emails that have already gone cannot be taken back.") : tr("It will not be sent. You can amend it afterwards to start again from a copy."),
      confirmLabel: sending ? tr("Stop sending") : tr("Cancel the send"),
      cancelLabel: tr("Keep it"),
      danger: true,
    });
    if (!ok) return;
    try {
      refresh(await api.post<Campaign>(docPath("EmailCampaign", id, "cancel"), undefined, { ifMatch: doc.version }));
      toast(sending ? tr("Stopped") : tr("Cancelled"));
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const duplicate = async () => {
    try {
      const made = await duplicateCampaign(id);
      toast(tr("Copied. It is a draft: change what you need."));
      navigate(`/c/${made.id}`);
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const amend = async () => {
    try {
      const made = await api.post<Campaign>(docPath("EmailCampaign", id, "amend"));
      toast(tr("Amended: this is the new draft"));
      void qc.invalidateQueries({ queryKey: keys.list });
      navigate(`/c/${made.id}`);
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const remove = async () => {
    const ok = await confirmDialog({ title: tr("Delete “{campaign_name}”?", { campaign_name: doc.campaign_name }), message: tr("It is deleted for good. Nothing was sent."), confirmLabel: tr("Delete"), danger: true });
    if (!ok) return;
    try {
      await api.del(docPath("EmailCampaign", id), { ifMatch: doc.version });
      void qc.invalidateQueries({ queryKey: keys.list });
      toast(tr("Deleted"));
      navigate("/");
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };

  const failed: Record<Tab, number> = { setup: 0, content: 0, review: 0, results: 0 };
  for (const f of Object.keys(draft.fieldErrors)) failed[TAB_OF[f] ?? "setup"]++;
  for (const r of draft.rowErrors) failed[r.field === "content" ? "content" : "setup"]++;

  const scheduledOrOn = doc.status === "Scheduled" || doc.status === "Sending";
  const tabs = [
    { id: "setup", label: <>{tr("Setup")}{failed.setup > 0 && <span className="tab-errors">{failed.setup}</span>}</> },
    { id: "content", label: <>{tr("Content")}{failed.content > 0 && <span className="tab-errors">{failed.content}</span>}</> },
    { id: "review", label: <>{tr("Review")}{doc.status === "Draft" && canSubmit ? tr(" and send") : ""}</> },
    { id: "results", label: tr("Results") },
  ];

  return (
    <div className="cm-page cm-camp">
      <header className="cm-camp-head">
        <div className="crumbs">
          <Link to="/">{tr("Campaigns")}</Link>
          <Icon name="chevronRight" size={12} />
        </div>
        <div className="cm-camp-bar">
          <div className="cm-camp-title">
            <div className="cm-camp-name">
              <h1 className="page-title">{doc.campaign_name || tr("Untitled campaign")}</h1>
              <Badge tone={tone(doc.status)}>{isolate(doc.status)}</Badge>
              {doc.ab_test && <Badge tone="purple">{tr("A/B test")}</Badge>}
            </div>
            <p className="page-sub">
              {isolate([doc.number, whenLine(doc)].filter(Boolean).join(" · "))}
              {editable && (
                <>
                  {isolate(" · ")}
                  <SaveLine draft={draft} editable />
                </>
              )}
            </p>
          </div>
          <div className="page-actions">
            {(doc.status === "Draft" || doc.status === "Scheduled") && (
              <Button icon="mail" onClick={() => setDialog("test")}>
                {tr("Send a test")}
              </Button>
            )}
            {scheduledOrOn && canCancel && (
              <Button variant="danger" onClick={() => void stop()}>
                {doc.status === "Sending" ? tr("Stop sending") : tr("Cancel the send")}
              </Button>
            )}
            {doc.status === "Sent" && canCreate && (
              <Button variant="primary" icon="copy" onClick={() => void duplicate()}>
                {tr("Duplicate")}
              </Button>
            )}
            {doc.status === "Cancelled" && canCreate && (
              <>
                <Button onClick={() => void duplicate()}>{tr("Duplicate")}</Button>
                <Button variant="primary" onClick={() => void amend()}>
                  {tr("Amend")}
                </Button>
              </>
            )}
            {editable && tab !== "review" && (
              <Button variant="primary" onClick={() => go("review")}>
                {canSubmit ? tr("Review and send") : tr("Review")}
              </Button>
            )}
            <Menu ariaLabel="More" icon="chevronDown" buttonClass="icon-btn">
              {(close) => (
                <>
                  <MenuItem onSelect={() => (close(), setDialog("talk"))}>{tr("Comments and history")}</MenuItem>
                  <MenuItem onSelect={() => (close(), navigate(`/records/EmailCampaign/${id}`))}>{tr("Open as a record")}</MenuItem>
                  {canCreate && doc.status !== "Sent" && doc.status !== "Cancelled" && <MenuItem onSelect={() => (close(), void duplicate())}>{tr("Duplicate")}</MenuItem>}
                  {can(meta.permissions, "delete") && (doc.status === "Draft" || doc.status === "Cancelled") && (
                    <MenuItem danger onSelect={() => (close(), void remove())}>
                      {tr("Delete…")}
                    </MenuItem>
                  )}
                </>
              )}
            </Menu>
          </div>
        </div>
      </header>

      {draft.status === "conflict" && (
        <Alert
          tone="warning"
          action={
            <div className="row">
              <Button onClick={() => void draft.reload()}>{tr("Use theirs")}</Button>
              <Button variant="primary" onClick={draft.keepMine}>
                {tr("Keep mine")}
              </Button>
            </div>
          }
        >
          <strong>{tr("Someone else changed this campaign.")}</strong> {tr("You both changed")} {draft.theirs.length ? draft.theirs.map((f) => humanize(f).toLowerCase()).join(", ") : tr("it")}{tr(". Keep what you wrote on top of theirs, or use theirs and lose your changes.")}
        </Alert>
      )}
      {!editable && doc.docstatus === 0 && <Alert tone="info">{tr("You can look at this campaign but not change it.")}</Alert>}
      {doc.docstatus === 1 && (tab === "setup" || tab === "content") && (
        <Alert tone="info" action={canCreate ? <Button onClick={() => void duplicate()}>{tr("Duplicate to change it")}</Button> : undefined}>
          {doc.status === "Scheduled" ? tr("It is scheduled, so it can no longer be edited. Cancel the send to stop it, or duplicate it to change a copy.") : tr("It has gone out, so it can no longer be edited.")}
        </Alert>
      )}

      <Tabs id="cm-tab" tabs={tabs} value={tab} onChange={(t) => go(t as Tab)} />
      <TabPanel id="cm-tab" tab={tab}>
        {tab === "setup" && <Setup draft={draft} editable={editable} check={counted.data} />}
        {tab === "content" && <Content draft={draft} editable={editable} />}
        {tab === "review" && <ReviewTab draft={draft} editable={editable} canSubmit={canSubmit} canCancel={canCancel} go={go} onTest={() => setDialog("test")} onCancel={() => void stop()} />}
        {tab === "results" && <ResultsTab doc={doc} />}
      </TabPanel>

      {dialog === "test" && <SendTest draft={draft} onClose={() => setDialog(null)} />}
      {dialog === "talk" && (
        <Dialog open wide onClose={() => setDialog(null)} title={tr("Comments and history")}>
          <CollabPanel entity="EmailCampaign" id={id} canWrite={can(meta.permissions, "write")} />
        </Dialog>
      )}
    </div>
  );
}

// Who can be emailed: how many people are subscribed and how that has moved in a month, and the
// segments that pick groups of them. A new segment starts here; the file of contacts to bring in is
// one click away.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useCallback, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";
import { api, ApiError, relativeTime, t as tr, isolate } from "@ferrum/client";
import { Button, Dialog, EmptyState, ErrorState, Field, Icon, PageLoading, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta } from "@ferrum/records";
import { countOf, keys, list } from "./data";
import { count, noun } from "./format";
import { LineChart } from "./LineChart";
import { Nav } from "./Nav";

interface Row {
  id: string;
  segment_name: string;
  description?: string | null;
  match_type: "All" | "Any";
  contact_count?: number | null;
  mailable_count?: number | null;
  counted_at?: string | null;
}

interface GrowthRow {
  day: string;
  subscribed: number;
  unsubscribed: number;
  bounced: number;
  complained: number;
}

/** The people, by what their consent says, and the list that blocks addresses whatever it says. */
function useStanding() {
  return useQuery({
    queryKey: ["campaigns", "standing"],
    queryFn: async () => {
      const by = (s: string) => countOf("MarketingContact", [["consent_status", "=", s]]);
      const [subscribed, pending, unsubscribed, bounced, complained, suppressed] = await Promise.all([by("Subscribed"), by("Pending"), by("Unsubscribed"), by("Bounced"), by("Complained"), countOf("EmailSuppression")]);
      return { subscribed, pending, unsubscribed, away: bounced + complained, suppressed };
    },
  });
}

function Standing() {
  const q = useStanding();
  const items = q.data
    ? [
        { id: "Subscribed", label: tr("Subscribed"), value: q.data.subscribed, to: "/e/MarketingContact?f.consent_status=Subscribed", note: "Emailed, unless they are on the suppression list." },
        { id: "Pending", label: tr("Not confirmed yet"), value: q.data.pending, to: "/e/MarketingContact?f.consent_status=Pending", note: "Signed up, waiting to confirm their address." },
        { id: "Unsubscribed", label: tr("Unsubscribed"), value: q.data.unsubscribed, to: "/e/MarketingContact?f.consent_status=Unsubscribed", note: "Opted out. Only they can opt back in." },
        { id: "away", label: tr("Bounced or complained"), value: q.data.away, to: "/e/MarketingContact?f.consent_status=Bounced", note: "Never emailed again." },
        { id: "suppressed", label: tr("On the suppression list"), value: q.data.suppressed, to: "/e/EmailSuppression", note: "Addresses that are never emailed." },
      ]
    : [];
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} />;
  if (!q.data) return <PageLoading label={tr("Counting contacts")} />;
  return (
    <dl className="cm-tiles">
      {items.map((t) => (
        <div key={t.id} className="cm-tile">
          <dt>{isolate(t.label)}</dt>
          <dd className="cm-tile-value">
            <Link to={t.to}>{isolate(count(t.value))}</Link>
          </dd>
          <dd className="cm-tile-note faint">{isolate(t.note)}</dd>
        </div>
      ))}
    </dl>
  );
}

function Growth() {
  const q = useQuery({ queryKey: ["campaigns", "growth"], queryFn: () => api.get<{ rows: GrowthRow[] }>("/api/reports/audience_growth") });
  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title={tr("The audience's growth could not be loaded")} />;
  if (!q.data) return <PageLoading label={tr("Loading the last 30 days")} />;
  const rows = q.data.rows;
  const joined = rows.map((r) => r.subscribed);
  const left = rows.map((r) => r.unsubscribed + r.bounced + r.complained);
  const sum = (v: number[]) => v.reduce((a, b) => a + b, 0);
  const net = sum(joined) - sum(left);
  return (
    <section className="card cm-section" aria-labelledby="cm-growth">
      <header className="card-head">
        <h2 id="cm-growth">{tr("The last 30 days")}</h2>
        <div className="cm-aside faint">
          {isolate(noun(sum(joined), "person", "people"))} {tr("joined,")} {tr("{value} left:", { value: noun(sum(left), "person", "people") })} {net >= 0 ? "+" : "−"}
          {isolate(count(Math.abs(net)))}
        </div>
      </header>
      <div className="card-body">
        <LineChart
          title={tr("People joining and leaving each day")}
          at={rows.map((r) => r.day)}
          interval="day"
          series={[
            { id: "joined", label: tr("Joined"), color: "var(--chart-1)", values: joined },
            { id: "left", label: tr("Unsubscribed, bounced or complained"), color: "var(--chart-5)", values: left },
          ]}
          empty="No one has joined or left in the last 30 days."
        />
      </div>
    </section>
  );
}

function NewSegmentDialog({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const make = useMutation({
    mutationFn: () => api.post<{ id: string }>("/api/v1/MarketingSegment", { segment_name: name.trim(), description: description.trim() || undefined, match_type: "All", rules: [] }),
    onSuccess: (made) => navigate(`/s/${made.id}`, { replace: true }),
  });
  const taken = make.error instanceof ApiError ? (make.error.fields.find((f) => f.field === "segment_name")?.message ?? make.error.message) : undefined;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (name.trim()) make.mutate();
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={tr("New segment")}
      footer={
        <>
          <Button onClick={onClose}>{tr("Cancel")}</Button>
          <Button type="submit" form="cm-new-segment" variant="primary" loading={make.isPending} disabled={!name.trim()}>
            {tr("Create and add rules")}
          </Button>
        </>
      }
    >
      <form id="cm-new-segment" className="stack" onSubmit={submit}>
        <Field label={tr("Name")} htmlFor="cm-seg-name" required error={taken} hint={tr("For example: Pune customers who opened something this month.")}>
          <input id="cm-seg-name" className="input" value={name} maxLength={140} data-autofocus onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label={tr("What it is for")} htmlFor="cm-seg-desc" hint={tr("Optional.")}>
          <textarea id="cm-seg-desc" className="textarea" rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </Field>
        <p className="faint">{tr("You choose who is in it on the next page, and see the count as you go.")}</p>
      </form>
    </Dialog>
  );
}

export function Audiences({ creating = false }: { creating?: boolean }) {
  useShellTitle(tr("Audiences"));
  const qc = useQueryClient();
  const navigate = useNavigate();
  const meta = useEntityMeta("MarketingSegment").data;
  const contacts = useEntityMeta("MarketingContact").data;
  const [filter, setFilter] = useState("");
  const [making, setMaking] = useState(creating);
  const segments = useQuery({
    queryKey: keys.segments,
    queryFn: ({ signal }) => list<Row>("MarketingSegment", { fields: "segment_name,description,match_type,contact_count,mailable_count,counted_at", sort: "segment_name asc", limit: 200 }, signal),
  });
  useLiveEntity("MarketingSegment", useCallback(() => void qc.invalidateQueries({ queryKey: keys.segments }), [qc]));
  const rows = useMemo(() => (segments.data?.rows ?? []).filter((r) => !filter.trim() || r.segment_name.toLowerCase().includes(filter.trim().toLowerCase())), [segments.data, filter]);
  const canCreate = !!meta && can(meta.permissions, "create");
  const canImport = !!contacts && can(contacts.permissions, "import");

  return (
    <div className="cm-page">
      <header className="cm-head">
        <div>
          <h1 className="sr-only">{tr("Audiences")}</h1>
          <Nav />
          <p className="page-sub">{tr("Who can be emailed, and the groups you pick from them.")}</p>
        </div>
        <div className="cm-tools">
          {canImport && (
            <Link className="btn btn-default btn-md" to="/import">
              <Icon name="upload" />
              {tr("Import contacts")}
            </Link>
          )}
          {canCreate && (
            <Button variant="primary" icon="plus" onClick={() => setMaking(true)}>
              {tr("New segment")}
            </Button>
          )}
        </div>
      </header>

      <Standing />
      <Growth />

      <section className="card cm-section" aria-labelledby="cm-segments">
        <header className="card-head">
          <h2 id="cm-segments">{tr("Segments")}</h2>
          {(segments.data?.rows.length ?? 0) > 8 && (
            <div className="cm-aside">
              <input className="input cm-search" type="search" placeholder={tr("Find a segment…")} aria-label={tr("Find a segment")} value={filter} onChange={(e) => setFilter(e.target.value)} />
            </div>
          )}
        </header>
        {segments.error ? (
          <ErrorState error={segments.error} onRetry={() => void segments.refetch()} />
        ) : !segments.data ? (
          <PageLoading label={tr("Loading segments")} />
        ) : segments.data.rows.length === 0 ? (
          <EmptyState title={tr("No segments yet")} icon="list">
            <p className="muted">{tr("A segment is a saved question about your contacts, such as “opened something in the last 30 days” or “lives in Pune”. It is worked out afresh whenever a campaign uses it.")}</p>
            {canCreate && (
              <Button variant="primary" icon="plus" onClick={() => setMaking(true)}>
                {tr("New segment")}
              </Button>
            )}
          </EmptyState>
        ) : rows.length === 0 ? (
          <EmptyState title={tr("No segment matches")} icon="search" />
        ) : (
          <ul className="cm-segs">
            {rows.map((r) => (
              <li key={r.id}>
                <div className="cm-seg-main">
                  <Link to={`/s/${r.id}`}>{isolate(r.segment_name)}</Link>
                  {r.description && <span className="muted cm-seg-desc">{isolate(r.description)}</span>}
                </div>
                <dl className="cm-seg-stats">
                  <div>
                    <dt>{tr("People")}</dt>
                    <dd>{r.counted_at ? count(r.contact_count) : "–"}</dd>
                  </div>
                  <div>
                    <dt>{tr("Can be emailed")}</dt>
                    <dd>{r.counted_at ? count(r.mailable_count) : "–"}</dd>
                  </div>
                </dl>
                <span className="faint cm-seg-when">{r.counted_at ? tr("Counted {value}", { value: relativeTime(r.counted_at) }) : tr("Not counted yet")}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {making && (
        <NewSegmentDialog
          onClose={() => {
            setMaking(false);
            if (creating) navigate("/audiences", { replace: true });
          }}
        />
      )}
    </div>
  );
}

/** `/s/new`: the segments page with the dialog to name a new one open. */
export function NewSegment() {
  return <Audiences creating />;
}

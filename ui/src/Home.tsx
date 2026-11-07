// Campaigns, newest first: what each is doing and how it did, with a way to write a new one, copy
// an old one, and find one by its status or its words. The filters live in the address, so a link
// to "everything that is sending" works.

import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { api, docPath, download, errorMessage, relativeTime, t } from "@ferrum/client";
import { Badge, Button, confirmDialog, EmptyState, ErrorState, Icon, Menu, MenuItem, PageLoading, toast, useDebounced, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta } from "@ferrum/records";
import { type Campaign, clickRate, hasResults, isStatus, LIST_FIELDS, listFilters, openRate, progress, STATUS_FILTERS, tone, whenLine } from "./campaign";
import { duplicateCampaign, keys, list } from "./data";
import { count, noun, percent } from "./format";
import { Nav } from "./Nav";
import { Bar } from "./ui";

type Row = Pick<Campaign, "id" | "campaign_name" | "status" | "subject" | "number" | "docstatus" | "version" | "schedule_at" | "started_at" | "finished_at" | "updated_at" | "total_recipients" | "sent_count" | "bounce_count" | "unique_opens" | "unique_clicks" | "unsubscribe_count" | "ab_test">;

const PAGE = 30;

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function Item({ row, canCreate, canDelete, onDuplicate, onDelete }: { row: Row; canCreate: boolean; canDelete: boolean; onDuplicate: (r: Row) => void; onDelete: (r: Row) => void }) {
  const navigate = useNavigate();
  const sending = row.status === "Sending";
  const pct = progress(row);
  return (
    <li className="cm-item">
      <div className="cm-item-main">
        <div className="cm-item-title">
          <Link to={`/c/${row.id}`}>{row.campaign_name}</Link>
          <Badge tone={tone(row.status)}>{row.status}</Badge>
          {row.ab_test && <Badge tone="purple">{t("A/B test")}</Badge>}
        </div>
        <div className="cm-item-subject muted">{row.subject}</div>
        <div className="cm-item-when faint">
          {[row.status === "Draft" && row.updated_at ? `Edited ${relativeTime(row.updated_at)}` : whenLine(row), row.number].filter(Boolean).join(" · ")}
        </div>
      </div>
      {hasResults(row) && (
        <dl className="cm-item-stats">
          <Stat label={t("Sent")} value={sending && row.total_recipients ? `${count(row.sent_count)} of ${count(row.total_recipients)}` : count(row.sent_count)} />
          <Stat label={t("Opened")} value={percent(openRate(row))} />
          <Stat label={t("Clicked")} value={percent(clickRate(row))} />
        </dl>
      )}
      {sending && <Bar value={pct} label={`${row.campaign_name}: ${percent(pct)} sent`} />}
      <Menu ariaLabel={`More for ${row.campaign_name}`} icon="chevronDown" buttonClass="icon-btn">
        {(close) => (
          <>
            <MenuItem
              onSelect={() => {
                close();
                navigate(`/c/${row.id}`);
              }}
            >
              {t("Open")}
            </MenuItem>
            {canCreate && (
              <MenuItem
                onSelect={() => {
                  close();
                  onDuplicate(row);
                }}
              >
                {t("Duplicate")}
              </MenuItem>
            )}
            {canDelete && row.docstatus === 0 && (
              <MenuItem
                danger
                onSelect={() => {
                  close();
                  onDelete(row);
                }}
              >
                {t("Delete draft…")}
              </MenuItem>
            )}
          </>
        )}
      </Menu>
    </li>
  );
}

export function Home() {
  useShellTitle("Campaigns");
  const qc = useQueryClient();
  const navigate = useNavigate();
  const meta = useEntityMeta("EmailCampaign").data;
  const [sp, setSp] = useSearchParams();
  const status = isStatus(sp.get("status")) ? sp.get("status")! : "";
  const q = sp.get("q") ?? "";
  const [typed, setTyped] = useState(q);
  const search = useDebounced(typed.trim(), 300);

  // What was typed goes into the address a moment later.
  useEffect(() => {
    if (search === q) return;
    setSp(
      (p) => {
        const next = new URLSearchParams(p);
        if (search) next.set("q", search);
        else next.delete("q");
        return next;
      },
      { replace: true },
    );
  }, [search, q, setSp]);

  const result = useInfiniteQuery({
    queryKey: [...keys.list, status, q],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      list<Row>("EmailCampaign", { fields: LIST_FIELDS, sort: "created_at desc", limit: PAGE, count: true, filters: listFilters(status), search: q || undefined, cursor: pageParam }, signal),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  useLiveEntity("EmailCampaign", useCallback(() => void qc.invalidateQueries({ queryKey: keys.list }), [qc]));

  const rows = result.data?.pages.flatMap((p) => p.rows) ?? [];
  const total = result.data?.pages[0]?.total ?? rows.length;
  const canCreate = !!meta && can(meta.permissions, "create");
  const canDelete = !!meta && can(meta.permissions, "delete");
  const filtered = !!status || !!q;

  const setStatus = (id: string) =>
    setSp(
      (p) => {
        const next = new URLSearchParams(p);
        if (id) next.set("status", id);
        else next.delete("status");
        return next;
      },
      { replace: true },
    );

  const duplicate = async (row: Row) => {
    try {
      const made = await duplicateCampaign(row.id);
      toast(t("Copied. It is a draft: change what you need."));
      navigate(`/c/${made.id}`);
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const remove = async (row: Row) => {
    const ok = await confirmDialog({ title: `Delete “${row.campaign_name}”?`, message: "The draft is deleted for good. Nothing was sent.", confirmLabel: "Delete draft", danger: true });
    if (!ok) return;
    try {
      await api.del(docPath("EmailCampaign", row.id), { ifMatch: row.version });
      toast(t("Draft deleted"));
      void qc.invalidateQueries({ queryKey: keys.list });
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };
  const exportCsv = () => download("/api/v1/EmailCampaign/_export", "campaigns.csv", { filters: listFilters(status), search: q || undefined }).catch((e) => toast(errorMessage(e), "error"));

  const body = result.error ? (
    <ErrorState error={result.error} onRetry={() => void result.refetch()} />
  ) : !result.data ? (
    <PageLoading label={t("Loading campaigns")} />
  ) : rows.length === 0 ? (
    filtered ? (
      <EmptyState title={t("No campaign matches")} icon="search">
        <Button
          onClick={() => {
            setTyped("");
            setSp({}, { replace: true });
          }}
        >
          {t("Show every campaign")}
        </Button>
      </EmptyState>
    ) : (
      <EmptyState title={t("No campaigns yet")} icon="mail">
        <p className="muted">{t("Write an email, check who it will reach, and send it. Everything you need is on one page.")}</p>
        {canCreate && (
          <Link className="btn btn-primary btn-md" to="/new">
            <Icon name="plus" />
            {t("New campaign")}
          </Link>
        )}
      </EmptyState>
    )
  ) : (
    <>
      <ul className="cm-items" aria-label={t("Campaigns")}>
        {rows.map((r) => (
          <Item key={r.id} row={r} canCreate={canCreate} canDelete={canDelete} onDuplicate={(x) => void duplicate(x)} onDelete={(x) => void remove(x)} />
        ))}
      </ul>
      {result.hasNextPage && (
        <div className="cm-more">
          <Button onClick={() => void result.fetchNextPage()} loading={result.isFetchingNextPage}>
            {t("Show more")}
          </Button>
        </div>
      )}
    </>
  );

  return (
    <div className="cm-page">
      <header className="cm-head">
        <div>
          <h1 className="sr-only">{t("Campaigns")}</h1>
          <Nav />
          <p className="page-sub" aria-live="polite">
            {result.data ? (filtered ? `${noun(total, "campaign")} found` : noun(total, "campaign")) : " "}
          </p>
        </div>
        <div className="cm-tools">
          <Link className="btn btn-default btn-md" to="/records/EmailCampaign">
            <Icon name="list" />
            {t("Table")}
          </Link>
          {meta && can(meta.permissions, "export") && rows.length > 0 && (
            <Button icon="download" onClick={() => void exportCsv()}>
              {t("Export")}
            </Button>
          )}
          {canCreate && (
            <Link className="btn btn-primary btn-md" to="/new">
              <Icon name="plus" />
              {t("New campaign")}
            </Link>
          )}
        </div>
      </header>
      <div className="cm-filters">
        <div className="segmented" role="group" aria-label={t("Show campaigns that are")}>
          {STATUS_FILTERS.map((s) => (
            <button key={s.id || "all"} type="button" aria-pressed={status === s.id} onClick={() => setStatus(s.id)}>
              {s.label}
            </button>
          ))}
        </div>
        <input className="input cm-search" type="search" placeholder={t("Find a campaign…")} aria-label={t("Find a campaign")} value={typed} onChange={(e) => setTyped(e.target.value)} />
      </div>
      {body}
    </div>
  );
}

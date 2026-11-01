// The fourth tab, and the first one of a campaign that has gone out: one place to judge a send. The
// headline numbers, each with what it is a share of; then what happened hour by hour (emails sent,
// opened and clicked, unsubscribes and bounces) as charts, the A/B test version against version,
// and which links people used. It refreshes every minute while the campaign is sending.

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { Alert, Badge, Button, EmptyState, ErrorState, PageLoading } from "@ferrum/kit";
import { localBucket } from "./chart";
import type { Campaign } from "./campaign";
import { abResultsOf, linksOf, performanceOf, timelineOf } from "./data";
import { count, noun, percent, rate } from "./format";
import { LineChart } from "./LineChart";
import { abBars, abStanding, hasAny, host, linkShares, shape, tiles, variantRates } from "./results";
import { Bar, Section } from "./ui";

/** Variants keep their colour: A is always the first, whatever the filter. */
const VARIANT_COLORS = ["var(--chart-1)", "var(--chart-3)", "var(--chart-4)", "var(--chart-2)", "var(--chart-5)"];
const MINUTE = 60_000;

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

function Tiles({ items }: { items: ReturnType<typeof tiles> }) {
  return (
    <dl className="cm-tiles">
      {items.map((t) => (
        <div key={t.id} className="cm-tile">
          <dt>{t.label}</dt>
          <dd className="cm-tile-value">{t.value}</dd>
          {t.sub && <dd className="cm-tile-sub muted">{t.sub}</dd>}
          {t.note && <dd className="cm-tile-note faint">{t.note}</dd>}
        </div>
      ))}
    </dl>
  );
}

function AbCard({ doc, interval }: { doc: Campaign; interval: "auto" | "hour" | "day" }) {
  const sending = doc.status === "Sending";
  const metric = doc.winner_metric === "Clicks" ? "Clicks" : "Opens";
  const ab = useQuery({ queryKey: ["campaigns", "ab", doc.id], queryFn: () => abResultsOf(doc.id), refetchInterval: sending ? MINUTE : false });
  const tl = useQuery({ queryKey: ["campaigns", "timeline", doc.id, interval], queryFn: () => timelineOf(doc.id, interval), placeholderData: keepPreviousData });
  const bars = abBars(ab.data?.rows ?? [], metric);
  const top = Math.max(0, ...bars.map((b) => b.rate ?? 0));
  const lines = tl.data ? variantRates(tl.data, metric === "Clicks" ? "clicks" : "opens") : [];
  const at = tl.data?.at ?? [];
  const chosen = doc.winner_chosen_at && tl.data ? at.indexOf(localBucket(doc.winner_chosen_at, tl.data.timezone, tl.data.interval)) : -1;
  const letters = ["A", "B", "C", "D", "E"];
  const colour = (variant: string) => VARIANT_COLORS[Math.max(0, letters.indexOf(variant))] ?? VARIANT_COLORS[0];

  return (
    <Section title="A/B test" id="cm-ab-results" aside={doc.winner_variant ? <Badge tone="green">Version {doc.winner_variant} won</Badge> : <Badge tone="blue">Deciding</Badge>}>
      <p className="muted">
        {abStanding(doc)} The winner is the version with the most {metric.toLowerCase()} for the emails that arrived, {doc.test_percent ?? 20}% of the audience got the versions first.
      </p>
      {ab.error ? (
        <ErrorState error={ab.error} onRetry={() => void ab.refetch()} />
      ) : !ab.data ? (
        <PageLoading label="Loading the test" />
      ) : (
        <ul className="cm-hbars" aria-label={`${metric} rate of each version`}>
          {bars.map((b) => (
            <li key={b.variant}>
              <span className="cm-letter" aria-hidden="true">
                {b.variant}
              </span>
              <div className="cm-hbar-main">
                <div className="cm-hbar-label">
                  <span className="cm-hbar-subject">{b.subject || `Version ${b.variant}`}</span>
                  {b.winner && <Badge tone="green">Winner</Badge>}
                </div>
                <div className="cm-hbar-track" aria-hidden="true">
                  <span style={{ width: `${top > 0 ? ((b.rate ?? 0) / top) * 100 : 0}%`, background: colour(b.variant) }} />
                </div>
              </div>
              <div className="cm-hbar-figure">
                <strong>{percent(b.rate)}</strong>
                <span className="faint">
                  {count(b.hits)} of {count(b.delivered)}
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
      {lines.length > 1 && (
        <LineChart
          title={`${metric === "Clicks" ? "Click" : "Open"} rate of each version as the send went on`}
          at={at}
          interval={tl.data!.interval}
          integers={false}
          format={(n) => percent(n)}
          tick={(n) => `${Math.round(n)}%`}
          series={lines.map((l) => ({ id: l.variant, label: `Version ${l.variant}`, color: colour(l.variant), values: l.values }))}
          marker={chosen >= 0 ? { index: chosen, label: "Winner chosen" } : undefined}
          stale={tl.isPlaceholderData}
          empty="Nothing yet."
        />
      )}
    </Section>
  );
}

export function ResultsTab({ doc }: { doc: Campaign }) {
  const sending = doc.status === "Sending";
  const [interval, setInterval] = useState<"auto" | "hour" | "day">("auto");
  const [running, setRunning] = useState(true);
  const perf = useQuery({ queryKey: ["campaigns", "perf", doc.id, doc.version], queryFn: () => performanceOf(doc.id), refetchInterval: sending ? MINUTE : false });
  const tl = useQuery({
    queryKey: ["campaigns", "timeline", doc.id, interval],
    queryFn: () => timelineOf(doc.id, interval),
    refetchInterval: sending ? MINUTE : false,
    placeholderData: keepPreviousData,
  });
  const links = useQuery({ queryKey: ["campaigns", "links", doc.id], queryFn: () => linksOf(doc.id), enabled: !!doc.track_clicks, refetchInterval: sending ? MINUTE : false });
  const [all, setAll] = useState(false);

  const row = perf.data?.rows[0];
  if (perf.error) return <ErrorState error={perf.error} onRetry={() => void perf.refetch()} />;
  if (!perf.data) return <PageLoading label="Loading the results" />;
  if (!row) {
    return (
      <EmptyState title="No results yet" icon="chart">
        <p className="muted">{doc.status === "Draft" ? "Results appear once the campaign has been sent." : "They appear as soon as it starts sending, within a minute of its time."}</p>
      </EmptyState>
    );
  }

  const data = tl.data;
  const per = data?.interval === "day" ? "day" : "hour";
  const s = data?.series;
  const refresh = () => void Promise.all([perf.refetch(), tl.refetch(), links.refetch()]);
  const shares = linkShares(links.data?.rows ?? [], row.delivered);
  const shownLinks = all ? shares : shares.slice(0, 8);
  const rare = s ? hasAny(s.unsubscribes) || hasAny(s.bounces) || hasAny(s.complaints) : false;
  const stale = tl.isPlaceholderData || tl.isFetching;

  return (
    <div className="cm-stack">
      {doc.last_error && (
        <Alert tone="error">
          <strong>Sending ran into a problem.</strong> {doc.last_error}
        </Alert>
      )}
      <div className="cm-results-bar">
        <span className="faint" role="status">
          {sending ? "Sending: " : ""}updated {time(Math.max(perf.dataUpdatedAt, tl.dataUpdatedAt))}
          {sending && ", and again every minute"}
        </span>
        <Button size="sm" icon="refresh" loading={perf.isFetching} onClick={refresh}>
          Refresh
        </Button>
        {doc.status === "Cancelled" && <Badge tone="red">Stopped before everyone was sent to</Badge>}
      </div>

      <Tiles items={tiles(row)} />

      <Section
        title="What happened, and when"
        id="cm-timeline"
        aside={
          <div className="row">
            <div className="segmented" role="group" aria-label="Group by">
              <button type="button" aria-pressed={data?.interval === "hour"} onClick={() => setInterval("hour")}>
                Hour by hour
              </button>
              <button type="button" aria-pressed={data?.interval === "day"} onClick={() => setInterval("day")}>
                Day by day
              </button>
            </div>
          </div>
        }
      >
        {tl.error && !data ? (
          <ErrorState error={tl.error} onRetry={() => void tl.refetch()} />
        ) : !data ? (
          <PageLoading label="Loading the timeline" />
        ) : (
          <div className="cm-charts">
            <LineChart
              title={`Emails sent each ${per}`}
              at={data.at}
              interval={data.interval}
              kind="columns"
              series={[{ id: "sent", label: "Sent", color: "var(--chart-8)", values: data.series.sent }]}
              empty="Nothing has gone out yet."
              stale={stale}
            />
            <div className="row cm-chart-mode">
              <span className="field-label">Opens, clicks and the rest as</span>
              <div className="segmented" role="group" aria-label="Show">
                <button type="button" aria-pressed={!running} onClick={() => setRunning(false)}>
                  Each {per}
                </button>
                <button type="button" aria-pressed={running} onClick={() => setRunning(true)}>
                  Running total
                </button>
              </div>
            </div>
            <LineChart
              title="Opened and clicked"
              at={data.at}
              interval={data.interval}
              series={[
                { id: "opens", label: "Opened", color: "var(--chart-1)", values: shape(data.series.opens, running) },
                { id: "clicks", label: "Clicked", color: "var(--chart-3)", values: shape(data.series.clicks, running) },
              ]}
              empty="Nobody has opened it yet."
              stale={stale}
            />
            {rare ? (
              <LineChart
                title="Unsubscribes, bounces and spam complaints"
                at={data.at}
                interval={data.interval}
                series={[
                  { id: "unsubscribes", label: "Unsubscribed", color: "var(--chart-4)", values: shape(data.series.unsubscribes, running) },
                  { id: "bounces", label: "Bounced", color: "var(--chart-5)", values: shape(data.series.bounces, running) },
                  { id: "complaints", label: "Spam complaints", color: "var(--chart-2)", values: shape(data.series.complaints, running) },
                ]}
                empty=""
                stale={stale}
              />
            ) : (
              <p className="muted cm-chart-calm">No unsubscribes, bounces or spam complaints so far.</p>
            )}
            <p className="faint cm-note">
              Times are in the workspace's time zone ({data.timezone}). An open is counted when the email's picture loads, once for each person. Opens by privacy proxies and scanners are not counted, so opens run lower than in tools that count them.
            </p>
          </div>
        )}
      </Section>

      {doc.ab_test && <AbCard doc={doc} interval={interval} />}

      <Section title="Links" id="cm-links" aside={doc.track_clicks ? <span className="faint">Share of the {noun(row.delivered, "email")} that arrived</span> : undefined}>
        {!doc.track_clicks ? (
          <p className="muted">Clicks were not counted for this campaign.</p>
        ) : links.error ? (
          <ErrorState error={links.error} onRetry={() => void links.refetch()} />
        ) : !links.data ? (
          <PageLoading label="Loading links" />
        ) : shares.length === 0 ? (
          <p className="muted">The content has no links that can be counted.</p>
        ) : (
          <>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Link</th>
                    <th scope="col" className="num">
                      People who clicked
                    </th>
                    <th scope="col">Share</th>
                    <th scope="col" className="num">
                      Clicks in all
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {shownLinks.map((l) => (
                    <tr key={l.link_no}>
                      <td className="cm-link-cell">
                        <span title={l.url}>{host(l.url)}</span>
                        <span className="faint cm-link-path">{l.url.replace(/^https?:\/\/[^/]+/, "") || "/"}</span>
                      </td>
                      <td className="num">{count(l.unique_clicks)}</td>
                      <td className="cm-share">
                        <Bar value={l.relative * 100} label={`${l.url}: ${percent(l.share)} of delivered`} />
                        <span>{percent(l.share)}</span>
                      </td>
                      <td className="num">{count(l.total_clicks)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {shares.length > shownLinks.length && (
              <div className="cm-more">
                <Button size="sm" onClick={() => setAll(true)}>
                  Show all {shares.length} links
                </Button>
              </div>
            )}
          </>
        )}
      </Section>

      <Section title="Who it went to" id="cm-people">
        <p className="muted">
          {noun(row.recipients, "person", "people")} {row.recipients === 1 ? "was" : "were"} chosen when it started:{" "}
          {[
            `${count(row.sent)} sent`,
            row.failed > 0 ? `${count(row.failed)} failed` : "",
            row.skipped > 0 ? `${count(row.skipped)} skipped, because they had unsubscribed or been blocked since` : "",
          ]
            .filter(Boolean)
            .join(", ")}
          .
        </p>
        <div className="row">
          <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}`}>
            See every recipient
          </Link>
          {row.failed > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.status=Failed`}>
              Failed ({count(row.failed)})
            </Link>
          )}
          {row.skipped > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.status=Skipped`}>
              Skipped ({count(row.skipped)})
            </Link>
          )}
          {row.bounces > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.bounce_type=Hard`}>
              Hard bounces ({count(row.hard_bounces)})
            </Link>
          )}
        </div>
        {rate(row.sent, row.recipients) !== null && rate(row.sent, row.recipients)! < 100 && sending && <Bar value={rate(row.sent, row.recipients)} label="Sent so far" />}
      </Section>
    </div>
  );
}

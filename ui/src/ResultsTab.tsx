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
import { clock, count, percent, rate } from "./format";
import { LineChart } from "./LineChart";
import { abBars, abStanding, hasAny, host, linkShares, shape, tiles, variantRates } from "./results";
import { Bar, Section } from "./ui";
import { t as tr, isolate } from "@ferrum/client";

/** Variants keep their colour: A is always the first, whatever the filter. */
const VARIANT_COLORS = ["var(--chart-1)", "var(--chart-3)", "var(--chart-4)", "var(--chart-2)", "var(--chart-5)"];
const MINUTE = 60_000;


function Tiles({ items }: { items: ReturnType<typeof tiles> }) {
  return (
    <dl className="cm-tiles">
      {items.map((t) => (
        <div key={t.id} className="cm-tile">
          <dt>{isolate(t.label)}</dt>
          <dd className="cm-tile-value">{isolate(t.value)}</dd>
          {t.sub && <dd className="cm-tile-sub muted">{isolate(t.sub)}</dd>}
          {t.note && <dd className="cm-tile-note faint">{isolate(t.note)}</dd>}
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
    <Section title={tr("A/B test")} id="cm-ab-results" aside={doc.winner_variant ? <Badge tone="green">{tr("Version {winner_variant} won", { winner_variant: doc.winner_variant })}</Badge> : <Badge tone="blue">{tr("Deciding")}</Badge>}>
      <p className="muted">
        {isolate(abStanding(doc))} {tr("The winner is the version with the most")} {isolate(metric.toLowerCase())} {tr("for the emails that arrived,")} {doc.test_percent ?? 20}{tr("% of the audience got the versions first.")}
      </p>
      {ab.error ? (
        <ErrorState error={ab.error} onRetry={() => void ab.refetch()} />
      ) : !ab.data ? (
        <PageLoading label={tr("Loading the test")} />
      ) : (
        <ul className="cm-hbars" aria-label={tr("{metric} rate of each version", { metric })}>
          {bars.map((b) => (
            <li key={b.variant}>
              <span className="cm-letter" aria-hidden="true">
                {isolate(b.variant)}
              </span>
              <div className="cm-hbar-main">
                <div className="cm-hbar-label">
                  <span className="cm-hbar-subject">{b.subject || tr("Version {variant}", { variant: b.variant })}</span>
                  {b.winner && <Badge tone="green">{tr("Winner")}</Badge>}
                </div>
                <div className="cm-hbar-track" aria-hidden="true">
                  <span style={{ width: `${top > 0 ? ((b.rate ?? 0) / top) * 100 : 0}%`, background: colour(b.variant) }} />
                </div>
              </div>
              <div className="cm-hbar-figure">
                <strong>{isolate(percent(b.rate))}</strong>
                <span className="faint">
                  {tr("{value} of {value2}", { value: count(b.hits), value2: count(b.delivered) })}
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
      {lines.length > 1 && (
        <LineChart
          title={tr("{value} rate of each version as the send went on", { value: metric === "Clicks" ? "Click" : "Open" })}
          at={at}
          interval={tl.data!.interval}
          integers={false}
          format={(n) => percent(n)}
          tick={(n) => `${Math.round(n)}%`}
          series={lines.map((l) => ({ id: l.variant, label: tr("Version {variant}", { variant: l.variant }), color: colour(l.variant), values: l.values }))}
          marker={chosen >= 0 ? { index: chosen, label: tr("Winner chosen") } : undefined}
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
  if (!perf.data) return <PageLoading label={tr("Loading the results")} />;
  if (!row) {
    return (
      <EmptyState title={tr("No results yet")} icon="chart">
        <p className="muted">{doc.status === "Draft" ? tr("Results appear once the campaign has been sent.") : tr("They appear as soon as it starts sending, within a minute of its time.")}</p>
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
          <strong>{tr("Sending ran into a problem.")}</strong> {isolate(doc.last_error)}
        </Alert>
      )}
      <div className="cm-results-bar">
        <span className="faint" role="status">
          {sending ? tr("Sending: ") : ""}{tr("updated {value}", { value: clock(Math.max(perf.dataUpdatedAt, tl.dataUpdatedAt)) })}
          {sending && tr(", and again every minute")}
        </span>
        <Button size="sm" icon="refresh" loading={perf.isFetching} onClick={refresh}>
          {tr("Refresh")}
        </Button>
        {doc.status === "Cancelled" && <Badge tone="red">{tr("Stopped before everyone was sent to")}</Badge>}
      </div>

      <Tiles items={tiles(row)} />

      <Section
        title={tr("What happened, and when")}
        id="cm-timeline"
        aside={
          <div className="row">
            <div className="segmented" role="group" aria-label={tr("Group by")}>
              <button type="button" aria-pressed={data?.interval === "hour"} onClick={() => setInterval("hour")}>
                {tr("Hour by hour")}
              </button>
              <button type="button" aria-pressed={data?.interval === "day"} onClick={() => setInterval("day")}>
                {tr("Day by day")}
              </button>
            </div>
          </div>
        }
      >
        {tl.error && !data ? (
          <ErrorState error={tl.error} onRetry={() => void tl.refetch()} />
        ) : !data ? (
          <PageLoading label={tr("Loading the timeline")} />
        ) : (
          <div className="cm-charts">
            <LineChart
              title={tr("Emails sent each {per}", { per })}
              at={data.at}
              interval={data.interval}
              kind="columns"
              series={[{ id: "sent", label: tr("Sent"), color: "var(--chart-8)", values: data.series.sent }]}
              empty="Nothing has gone out yet."
              stale={stale}
            />
            <div className="row cm-chart-mode">
              <span className="field-label">{tr("Opens, clicks and the rest as")}</span>
              <div className="segmented" role="group" aria-label={tr("Show")}>
                <button type="button" aria-pressed={!running} onClick={() => setRunning(false)}>
                  {tr("Each {per}", { per })}
                </button>
                <button type="button" aria-pressed={running} onClick={() => setRunning(true)}>
                  {tr("Running total")}
                </button>
              </div>
            </div>
            <LineChart
              title={tr("Opened and clicked")}
              at={data.at}
              interval={data.interval}
              series={[
                { id: "opens", label: tr("Opened"), color: "var(--chart-1)", values: shape(data.series.opens, running) },
                { id: "clicks", label: tr("Clicked"), color: "var(--chart-3)", values: shape(data.series.clicks, running) },
              ]}
              empty="Nobody has opened it yet."
              stale={stale}
            />
            {rare ? (
              <LineChart
                title={tr("Unsubscribes, bounces and spam complaints")}
                at={data.at}
                interval={data.interval}
                series={[
                  { id: "unsubscribes", label: tr("Unsubscribed"), color: "var(--chart-4)", values: shape(data.series.unsubscribes, running) },
                  { id: "bounces", label: tr("Bounced"), color: "var(--chart-5)", values: shape(data.series.bounces, running) },
                  { id: "complaints", label: tr("Spam complaints"), color: "var(--chart-2)", values: shape(data.series.complaints, running) },
                ]}
                empty=""
                stale={stale}
              />
            ) : (
              <p className="muted cm-chart-calm">{tr("No unsubscribes, bounces or spam complaints so far.")}</p>
            )}
            <p className="faint cm-note">
              {tr("Times are in the workspace's time zone ({timezone}). An open is counted when the email's picture loads, once for each person. Opens by privacy proxies and scanners are not counted, so opens run lower than in tools that count them.", { timezone: data.timezone })}
            </p>
          </div>
        )}
      </Section>

      {doc.ab_test && <AbCard doc={doc} interval={interval} />}

      <Section title={tr("Links")} id="cm-links" aside={doc.track_clicks ? <span className="faint">{tr("Share of the")} {isolate((row.delivered === 1 ? tr("1 email") : tr("{n} emails", { n: count(row.delivered) })))} {tr("that arrived")}</span> : undefined}>
        {!doc.track_clicks ? (
          <p className="muted">{tr("Clicks were not counted for this campaign.")}</p>
        ) : links.error ? (
          <ErrorState error={links.error} onRetry={() => void links.refetch()} />
        ) : !links.data ? (
          <PageLoading label={tr("Loading links")} />
        ) : shares.length === 0 ? (
          <p className="muted">{tr("The content has no links that can be counted.")}</p>
        ) : (
          <>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">{tr("Link")}</th>
                    <th scope="col" className="num">
                      {tr("People who clicked")}
                    </th>
                    <th scope="col">{tr("Share")}</th>
                    <th scope="col" className="num">
                      {tr("Clicks in all")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {shownLinks.map((l) => (
                    <tr key={l.link_no}>
                      <td className="cm-link-cell">
                        <span title={l.url}>{isolate(host(l.url))}</span>
                        <span className="faint cm-link-path">{l.url.replace(/^https?:\/\/[^/]+/, "") || "/"}</span>
                      </td>
                      <td className="num">{isolate(count(l.unique_clicks))}</td>
                      <td className="cm-share">
                        <Bar value={l.relative * 100} label={tr("{url}: {value} of delivered", { url: l.url, value: percent(l.share) })} />
                        <span>{isolate(percent(l.share))}</span>
                      </td>
                      <td className="num">{isolate(count(l.total_clicks))}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {shares.length > shownLinks.length && (
              <div className="cm-more">
                <Button size="sm" onClick={() => setAll(true)}>
                  {tr("Show all {length} links", { length: shares.length })}
                </Button>
              </div>
            )}
          </>
        )}
      </Section>

      <Section title={tr("Who it went to")} id="cm-people">
        <p className="muted">
          {isolate((row.recipients === 1 ? tr("1 person") : tr("{n} people", { n: count(row.recipients) })))} {row.recipients === 1 ? tr("was") : tr("were")} {tr("chosen when it started:")}{isolate(" ")}
          {isolate([
            `${count(row.sent)} sent`,
            row.failed > 0 ? `${count(row.failed)} failed` : "",
            row.skipped > 0 ? `${count(row.skipped)} skipped, because they had unsubscribed or been blocked since` : "",
          ]
            .filter(Boolean)
            .join(", "))}
          .
        </p>
        <div className="row">
          <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}`}>
            {tr("See every recipient")}
          </Link>
          {row.failed > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.status=Failed`}>
              {tr("Failed (")}{isolate(count(row.failed))})
            </Link>
          )}
          {row.skipped > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.status=Skipped`}>
              {tr("Skipped (")}{isolate(count(row.skipped))})
            </Link>
          )}
          {row.bounces > 0 && (
            <Link className="btn btn-default btn-md" to={`/e/CampaignRecipient?f.campaign=${doc.id}&f.bounce_type=Hard`}>
              {tr("Hard bounces (")}{isolate(count(row.hard_bounces))})
            </Link>
          )}
        </div>
        {rate(row.sent, row.recipients) !== null && rate(row.sent, row.recipients)! < 100 && sending && <Bar value={rate(row.sent, row.recipients)} label={tr("Sent so far")} />}
      </Section>
    </div>
  );
}

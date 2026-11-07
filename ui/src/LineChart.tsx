// A chart of counts over time, drawn as plain SVG from the tokens (--chart-1 …), so it follows the
// theme with nothing hard-coded. Lines are 2px with an end dot, one series also gets a wash of its
// colour; columns are thin with a rounded top. A pointer (or the arrow keys) finds the nearest
// moment and one tooltip lists every series there; a table of the same numbers is one click away,
// so nothing is readable only by hovering or only by colour.

import { type KeyboardEvent, type PointerEvent, useLayoutEffect, useRef, useState } from "react";
import { areaPath, bucketName, columnPath, type Interval, labelled, linePath, nearest, niceAxis, scaleLinear, tickLabel } from "./chart";
import { compact, count } from "./format";
import { t as tr, isolate } from "@ferrum/client";

export interface ChartSeries {
  id: string;
  label: string;
  /** A CSS colour: a token such as `var(--chart-1)`. */
  color: string;
  values: number[];
}

export interface Marker {
  /** The bucket it falls in. */
  index: number;
  label: string;
}

interface Props {
  title: string;
  at: string[];
  interval: Interval;
  series: ChartSeries[];
  kind?: "line" | "columns";
  /** How a value reads in the tooltip and the table. */
  format?: (n: number) => string;
  /** How a tick on the vertical axis reads. */
  tick?: (n: number) => string;
  /** Counts step by whole numbers; rates do not. */
  integers?: boolean;
  /** What to say when there is nothing to draw. */
  empty: string;
  /** Held at reduced opacity while the next numbers load. */
  stale?: boolean;
  marker?: Marker;
}

function useWidth() {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    setWidth(Math.floor(el.clientWidth));
    const watch = new ResizeObserver(([e]) => setWidth(Math.floor(e.contentRect.width)));
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  return [box, width] as const;
}

export function LineChart({ title, at, interval, series, kind = "line", format = count, tick = compact, integers = true, empty, stale, marker }: Props) {
  const [box, width] = useWidth();
  const [shown, setShown] = useState<number | null>(null);
  const [keys, setKeys] = useState(false);
  const [table, setTable] = useState(false);
  const n = at.length;
  const height = width && width < 480 ? 190 : 220;
  const m = { top: 14, right: kind === "line" && series.length <= 3 ? 56 : 14, bottom: 28, left: 46 };
  const plotW = Math.max(0, width - m.left - m.right);
  const plotH = height - m.top - m.bottom;
  const any = series.some((s) => s.values.some((v) => v > 0));
  const axis = niceAxis(Math.max(0, ...series.flatMap((s) => s.values)), { integers });
  const y = scaleLinear(0, axis.max, m.top + plotH, m.top);
  const base = m.top + plotH;
  const xs = at.map((_, i) => (kind === "columns" ? m.left + (i + 0.5) * (plotW / n) : m.left + (n === 1 ? plotW / 2 : (i / (n - 1)) * plotW)));
  const band = kind === "columns" ? Math.min(24, Math.max(2, plotW / n - 2)) : 0;
  const labels = labelled(n, Math.max(2, Math.floor(plotW / 84)));

  const point = (e: PointerEvent<SVGSVGElement>) => {
    setKeys(false);
    setShown(nearest(xs, e.clientX - e.currentTarget.getBoundingClientRect().left));
  };
  const key = (e: KeyboardEvent<SVGSVGElement>) => {
    const at0 = shown ?? n - 1;
    const next = e.key === "ArrowLeft" ? at0 - 1 : e.key === "ArrowRight" ? at0 + 1 : e.key === "Home" ? 0 : e.key === "End" ? n - 1 : null;
    if (e.key === "Escape") setShown(null);
    if (next === null) return;
    e.preventDefault();
    setKeys(true);
    setShown(Math.max(0, Math.min(n - 1, next)));
  };

  // A value at the end of each line, when the lines leave room for it.
  const ends = kind === "line" && series.length <= 3 ? series.map((s) => ({ s, y: y(s.values[n - 1] ?? 0), text: format(s.values[n - 1] ?? 0) })).sort((a, b) => a.y - b.y) : [];
  const labelled_ends = ends.filter((e, i) => i === 0 || e.y - ends[i - 1].y >= 14);
  const summary = shown === null ? "" : `${bucketName(at[shown], interval)}: ${series.map((s) => `${s.label} ${format(s.values[shown] ?? 0)}`).join(", ")}`;

  return (
    <figure className="cm-chart" data-stale={stale || undefined}>
      <figcaption className="cm-chart-head">
        <h3>{isolate(title)}</h3>
        {series.length > 1 && (
          <ul className="cm-legend" aria-label={tr("Lines")}>
            {series.map((s) => (
              <li key={s.id}>
                <i className={`cm-key ${kind === "columns" ? "cm-key-box" : "cm-key-line"}`} style={{ background: s.color }} aria-hidden="true" />
                {isolate(s.label)}
              </li>
            ))}
          </ul>
        )}
        {any && (
          <button type="button" className="link-btn cm-chart-toggle" aria-pressed={table} onClick={() => setTable(!table)}>
            {table ? tr("Show the chart") : tr("Show as a table")}
          </button>
        )}
      </figcaption>
      <div className="cm-chart-plot" ref={box}>
        {!any ? (
          <p className="cm-chart-empty muted">{isolate(empty)}</p>
        ) : table ? (
          <div className="cm-chart-table">
            <table className="table">
              <caption className="sr-only">{isolate(title)}</caption>
              <thead>
                <tr>
                  <th scope="col">{tr("When")}</th>
                  {series.map((s) => (
                    <th key={s.id} scope="col" className="num">
                      {isolate(s.label)}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {at.map((a, i) => (
                  <tr key={a}>
                    <th scope="row">{isolate(bucketName(a, interval))}</th>
                    {series.map((s) => (
                      <td key={s.id} className="num">
                        {isolate(format(s.values[i] ?? 0))}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          width > 0 && (
            <>
              <svg
                className="cm-svg"
                width={width}
                height={height}
                role="img"
                tabIndex={0}
                aria-label={tr("{title}. Use the left and right arrow keys to read it point by point.", { title })}
                onPointerMove={point}
                onPointerDown={point}
                onPointerLeave={(e) => e.pointerType === "mouse" && setShown(null)}
                onKeyDown={key}
                onFocus={() => setShown((s) => s ?? n - 1)}
                onBlur={() => setShown(null)}
              >
                {axis.ticks.map((t) => (
                  <g key={t}>
                    <line className="cm-grid" x1={m.left} x2={m.left + plotW} y1={y(t)} y2={y(t)} />
                    <text className="cm-axis" x={m.left - 8} y={y(t)} dy="0.32em" textAnchor="end">
                      {isolate(tick(t))}
                    </text>
                  </g>
                ))}
                {labels.map((i) => (
                  <text key={i} className="cm-axis" x={xs[i]} y={height - 8} textAnchor={i === 0 ? "start" : "middle"}>
                    {isolate(tickLabel(at[i], interval, i === 0))}
                  </text>
                ))}
                {marker && marker.index >= 0 && marker.index < n && (
                  <g>
                    <line className="cm-marker" x1={xs[marker.index]} x2={xs[marker.index]} y1={m.top} y2={base} />
                    <text className="cm-axis cm-marker-label" x={xs[marker.index] + 4} y={m.top + 10}>
                      {isolate(marker.label)}
                    </text>
                  </g>
                )}
                {kind === "columns"
                  ? series.map((s) => (
                      <g key={s.id} style={{ fill: s.color }}>
                        {s.values.map((v, i) => (
                          <path key={i} d={columnPath(xs[i] - band / 2, band, y(v), base)} opacity={shown === null || shown === i ? 1 : 0.55} />
                        ))}
                      </g>
                    ))
                  : series.map((s) => (
                      <g key={s.id}>
                        {series.length === 1 && <path d={areaPath(xs, s.values.map(y), base)} style={{ fill: s.color }} opacity={0.1} />}
                        <path d={linePath(xs, s.values.map(y))} fill="none" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" style={{ stroke: s.color }} />
                        <circle cx={xs[n - 1]} cy={y(s.values[n - 1] ?? 0)} r={4} className="cm-dot" style={{ fill: s.color }} />
                      </g>
                    ))}
                {labelled_ends.map((e) => (
                  <text key={e.s.id} className="cm-axis cm-end" x={xs[n - 1] + 10} y={e.y} dy="0.32em">
                    {isolate(e.text)}
                  </text>
                ))}
                {shown !== null && (
                  <g>
                    <line className="cm-cursor" x1={xs[shown]} x2={xs[shown]} y1={m.top} y2={base} />
                    {kind === "line" &&
                      series.map((s) => <circle key={s.id} cx={xs[shown]} cy={y(s.values[shown] ?? 0)} r={4} className="cm-dot" style={{ fill: s.color }} />)}
                  </g>
                )}
              </svg>
              {shown !== null && (
                <div className="cm-tip" aria-hidden="true" style={{ left: xs[shown], top: m.top, transform: xs[shown] > width * 0.6 ? "translateX(calc(-100% - 12px))" : "translateX(12px)" }}>
                  <div className="cm-tip-when">{isolate(bucketName(at[shown], interval))}</div>
                  {series.map((s) => (
                    <div key={s.id} className="cm-tip-row">
                      <i className={`cm-key ${kind === "columns" ? "cm-key-box" : "cm-key-line"}`} style={{ background: s.color }} />
                      <strong>{isolate(format(s.values[shown] ?? 0))}</strong>
                      <span className="muted">{isolate(s.label)}</span>
                    </div>
                  ))}
                </div>
              )}
              <span className="sr-only" role="status" aria-live="polite">
                {keys ? summary : ""}
              </span>
            </>
          )
        )}
      </div>
    </figure>
  );
}

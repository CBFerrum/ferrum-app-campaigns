// What the screens ask the server, in one place: the records API for campaigns and segments, the
// campaign's own actions (preview, test, the audience check, the timeline) and its reports.

import { api, docPath } from "@ferrum/client";
import { type Campaign, duplicateBody } from "./campaign";
import type { AudienceCheck } from "./review";
import type { AbRow, LinkRow, Perf, Timeline } from "./results";
import type { Rule } from "./segment";

export const keys = {
  campaign: (id: string) => ["campaigns", "campaign", id] as const,
  list: ["campaigns", "list"] as const,
  segments: ["campaigns", "segments"] as const,
  segment: (id: string) => ["campaigns", "segment", id] as const,
};

export interface Page<T> {
  rows: T[];
  next_cursor?: string | null;
  total?: number | null;
}

/** A list of records of one kind. */
export const list = <T>(entity: string, query: Record<string, string | number | boolean | undefined>, signal?: AbortSignal) =>
  api.get<Page<T>>(`/api/v1/${entity}`, query, signal);

/** How many records match, without loading them. */
export const countOf = (entity: string, filters?: unknown[]) =>
  list<{ id: string }>(entity, { limit: 1, count: true, fields: "id", filters: filters ? JSON.stringify(filters) : undefined }).then((p) => p.total ?? 0);

/** A campaign as the screens use it: its tables are always lists. */
export const whole = (c: Campaign): Campaign => ({ ...c, audience: c.audience ?? [], variants: c.variants ?? [] });

export const getCampaign = (id: string, signal?: AbortSignal) => api.get<Campaign>(docPath("EmailCampaign", id), { titles: true }, signal).then(whole);

const action = <T>(id: string, name: string, body: unknown = {}) => api.post<T>(docPath("EmailCampaign", id, "actions", name), body);

export interface Preview {
  subject: string;
  html: string;
  text: string;
  variant: string | null;
}

/** The saved email as one person would get it (a stand-in without a contact). */
export const previewOf = (id: string, p: { variant?: string | null; contact?: string | null }) =>
  action<Preview>(id, "preview", { ...(p.variant ? { variant: p.variant } : {}), ...(p.contact ? { contact: p.contact } : {}) });

export const sendTest = (id: string, p: { emails: string[]; variant?: string | null; contact?: string | null }) =>
  action<{ sent: number }>(id, "send_test", { emails: p.emails, ...(p.variant ? { variant: p.variant } : {}), ...(p.contact ? { contact: p.contact } : {}) });

export const checkAudience = (id: string) => action<AudienceCheck>(id, "audience_check");

export const timelineOf = (id: string, interval: "auto" | "hour" | "day") => action<Timeline>(id, "timeline", { interval });

interface Report<R> {
  rows: R[];
  totals?: Record<string, unknown>;
  meta?: Record<string, unknown>;
}

export const performanceOf = (campaign: string) => api.get<Report<Perf & { campaign_id: string }>>("/api/reports/campaign_performance", { campaign });
export const abResultsOf = (campaign: string) => api.get<Report<AbRow>>("/api/reports/ab_test_results", { campaign });
export const linksOf = (campaign: string) => api.get<Report<LinkRow>>("/api/reports/link_clicks", { campaign });

export interface SegmentCount {
  contact_count: number;
  mailable_count: number;
  sample: { id: string; email: string; full_name: string | null; consent_status: string; mailable: boolean }[];
}

/** Counts rules as they are being edited, without saving them. */
export const countSegment = (id: string, match: "All" | "Any", rules: Record<string, unknown>[]) =>
  api.post<SegmentCount>(docPath("MarketingSegment", id, "actions", "count"), { match_type: match, rules });

export interface Segment {
  id: string;
  version: number;
  segment_name: string;
  description?: string | null;
  match_type: "All" | "Any";
  rules: Rule[];
  contact_count?: number | null;
  mailable_count?: number | null;
  counted_at?: string | null;
}

/** A new draft made from a campaign: the same email to the same audience. */
export async function duplicateCampaign(id: string): Promise<Campaign> {
  return api.post<Campaign>("/api/v1/EmailCampaign", duplicateBody(await getCampaign(id)));
}

// Campaigns' own interface (Ferrum docs/plan/apps-ui.md §13) at /campaigns/: campaigns on its home,
// one page to write, preview, test, check and send each of them and then to judge it, audiences
// (segments, and contact files brought in), and every other record in the records frame, with
// Campaigns' modules in the sidebar under the platform's bar. Campaigns and segments open on their
// own pages wherever the frame links to them.

import { useEffect } from "react";
import { Link, useRouteError } from "react-router";
import { shell, t } from "@ferrum/client";
import { StatePanel } from "@ferrum/kit";
import { Problem } from "./ui";
import { mountApp, RequireAuth, setFrameMode } from "@ferrum/records/frame";
import "./styles.css";

/** A page of the records frame, loaded when first opened. */
const page = (name: "FormView" | "ListView" | "ReportView") => () =>
  import("./records").then((m) => ({ Component: m[name] }));

/** Campaigns' own pages, loaded when first opened: the first script holds only the frame. */
const home = () => import("./Home").then((m) => ({ Component: m.Home }));
const start = () => import("./New").then((m) => ({ Component: m.NewCampaign }));
const campaign = () => import("./CampaignPage").then((m) => ({ Component: m.CampaignRoute }));
const audiences = () => import("./Audiences").then((m) => ({ Component: m.Audiences }));
const newSegment = () => import("./Audiences").then((m) => ({ Component: m.NewSegment }));
const segment = () => import("./SegmentPage").then((m) => ({ Component: m.SegmentRoute }));
const importer = () => import("./Import").then((m) => ({ Component: m.Import }));

/** Campaigns' packages: the sidebar shows their modules. */
const PACKAGES = ["campaigns"];

function RouteError() {
  return <Problem error={useRouteError()} onRetry={() => window.location.reload()} />;
}

function NotFound() {
  return (
    <StatePanel kind="empty" title={t("Page not found")} action={<Link to="/">{t("Go to Campaigns")}</Link>} />
  );
}

/** Every page tells the shell it started. */
function Started() {
  useEffect(() => shell().ready(), []);
  return null;
}

setFrameMode("app", PACKAGES);

mountApp({
  route: "campaigns",
  catalogs: { ar: () => import("./locales/ar.json"), hi: () => import("./locales/hi.json") },
  routes: [
    {
      path: "/",
      element: (
        <>
          <Started />
          <RequireAuth />
        </>
      ),
      errorElement: <RouteError />,
      children: [
        { index: true, lazy: home },
        { path: "new", lazy: start },
        { path: "c/:id", lazy: campaign },
        { path: "audiences", lazy: audiences },
        { path: "s/new", lazy: newSegment },
        { path: "s/:id", lazy: segment },
        { path: "import", lazy: importer },
        // The frame's list and form of a campaign or a segment are these pages.
        { path: "e/EmailCampaign", lazy: home },
        { path: "e/EmailCampaign/new", lazy: start },
        { path: "e/EmailCampaign/:id", lazy: campaign },
        { path: "e/MarketingSegment", lazy: audiences },
        { path: "e/MarketingSegment/new", lazy: newSegment },
        { path: "e/MarketingSegment/:id", lazy: segment },
        // The same records in the records frame's own list and form (saved views, bulk actions, every field).
        { path: "records/:entity", lazy: page("ListView") },
        { path: "records/:entity/:id", lazy: page("FormView") },
        { path: "e/:entity", lazy: page("ListView") },
        { path: "e/:entity/:id", lazy: page("FormView") },
        { path: "r/:name", lazy: page("ReportView") },
        { path: "*", element: <NotFound /> },
      ],
    },
  ],
});

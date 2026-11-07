// Campaigns' own screens, at the top of each.

import { NavLink } from "react-router";
import { t } from "@ferrum/client";

export function Nav() {
  return (
    <nav className="cm-tabs" aria-label={t("Campaigns")}>
      <NavLink to="/" end>
        {t("Campaigns")}
      </NavLink>
      <NavLink to="/audiences">{t("Audiences")}</NavLink>
    </nav>
  );
}

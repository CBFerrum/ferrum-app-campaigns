// Campaigns' own screens, at the top of each.

import { NavLink } from "react-router";

export function Nav() {
  return (
    <nav className="cm-tabs" aria-label="Campaigns">
      <NavLink to="/" end>
        Campaigns
      </NavLink>
      <NavLink to="/audiences">Audiences</NavLink>
    </nav>
  );
}

// The audience check of a campaign, counted again whenever what it counts changes: the Review tab
// counts after every save (the whole campaign is being judged), the Setup tab only when the audience is.

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { checkAudience } from "./data";

export const checkKey = (id: string, key: string | number) => ["campaigns", "check", id, key] as const;

/** Who would get the campaign if it started now, and what would stop it; it keeps the last answer on screen while the next is counted. */
export function useAudienceCheck(id: string, key: string | number, enabled = true) {
  return useQuery({
    queryKey: checkKey(id, key),
    queryFn: () => checkAudience(id),
    enabled,
    staleTime: 0,
    refetchOnMount: "always",
    placeholderData: keepPreviousData,
  });
}

/* eslint-disable react-refresh/only-export-components */
import useSWR from "swr";
import { api } from "@/lib/api";

export const ORGS_KEY = "/api/dashboard/orgs";

export function useMyOrgs() {
  return useSWR<import("@/lib/api").OrgsOverview>(ORGS_KEY, () => api.listMyOrgs(), {
    fallbackData: { orgs: [], creation_limit: 2, creation_used: 0, can_create: false },
  });
}

export function OrgAvatar({
  emoji,
  color,
  image,
  size = "size-10",
  text = "text-xl",
}: {
  emoji: string;
  color: string;
  image?: string | null;
  size?: string;
  text?: string;
}) {
  if (image) {
    return <img src={image} alt="" className={`${size} shrink-0 rounded-xl object-cover`} />;
  }
  return (
    <div
      className={`${size} ${text} flex shrink-0 items-center justify-center rounded-xl`}
      style={{ backgroundColor: `${color}22`, color }}
    >
      {emoji}
    </div>
  );
}

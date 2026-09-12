import { useParams } from "react-router-dom";
import { UsageAnalysisPage } from "../usage-analysis";
import { UsageCachePage } from "../usage-cache";

/** The workspace usage page bound to this org's shared keys (ORG-23). */
export function OrgUsagePage() {
  const { orgId } = useParams();
  return <UsageAnalysisPage orgId={orgId} />;
}

/** The workspace cache hit-rate page bound to this org's shared keys (ORG-23). */
export function OrgCachePage() {
  const { orgId } = useParams();
  return <UsageCachePage orgId={orgId} />;
}

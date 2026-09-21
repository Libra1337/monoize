import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { mutate } from "swr";
import { Gauge } from "lucide-react";
import { api, type OrgLimitsResponse, type OrgSpendLimitSet } from "@/lib/api";
import {
  SPEND_WINDOWS,
  nanoToUsdInput,
  usdToNanoLimit,
  type SpendWindowKey,
} from "@/lib/spend-limits";
import { formatCost } from "../request-logs/utils";
import { useMyOrgs } from "./shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

type WindowKey = SpendWindowKey;

const WINDOWS = SPEND_WINDOWS;

function LimitInputs({
  limits,
  labels,
  onChange,
}: {
  limits: OrgSpendLimitSet;
  labels: (key: WindowKey) => string;
  onChange: (window: WindowKey, raw: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      {WINDOWS.map(({ key }) => (
        <label key={key} className="flex items-center gap-1 text-xs text-muted-foreground">
          {labels(key)}
          <Input
            className="h-7 w-24 text-xs"
            placeholder="∞"
            inputMode="decimal"
            value={nanoToUsdInput(limits[key])}
            onChange={(e) => onChange(key, e.target.value)}
          />
        </label>
      ))}
    </div>
  );
}

export function OrgLimitsPage() {
  const { orgId } = useParams();
  const { t } = useTranslation();
  const { data: overview } = useMyOrgs();
  const [data, setData] = useState<OrgLimitsResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [spaceDraft, setSpaceDraft] = useState<OrgSpendLimitSet>({});
  const [memberDrafts, setMemberDrafts] = useState<Record<string, OrgSpendLimitSet>>({});
  const [keyDrafts, setKeyDrafts] = useState<Record<string, OrgSpendLimitSet>>({});

  const role = overview?.orgs?.find((o) => o.id === orgId)?.role;
  const isOwner = role === "owner";

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .getOrgLimits(orgId ?? "")
      .then((res) => {
        if (cancelled) return;
        setData(res);
        setSpaceDraft(res.space.limits);
        const md: Record<string, OrgSpendLimitSet> = {};
        for (const m of res.members) md[m.user_id] = m.limits;
        setMemberDrafts(md);
        const kd: Record<string, OrgSpendLimitSet> = {};
        for (const k of res.keys) kd[k.key_id] = k.limits;
        setKeyDrafts(kd);
        setError(null);
      })
      .catch((e) => !cancelled && setError(String(e?.message ?? e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [orgId]);

  if (!isOwner) {
    return (
      <div className="p-6 text-sm text-muted-foreground">{t("orgLimits.ownerOnly")}</div>
    );
  }
  if (loading) {
    return (
      <div className="space-y-3 p-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-40 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }
  if (error || !data) {
    return <div className="p-6 text-sm text-destructive">{error ?? t("orgLimits.loadFailed")}</div>;
  }

  const windowLabel = (key: WindowKey) =>
    t(WINDOWS.find((w) => w.key === key)?.labelKey ?? "");

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await api.updateOrgLimits(orgId ?? "", spaceDraft, memberDrafts);
      const refreshed = await api.getOrgLimits(orgId ?? "");
      setData(refreshed);
      await mutate(`/api/dashboard/orgs/${orgId}/limits`);
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setSaving(false);
    }
  };

  const saveKeyLimits = async (keyId: string) => {
    setSaving(true);
    try {
      await api.updateOrgKeyLimits(orgId ?? "", keyId, keyDrafts[keyId] ?? {});
      const refreshed = await api.getOrgLimits(orgId ?? "");
      setData(refreshed);
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
    } finally {
      setSaving(false);
    }
  };

  const spentCell = (spent: string) => (
    <span className="font-mono text-xs">{formatCost(spent)}</span>
  );

  return (
    <div className="space-y-6 p-6">
      <div className="flex items-center gap-2">
        <Gauge className="h-5 w-5 text-muted-foreground" />
        <h1 className="text-lg font-semibold tracking-tight">{t("orgLimits.title")}</h1>
      </div>
      {error && <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</div>}

      {/* Space level */}
      <section className="rounded-lg border p-4">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-sm font-semibold">{t("orgLimits.spaceTitle")}</h2>
          <div className="flex gap-3 text-xs text-muted-foreground">
            {WINDOWS.map(({ key }) => (
              <span key={key}>
                {windowLabel(key)}: {spentCell(data.space.spent[key])}
              </span>
            ))}
          </div>
        </div>
        <LimitInputs
          limits={spaceDraft}
          labels={windowLabel}
          onChange={(w, raw) => {
            const nano = usdToNanoLimit(raw);
            if (nano === undefined) return;
            setSpaceDraft((prev) => ({ ...prev, [w]: nano }));
          }}
        />
      </section>

      {/* Member level */}
      <section className="rounded-lg border">
        <div className="border-b px-4 py-3">
          <h2 className="text-sm font-semibold">{t("orgLimits.membersTitle")}</h2>
        </div>
        <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("orgLimits.member")}</TableHead>
              <TableHead>{t("orgLimits.spentTotal")}</TableHead>
              <TableHead>{t("orgLimits.limits")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.members
              .filter((m) => m.role !== "owner")
              .map((m) => (
                <TableRow key={m.user_id}>
                  <TableCell className="font-medium">{m.username ?? m.user_id}</TableCell>
                  <TableCell>{spentCell(m.spent.total_nano_usd)}</TableCell>
                  <TableCell>
                    <LimitInputs
                      limits={memberDrafts[m.user_id] ?? {}}
                      labels={windowLabel}
                      onChange={(w, raw) => {
                        const nano = usdToNanoLimit(raw);
                        if (nano === undefined) return;
                        setMemberDrafts((prev) => ({
                          ...prev,
                          [m.user_id]: { ...prev[m.user_id], [w]: nano },
                        }));
                      }}
                    />
                  </TableCell>
                </TableRow>
              ))}
          </TableBody>
        </Table>
        </div>
      </section>

      {/* Key level */}
      <section className="rounded-lg border">
        <div className="border-b px-4 py-3">
          <h2 className="text-sm font-semibold">{t("orgLimits.keysTitle")}</h2>
        </div>
        <div className="overflow-x-auto">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("orgLimits.keyName")}</TableHead>
              <TableHead>{t("orgLimits.creator")}</TableHead>
              <TableHead>{t("orgLimits.spentTotal")}</TableHead>
              <TableHead>{t("orgLimits.limits")}</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.keys.map((k) => (
              <TableRow key={k.key_id}>
                <TableCell className="font-medium">{k.name}</TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  {k.creator_username ?? k.created_by ?? "-"}
                </TableCell>
                <TableCell>{spentCell(k.spent.total_nano_usd)}</TableCell>
                <TableCell>
                  <LimitInputs
                    limits={keyDrafts[k.key_id] ?? {}}
                    labels={windowLabel}
                    onChange={(w, raw) => {
                      const nano = usdToNanoLimit(raw);
                      if (nano === undefined) return;
                      setKeyDrafts((prev) => ({
                        ...prev,
                        [k.key_id]: { ...prev[k.key_id], [w]: nano },
                      }));
                    }}
                  />
                </TableCell>
                <TableCell>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={saving}
                    onClick={() => saveKeyLimits(k.key_id)}
                  >
                    {t("orgLimits.save")}
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        </div>
      </section>

      <div className="flex justify-end">
        <Button disabled={saving} onClick={save}>
          {saving ? t("common.saving") : t("orgLimits.saveSpaceAndMembers")}
        </Button>
      </div>
    </div>
  );
}

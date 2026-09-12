import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Loader2, ShieldQuestion } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { motion, springs } from "@/components/ui/motion";

/** ORG-19: the invite landing — org identity in the middle, accept or decline. */
export function JoinOrgPage() {
  const { token = "" } = useParams();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const preview = useSWR(
    token ? `/api/dashboard/orgs/invite/${token}` : null,
    () => api.previewOrgInvite(token),
  );

  const handleAccept = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const joined = await api.joinOrg(token);
      toast.success(t("org.joined"));
      navigate(`/dashboard/org?org=${joined.org_id}`, { replace: true });
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-dvh items-center justify-center bg-muted/40 p-4">
      <motion.div initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={springs.snappy}>
        <Card className="w-full max-w-md rounded-3xl">
          <CardContent className="flex flex-col items-center gap-5 p-8 text-center">
            {preview.isLoading ? (
              <>
                <Skeleton className="size-20 rounded-3xl" />
                <Skeleton className="h-7 w-40" />
                <Skeleton className="h-4 w-56" />
                <Skeleton className="h-10 w-full" />
              </>
            ) : preview.error ? (
              <>
                <ShieldQuestion className="size-16 text-muted-foreground" />
                <p className="text-lg font-semibold">{t("org.inviteInvalid")}</p>
                <p className="text-sm text-muted-foreground">{t("org.inviteInvalidDescription")}</p>
                <Button asChild variant="outline" className="w-full">
                  <Link to="/dashboard">{t("org.backToDashboard")}</Link>
                </Button>
              </>
            ) : preview.data ? (
              <>
                <div
                  className="flex size-20 items-center justify-center rounded-3xl text-4xl"
                  style={{ backgroundColor: `${preview.data.avatar_color}22`, color: preview.data.avatar_color }}
                >
                  {preview.data.avatar_emoji}
                </div>
                <div>
                  <h1 className="text-xl font-semibold">{preview.data.display_name}</h1>
                  <p className="mt-1 text-sm text-muted-foreground">
                    {t("org.inviteOwner", { owner: preview.data.owner_username })}
                    {" · "}
                    {t("org.members", { count: preview.data.member_count })}
                  </p>
                </div>
                <p className="text-sm text-muted-foreground">{t("org.inviteJoinHint")}</p>
                <div className="grid w-full grid-cols-2 gap-3">
                  <Button variant="outline" onClick={() => navigate("/dashboard", { replace: true })}>
                    {t("org.decline")}
                  </Button>
                  <Button onClick={() => void handleAccept()} disabled={busy}>
                    {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                    {t("org.accept")}
                  </Button>
                </div>
              </>
            ) : null}
          </CardContent>
        </Card>
      </motion.div>
    </div>
  );
}

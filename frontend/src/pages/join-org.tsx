import { useState } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import useSWR from "swr";
import { toast } from "sonner";
import { Loader2, ShieldQuestion, UserRoundX, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api } from "@/lib/api";
import { motion, springs } from "@/components/ui/motion";
import { useAuth } from "@/hooks/use-auth";

/** ORG-19: the invite landing — a bold centered card with accept/decline. */
export function JoinOrgPage() {
  const { token = "" } = useParams();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { user, loading } = useAuth();
  const [busy, setBusy] = useState(false);
  const preview = useSWR(
    token ? `/api/dashboard/orgs/invite/${token}` : null,
    () => api.previewOrgInvite(token),
  );

  // ORG-19c: the preview needs a session. An unauthenticated visitor is sent to
  // sign in and comes back to this link afterwards; otherwise the 401 would be
  // reported as an invalid invite.
  if (!loading && !user) {
    return <Navigate to="/login" state={{ from: `/join/${token}` }} replace />;
  }

  const handleAccept = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const joined = await api.joinOrg(token);
      toast.success(t("org.joined"));
      navigate(`/org/${joined.org_id}/home`, { replace: true });
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("common.error"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="relative flex min-h-dvh items-center justify-center overflow-hidden bg-background p-6">
      <div
        className="pointer-events-none absolute -top-32 left-1/2 h-[28rem] w-[42rem] -translate-x-1/2 rounded-full opacity-20 blur-3xl"
        style={{ backgroundColor: preview.data?.avatar_color ?? "#6366f1" }}
      />
      <motion.div
        initial={{ opacity: 0, y: 24, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={springs.snappy}
        className="relative w-full max-w-lg rounded-3xl border bg-card p-10 text-center shadow-xl"
      >
        {preview.isLoading ? (
          <div className="flex flex-col items-center gap-5">
            <Skeleton className="size-24 rounded-3xl" />
            <Skeleton className="h-8 w-52" />
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-12 w-full" />
          </div>
        ) : preview.error ? (
          <div className="flex flex-col items-center gap-4">
            <ShieldQuestion className="size-16 text-muted-foreground" />
            <h1 className="text-xl font-semibold">{t("org.inviteInvalid")}</h1>
            <p className="text-sm text-muted-foreground">{t("org.inviteInvalidDescription")}</p>
            <Button asChild variant="outline" className="mt-2 w-full">
              <Link to="/dashboard">{t("org.backToDashboard")}</Link>
            </Button>
          </div>
        ) : preview.data ? (
          <>
            <p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">
              {t("org.inviteBanner")}
            </p>
            {preview.data.avatar_image ? (
              <img
                src={preview.data.avatar_image}
                alt=""
                className="mx-auto mt-5 size-24 rounded-3xl object-cover shadow-md"
              />
            ) : (
              <div
                className="mx-auto mt-5 flex size-24 items-center justify-center rounded-3xl text-5xl shadow-md"
                style={{
                  backgroundColor: `${preview.data.avatar_color}22`,
                  color: preview.data.avatar_color,
                }}
              >
                {preview.data.avatar_emoji}
              </div>
            )}
            <h1 className="mt-5 text-3xl font-semibold tracking-tight">
              {preview.data.display_name}
            </h1>
            <p className="mt-2 text-sm text-muted-foreground">
              {t("org.inviteOwner", { owner: preview.data.owner_username })}
            </p>
            <div className="mx-auto mt-4 flex w-fit items-center gap-2 rounded-full border px-4 py-1.5 text-sm text-muted-foreground">
              <UsersRound className="size-4" />
              {t("org.members", { count: preview.data.member_count })}
              <span className="text-muted-foreground/60">·</span>
              {t("org.inviteJoinHint")}
            </div>
            {preview.data.is_full && (
              <div className="mx-auto mt-4 flex max-w-sm items-start gap-2.5 rounded-lg border border-warning/40 bg-warning/10 px-4 py-3 text-left">
                <UserRoundX className="mt-0.5 size-4 shrink-0 text-warning" />
                <div>
                  <p className="text-sm font-medium text-warning">{t("org.fullTitle")}</p>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    {t("org.fullDescription", { count: preview.data.max_members })}
                  </p>
                </div>
              </div>
            )}
            <div className="mt-8 grid grid-cols-2 gap-3">
              <Button
                variant="outline"
                size="lg"
                onClick={() => navigate("/dashboard", { replace: true })}
              >
                {t("org.decline")}
              </Button>
              <Button
                size="lg"
                onClick={() => void handleAccept()}
                disabled={busy || preview.data.is_full}
              >
                {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                {t("org.accept")}
              </Button>
            </div>
          </>
        ) : null}
      </motion.div>
    </div>
  );
}

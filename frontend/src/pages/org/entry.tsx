import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Building2, Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { EmptyState } from "@/components/ui/empty-state";
import { api, type OrgInviteExpiry } from "@/lib/api";
import { useAuth } from "@/hooks/use-auth";
import { motion, springs } from "@/components/ui/motion";
import { ORGS_KEY, useMyOrgs, OrgAvatar } from "./shell";
import { mutate } from "swr";

async function useMyOrgsRefresh() {
  await mutate(ORGS_KEY);
}

const INVITE_EXPIRIES: OrgInviteExpiry[] = ["24h", "3d", "7d", "30d", "never"];
const EMOJI_CHOICES = ["🏢", "🚀", "⚡", "🧠", "🛠️", "📊", "🎯", "🔮", "🌿", "🐙", "🦾", "💼"];
const COLOR_CHOICES = ["#6366f1", "#0ea5e9", "#10b981", "#f59e0b", "#ef4444", "#8b5cf6"];

/** Client-side resize of a picked avatar into a compact data URL. */
async function fileToAvatarDataUrl(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("canvas unavailable");
  const side = Math.min(bitmap.width, bitmap.height);
  context.drawImage(
    bitmap,
    (bitmap.width - side) / 2,
    (bitmap.height - side) / 2,
    side,
    side,
    0,
    0,
    128,
    128,
  );
  return canvas.toDataURL("image/jpeg", 0.85);
}

/** `/org` without an org id: pick the first org, or create/join. */
export function OrgEntry() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const navigate = useNavigate();
  const { data: orgs, isLoading } = useMyOrgs();
  const [createOpen, setCreateOpen] = useState(false);
  const [joinInput, setJoinInput] = useState("");

  const canCreate =
    user?.account_class === "enterprise" && !user?.parent_user_id && !user?.is_sales_agent;

  if (isLoading) {
    return <div className="flex min-h-dvh items-center justify-center text-sm text-muted-foreground">…</div>;
  }

  if (orgs && orgs.length > 0) {
    navigate(`/org/${orgs[0].id}/home`, { replace: true });
    return null;
  }

  return (
    <div className="flex min-h-dvh items-center justify-center bg-muted/40 p-6">
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={springs.snappy}
        className="w-full max-w-2xl"
      >
        <EmptyState
          variant="card"
          icon={<Building2 className="h-12 w-12" />}
          title={t("org.empty")}
          description={canCreate ? t("org.emptyCreateHint") : t("org.emptyJoinHint")}
        />
        <Card className="mt-4 rounded-2xl">
          <CardContent className="flex flex-col gap-4 p-6">
            {canCreate && (
              <Button onClick={() => setCreateOpen(true)} className="w-full">
                <Plus className="h-4 w-4 mr-2" />
                {t("org.create")}
              </Button>
            )}
            <CreateOrgDialog
              open={createOpen}
              onOpenChange={setCreateOpen}
              onCreated={(orgId) => {
                void useMyOrgsRefresh();
                navigate(`/org/${orgId}/home`);
              }}
            />
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                placeholder={t("org.joinPlaceholder")}
                value={joinInput}
                onChange={(event) => setJoinInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || !joinInput.trim()) return;
                  const token = joinInput.trim().split("/join/")[1]?.split(/[?#]/)[0] ?? joinInput.trim();
                  navigate(`/join/${encodeURIComponent(token)}`);
                }}
              />
              <Button
                variant="outline"
                disabled={!joinInput.trim()}
                onClick={() => {
                  const token = joinInput.trim().split("/join/")[1]?.split(/[?#]/)[0] ?? joinInput.trim();
                  navigate(`/join/${encodeURIComponent(token)}`);
                }}
              >
                {t("org.join")}
              </Button>
            </div>
            <Button asChild variant="ghost">
              <Link to="/dashboard">{t("org.backToDashboard")}</Link>
            </Button>
          </CardContent>
        </Card>
      </motion.div>
    </div>
  );
}

export function CreateOrgDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (orgId: string) => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [emoji, setEmoji] = useState(EMOJI_CHOICES[0]);
  const [color, setColor] = useState(COLOR_CHOICES[0]);
  const [image, setImage] = useState<string | null>(null);
  const [expiry, setExpiry] = useState<OrgInviteExpiry>("7d");
  const [busy, setBusy] = useState(false);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("org.createTitle")}</DialogTitle>
          <DialogDescription>{t("org.createDescription")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
          <div className="flex items-center gap-4">
            {image ? (
              <img src={image} alt="" className="size-16 shrink-0 rounded-2xl object-cover" />
            ) : (
              <OrgAvatar emoji={emoji} color={color} size="size-16" text="text-3xl" />
            )}
            <div className="grid flex-1 gap-2">
              <Label htmlFor="org-name">{t("org.nameLabel")}</Label>
              <Input
                id="org-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder={t("org.namePlaceholder")}
              />
            </div>
          </div>
          <div className="grid gap-2">
            <Label>{t("org.avatarUpload")}</Label>
            <div className="flex items-center gap-2">
              <Input
                type="file"
                accept="image/*"
                className="text-xs"
                onChange={async (event) => {
                  const file = event.target.files?.[0];
                  if (!file) return;
                  try {
                    setImage(await fileToAvatarDataUrl(file));
                  } catch {
                    toast.error(t("common.error"));
                  }
                }}
              />
              {image && (
                <Button variant="ghost" size="sm" onClick={() => setImage(null)}>
                  {t("org.avatarReset")}
                </Button>
              )}
            </div>
            {!image && (
              <div className="flex flex-wrap gap-1.5 pt-1">
                {EMOJI_CHOICES.map((choice) => (
                  <button
                    key={choice}
                    type="button"
                    onClick={() => setEmoji(choice)}
                    className={`flex size-9 items-center justify-center rounded-lg border text-lg transition-colors ${choice === emoji ? "border-primary bg-accent" : "hover:bg-accent/50"}`}
                  >
                    {choice}
                  </button>
                ))}
              </div>
            )}
          </div>
          <div className="grid gap-2">
            <Label>{t("org.colorLabel")}</Label>
            <div className="flex gap-2">
              {COLOR_CHOICES.map((choice) => (
                <button
                  key={choice}
                  type="button"
                  aria-label={choice}
                  onClick={() => setColor(choice)}
                  className={`size-8 rounded-full border-2 transition-transform ${choice === color ? "scale-110 border-foreground" : "border-transparent"}`}
                  style={{ backgroundColor: choice }}
                />
              ))}
            </div>
          </div>
          <div className="grid gap-2">
            <Label>{t("org.inviteExpiryLabel")}</Label>
            <div className="grid grid-cols-5 gap-1">
              {INVITE_EXPIRIES.map((value) => (
                <Button
                  key={value}
                  type="button"
                  size="sm"
                  variant={expiry === value ? "default" : "outline"}
                  onClick={() => setExpiry(value)}
                >
                  {value === "never" ? t("org.never") : value}
                </Button>
              ))}
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            disabled={busy || !name.trim()}
            onClick={async () => {
              setBusy(true);
              try {
                const created = await api.createOrg({
                  display_name: name.trim(),
                  avatar_emoji: emoji,
                  avatar_color: color,
                  avatar_image: image ?? undefined,
                  invite_expiry: expiry,
                });
                toast.success(t("org.created"));
                onOpenChange(false);
                onCreated(created.id);
              } catch (error) {
                toast.error(error instanceof Error ? error.message : t("common.error"));
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {t("org.create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export { INVITE_EXPIRIES, EMOJI_CHOICES, COLOR_CHOICES };

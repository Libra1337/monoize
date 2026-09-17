import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bell, Megaphone, Pencil, Pin, Plus, RefreshCw, Trash2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { PageHeader } from "@/components/ui/page-header";
import { PageWrapper } from "@/components/ui/motion";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { api } from "@/lib/api";
import type { Announcement, AnnouncementType } from "@/lib/api";
import { SWR_KEYS, useAdminAnnouncements } from "@/lib/swr";
import { mutate } from "swr";

const ANNOUNCEMENT_TYPES: AnnouncementType[] = ["info", "success", "warning", "error"];
const TITLE_MAX = 200;
const CONTENT_MAX = 5000;

interface FormState {
  title: string;
  content: string;
  type: AnnouncementType;
  pinned: boolean;
  enabled: boolean;
}

const EMPTY_FORM: FormState = {
  title: "",
  content: "",
  type: "info",
  pinned: false,
  enabled: true,
};

const TYPE_BADGE_CLASS: Record<AnnouncementType, string> = {
  info: "bg-primary/10 text-primary",
  success: "bg-success/10 text-success",
  warning: "bg-warning/10 text-warning",
  error: "bg-destructive/10 text-destructive",
};

export function AnnouncementsAdminPage() {
  const { t } = useTranslation();
  const { data, error, isLoading, mutate: revalidate } = useAdminAnnouncements();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<Announcement | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const announcements = useMemo(() => data?.announcements ?? [], [data]);

  const openCreate = () => {
    setEditing(null);
    setForm(EMPTY_FORM);
    setFormError(null);
    setDialogOpen(true);
  };

  const openEdit = (announcement: Announcement) => {
    setEditing(announcement);
    setForm({
      title: announcement.title,
      content: announcement.content,
      type: announcement.type,
      pinned: announcement.pinned,
      enabled: announcement.enabled,
    });
    setFormError(null);
    setDialogOpen(true);
  };

  const submit = async () => {
    const title = form.title.trim();
    const content = form.content.trim();
    if (!title || title.length > TITLE_MAX) {
      setFormError(t("announcements.admin.titleError"));
      return;
    }
    if (!content || content.length > CONTENT_MAX) {
      setFormError(t("announcements.admin.contentError"));
      return;
    }
    setSaving(true);
    setFormError(null);
    try {
      if (editing) {
        const updated = await api.updateAnnouncement(editing.id, {
          title,
          content,
          type: form.type,
          pinned: form.pinned,
          enabled: form.enabled,
        });
        mutate(
          SWR_KEYS.ADMIN_ANNOUNCEMENTS,
          {
            announcements: announcements.map((item) =>
              item.id === updated.id ? updated : item
            ),
          },
          false
        );
      } else {
        const created = await api.createAnnouncement({
          title,
          content,
          type: form.type,
          pinned: form.pinned,
          enabled: form.enabled,
        });
        mutate(
          SWR_KEYS.ADMIN_ANNOUNCEMENTS,
          { announcements: [created, ...announcements] },
          false
        );
        // The bell list gains a new unread item immediately.
        void mutate(SWR_KEYS.ANNOUNCEMENTS);
      }
      setDialogOpen(false);
    } catch (requestError) {
      setFormError(
        requestError instanceof Error
          ? requestError.message
          : t("common.error")
      );
      await revalidate();
    } finally {
      setSaving(false);
    }
  };

  const remove = async (id: string) => {
    const previous = announcements;
    mutate(
      SWR_KEYS.ADMIN_ANNOUNCEMENTS,
      { announcements: announcements.filter((item) => item.id !== id) },
      false
    );
    try {
      await api.deleteAnnouncement(id);
      void mutate(SWR_KEYS.ANNOUNCEMENTS);
    } catch {
      mutate(SWR_KEYS.ADMIN_ANNOUNCEMENTS, { announcements: previous }, false);
    }
  };

  return (
    <PageWrapper className="space-y-5 pb-6">
      <PageHeader
        title={t("announcements.admin.title")}
        description={t("announcements.admin.description")}
        actions={
          <Button size="sm" onClick={openCreate}>
            <Plus data-icon />
            {t("announcements.admin.create")}
          </Button>
        }
      />

      <Card className="overflow-hidden rounded-xl">
        <CardContent className="p-0">
          {isLoading && !data ? (
            <div className="space-y-2 p-5">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : error && !data ? (
            <EmptyState
              variant="card"
              title={t("announcements.admin.loadFailed")}
              description={error instanceof Error ? error.message : t("common.error")}
              action={
                <Button variant="outline" onClick={() => void revalidate()}>
                  <RefreshCw data-icon />
                  {t("common.retry")}
                </Button>
              }
            />
          ) : announcements.length === 0 ? (
            <EmptyState
              icon={<Megaphone className="size-8 text-muted-foreground" />}
              title={t("announcements.admin.empty")}
              className="py-14"
            />
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[720px] text-sm">
                <thead>
                  <tr className="border-b bg-muted/35 text-left text-xs text-muted-foreground">
                    <th className="px-5 py-3 font-medium">{t("announcements.admin.titleLabel")}</th>
                    <th className="px-3 py-3 font-medium">{t("announcements.admin.typeLabel")}</th>
                    <th className="px-3 py-3 font-medium">{t("announcements.admin.pinnedLabel")}</th>
                    <th className="px-3 py-3 font-medium">{t("announcements.admin.statusLabel")}</th>
                    <th className="px-3 py-3 font-medium">{t("announcements.admin.timeLabel")}</th>
                    <th className="px-5 py-3 text-right font-medium">{t("common.actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {announcements.map((announcement) => (
                    <tr
                      key={announcement.id}
                      className="border-b transition-colors duration-200 last:border-b-0 hover:bg-accent/45"
                    >
                      <td className="max-w-72 px-5 py-3">
                        <span className="block truncate font-medium">
                          {announcement.title}
                        </span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {announcement.content}
                        </span>
                      </td>
                      <td className="px-3 py-3">
                        <span
                          className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${TYPE_BADGE_CLASS[announcement.type] ?? TYPE_BADGE_CLASS.info}`}
                        >
                          {t(`announcements.types.${announcement.type}`)}
                        </span>
                      </td>
                      <td className="px-3 py-3">
                        {announcement.pinned ? (
                          <Pin className="size-4 text-primary" aria-label={t("announcements.admin.pinnedLabel")} />
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </td>
                      <td className="px-3 py-3">
                        <Badge variant={announcement.enabled ? "secondary" : "outline"}>
                          {announcement.enabled
                            ? t("common.enabled")
                            : t("common.disabled")}
                        </Badge>
                      </td>
                      <td className="whitespace-nowrap px-3 py-3 font-mono text-xs text-muted-foreground">
                        {new Date(announcement.created_at).toLocaleString()}
                      </td>
                      <td className="px-5 py-3 text-right">
                        <div className="flex justify-end gap-1">
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => openEdit(announcement)}
                            aria-label={t("common.edit")}
                          >
                            <Pencil data-icon />
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={deletingId === announcement.id}
                            onClick={() => {
                              setDeletingId(announcement.id);
                              void remove(announcement.id).finally(() =>
                                setDeletingId(null)
                              );
                            }}
                            aria-label={t("common.delete")}
                          >
                            <Trash2 data-icon className="text-destructive" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-lg">
          <div className="flex max-h-[calc(100dvh-2rem)] flex-col overflow-y-auto p-5 sm:p-6">
            <DialogHeader className="shrink-0 pr-10">
              <DialogTitle className="flex items-center gap-2">
                <Bell className="size-5 text-primary" />
                {editing
                  ? t("announcements.admin.editTitle")
                  : t("announcements.admin.createTitle")}
              </DialogTitle>
              <DialogDescription className="mt-2 text-pretty">
                {t("announcements.admin.formDescription")}
              </DialogDescription>
            </DialogHeader>

            <div className="mt-4 space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="announcement-title">
                  {t("announcements.admin.titleLabel")}
                </Label>
                <Input
                  id="announcement-title"
                  value={form.title}
                  maxLength={TITLE_MAX}
                  onChange={(event) =>
                    setForm((state) => ({ ...state, title: event.target.value }))
                  }
                />
              </div>

              <div className="space-y-1.5">
                <Label htmlFor="announcement-content">
                  {t("announcements.admin.contentLabel")}
                </Label>
                <Textarea
                  id="announcement-content"
                  value={form.content}
                  maxLength={CONTENT_MAX}
                  rows={6}
                  onChange={(event) =>
                    setForm((state) => ({ ...state, content: event.target.value }))
                  }
                />
              </div>

              <div className="space-y-1.5">
                <Label>{t("announcements.admin.typeLabel")}</Label>
                <Select
                  value={form.type}
                  onValueChange={(value) =>
                    setForm((state) => ({ ...state, type: value as AnnouncementType }))
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ANNOUNCEMENT_TYPES.map((type) => (
                      <SelectItem key={type} value={type}>
                        {t(`announcements.types.${type}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="flex items-center justify-between">
                <Label htmlFor="announcement-pinned">
                  {t("announcements.admin.pinnedLabel")}
                </Label>
                <Switch
                  id="announcement-pinned"
                  checked={form.pinned}
                  onCheckedChange={(checked) =>
                    setForm((state) => ({ ...state, pinned: checked }))
                  }
                />
              </div>

              <div className="flex items-center justify-between">
                <Label htmlFor="announcement-enabled">
                  {t("announcements.admin.statusLabel")}
                </Label>
                <Switch
                  id="announcement-enabled"
                  checked={form.enabled}
                  onCheckedChange={(checked) =>
                    setForm((state) => ({ ...state, enabled: checked }))
                  }
                />
              </div>

              {formError && <p className="text-sm text-destructive">{formError}</p>}
            </div>

            <div className="mt-5 flex justify-end gap-2 border-t pt-4">
              <Button variant="outline" onClick={() => setDialogOpen(false)}>
                {t("common.cancel")}
              </Button>
              <Button disabled={saving} onClick={() => void submit()}>
                {saving && <RefreshCw data-icon className="animate-spin" />}
                {t("common.save")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </PageWrapper>
  );
}

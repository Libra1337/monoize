import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { mutate } from "swr";
import { Bell, CheckCheck, Pin, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { useAnnouncements } from "@/lib/swr";
import { api } from "@/lib/api";
import type { Announcement, AnnouncementType } from "@/lib/api";
import { SWR_KEYS } from "@/lib/swr";
import { cn } from "@/lib/utils";

/** AN-13: snooze key holds the Beijing day id until which popups are suppressed. */
const SNOOZE_KEY = "lynshen-announcement-snooze-until";

function beijingTodayId(): string {
  const beijingNow = new Date(Date.now() + 8 * 3600 * 1000);
  return beijingNow.toISOString().slice(0, 10);
}

function snoozedToday(): boolean {
  try {
    return window.localStorage.getItem(SNOOZE_KEY) === beijingTodayId();
  } catch {
    return false;
  }
}

const TYPE_DOT_CLASS: Record<AnnouncementType, string> = {
  info: "bg-primary",
  success: "bg-success",
  warning: "bg-warning",
  error: "bg-destructive",
};

function formatPublishedAt(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const diffMs = Date.now() - date.getTime();
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 1) return new Date().toLocaleDateString();
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return date.toLocaleDateString();
}

export function NotificationBell({ collapsed }: { collapsed?: boolean }) {
  const { t } = useTranslation();
  const { data } = useAnnouncements();
  const [open, setOpen] = useState(false);
  const autoPopupChecked = useRef(false);

  const unread = data?.unread_count ?? 0;

  // AN-13: auto-open once per dashboard mount when unread work exists and the
  // user has not snoozed today.
  useEffect(() => {
    if (autoPopupChecked.current || !data) return;
    autoPopupChecked.current = true;
    if (data.unread_count > 0 && !snoozedToday()) {
      setOpen(true);
    }
  }, [data]);

  // AN-12: opening the dialog marks every listed announcement read.
  useEffect(() => {
    if (!open || !data || data.announcements.length === 0) return;
    const unreadIds = data.announcements
      .filter((item) => !item.is_read)
      .map((item) => item.id);
    if (unreadIds.length === 0) return;
    mutate(
      SWR_KEYS.ANNOUNCEMENTS,
      {
        announcements: data.announcements.map((item) =>
          unreadIds.includes(item.id) ? { ...item, is_read: true } : item
        ),
        unread_count: 0,
      },
      false
    );
    void api
      .markAnnouncementsRead(unreadIds)
      .then((result) => {
        mutate(
          SWR_KEYS.ANNOUNCEMENTS,
          { announcements: data.announcements.map((item) => ({ ...item, is_read: true })), unread_count: result.unread_count },
          false
        );
      })
      .catch(() => {
        void mutate(SWR_KEYS.ANNOUNCEMENTS);
      });
  }, [open, data]);

  const snoozeToday = () => {
    try {
      window.localStorage.setItem(SNOOZE_KEY, beijingTodayId());
    } catch {
      // localStorage may be unavailable; the popup then simply re-opens.
    }
    setOpen(false);
  };

  const markAllRead = async (announcements: Announcement[]) => {
    await api.markAnnouncementsRead([]).catch(() => undefined);
    mutate(
      SWR_KEYS.ANNOUNCEMENTS,
      { announcements: announcements.map((item) => ({ ...item, is_read: true })), unread_count: 0 },
      false
    );
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={t("announcements.bell")}
        className={cn(
          "relative flex size-9 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
          collapsed ? "mx-auto" : "mx-1"
        )}
      >
        <Bell className="size-4" />
        {unread > 0 && (
          <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 font-mono text-[10px] font-semibold leading-none text-destructive-foreground">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
      </button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-hidden rounded-2xl p-0 sm:max-w-xl">
          <div className="flex max-h-[calc(100dvh-2rem)] flex-col p-5 sm:p-6">
            <DialogHeader className="shrink-0 pr-10">
              <DialogTitle className="flex items-center gap-2">
                <Bell className="size-5 text-primary" />
                {t("announcements.title")}
              </DialogTitle>
              <DialogDescription className="mt-2 text-pretty">
                {t("announcements.description")}
              </DialogDescription>
            </DialogHeader>

            <div className="mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto pr-1">
              {(data?.announcements ?? []).length === 0 ? (
                <EmptyState
                  icon={<Bell className="size-8 text-muted-foreground" />}
                  title={t("announcements.empty")}
                  className="py-10"
                />
              ) : (
                (data?.announcements ?? []).map((item) => (
                  <article
                    key={item.id}
                    className={cn(
                      "relative rounded-xl border p-4",
                      item.is_read ? "bg-card" : "border-primary/40 bg-primary/5"
                    )}
                  >
                    <div className="flex items-start gap-3">
                      <span
                        className={cn(
                          "mt-1.5 size-2 shrink-0 rounded-full",
                          TYPE_DOT_CLASS[item.type] ?? TYPE_DOT_CLASS.info
                        )}
                        aria-hidden
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <h3 className="truncate font-medium">{item.title}</h3>
                          {item.pinned && (
                            <Pin className="size-3.5 shrink-0 text-primary" aria-label={t("announcements.pinned")} />
                          )}
                          {!item.is_read && (
                            <span className="shrink-0 rounded-full bg-primary px-1.5 py-0.5 text-[10px] font-medium text-primary-foreground">
                              {t("announcements.unread")}
                            </span>
                          )}
                        </div>
                        <p className="mt-1.5 whitespace-pre-wrap break-words text-sm text-muted-foreground">
                          {item.content}
                        </p>
                        <p className="mt-2 font-mono text-xs text-muted-foreground/70">
                          {formatPublishedAt(item.created_at)}
                        </p>
                      </div>
                    </div>
                  </article>
                ))
              )}
            </div>

            <div className="mt-4 flex shrink-0 items-center justify-between gap-2 border-t pt-4">
              <Button
                size="sm"
                variant="ghost"
                disabled={(data?.announcements ?? []).length === 0}
                onClick={() => void markAllRead(data?.announcements ?? [])}
              >
                <CheckCheck data-icon />
                {t("announcements.markAllRead")}
              </Button>
              <div className="flex items-center gap-2">
                <Button size="sm" variant="outline" onClick={snoozeToday}>
                  {t("announcements.snoozeToday")}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setOpen(false)}>
                  <X data-icon />
                  {t("common.close")}
                </Button>
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}

import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-4">
      <div className="min-w-0">
        <h1 className="truncate font-display text-2xl font-semibold tracking-tight text-balance">
          {title}
        </h1>
        {description ? (
          <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        ) : null}
      </div>
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  variant = "card",
}: {
  icon: LucideIcon;
  title: string;
  description?: string;
  action?: React.ReactNode;
  variant?: "card" | "inline";
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 py-16 text-center",
        variant === "card" && "rounded-lg border border-dashed bg-card",
      )}
    >
      <div className="flex h-11 w-11 items-center justify-center rounded-lg bg-muted">
        <Icon className="h-5 w-5 text-muted-foreground" />
      </div>
      <div className="space-y-1">
        <div className="text-sm font-medium">{title}</div>
        {description ? (
          <div className="mx-auto max-w-sm text-sm text-muted-foreground">{description}</div>
        ) : null}
      </div>
      {action}
    </div>
  );
}

export function StatusDot({ status }: { status: string }) {
  const tone =
    status === "succeeded"
      ? "bg-success"
      : status === "failed"
        ? "bg-destructive"
        : status === "running" || status === "claimed" || status === "queued"
          ? "bg-primary"
          : status === "partial"
            ? "bg-warning"
            : "bg-muted-foreground/40";
  const pulse = status === "running" || status === "claimed";
  return (
    <span className="relative inline-flex h-2 w-2">
      {pulse ? <span className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-60", tone)} /> : null}
      <span className={cn("relative inline-flex h-2 w-2 rounded-full", tone)} />
    </span>
  );
}

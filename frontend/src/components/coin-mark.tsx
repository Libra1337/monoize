import { cn } from "@/lib/utils";

export function CoinMark({ className, label = "Coin" }: { className?: string; label?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      role={label ? "img" : undefined}
      aria-label={label || undefined}
      aria-hidden={label ? undefined : true}
      className={cn("shrink-0", className)}
      fill="none"
    >
      <circle cx="12" cy="12" r="9" fill="currentColor" className="text-amber-400" />
      <circle cx="12" cy="12" r="7.1" stroke="currentColor" strokeWidth="1.35" className="text-amber-700/70" />
      <path d="M8.1 10.1h7.8M8.1 13.9h7.8" stroke="currentColor" strokeWidth="1.35" strokeLinecap="round" className="text-amber-800/80" />
      <path d="M10.2 8.1v7.8M13.8 8.1v7.8" stroke="currentColor" strokeWidth="0.85" strokeLinecap="round" className="text-amber-500/80" />
      <circle cx="9.1" cy="8.9" r="1" fill="white" fillOpacity="0.45" />
    </svg>
  );
}

import { cn } from "@/lib/utils";

/**
 * Apeiron brand mark: an "A" frame cut by an aperture beam, drawn in
 * currentColor with one primary accent. Product mark for the standalone
 * studio site (the platform keeps its own Monoize mark).
 */
export function ApeironLogo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" className={cn("h-5 w-5", className)} aria-hidden>
      <path
        d="M5 20 12 4l7 16"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path d="M8.5 14h7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <path
        d="M12 4c3 2.2 4.4 4.6 4.2 7.2"
        stroke="hsl(var(--primary))"
        strokeWidth="1.6"
        strokeLinecap="round"
        opacity="0.85"
      />
    </svg>
  );
}

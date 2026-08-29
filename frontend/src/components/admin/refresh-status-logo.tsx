import { useReducedMotion } from "framer-motion";

import { MonoizeLogo } from "@/components/MonoizeLogo";
import { motion } from "@/components/ui/motion";
import { cn } from "@/lib/utils";

export function RefreshStatusLogo({
  refreshing,
  label,
  className,
}: {
  refreshing: boolean;
  label: string;
  className?: string;
}) {
  const reducedMotion = useReducedMotion();
  return (
    <div
      className={cn(
        "flex size-9 shrink-0 items-center justify-center rounded-lg border bg-card text-foreground shadow-sm",
        className,
      )}
      role="status"
      aria-label={label}
      title={label}
    >
      <motion.div
        className="size-6"
        animate={refreshing && !reducedMotion ? { rotate: 360 } : { rotate: 0 }}
        transition={
          refreshing && !reducedMotion
            ? { duration: 1.15, ease: "linear", repeat: Infinity }
            : { duration: 0.2 }
        }
      >
        <MonoizeLogo className="size-full" />
      </motion.div>
    </div>
  );
}

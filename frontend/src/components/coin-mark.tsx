import { Coins } from "lucide-react";
import { cn } from "@/lib/utils";

export function CoinMark({ className, label = "Coin" }: { className?: string; label?: string }) {
  return <Coins aria-label={label} aria-hidden={label ? undefined : true} className={cn("shrink-0 text-amber-500", className)} />;
}

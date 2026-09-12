import { cn } from "@/lib/utils";
import { CoinMark } from "./coin-mark";

function withoutCoinPrefix(value: string): string {
  // The coin icon replaces the currency sign, so strip a legacy `C` and the current
  // `¥`/`$` labels while keeping a leading minus.
  return value.replace(/^-?[C¥$](?=\d)/, (prefix) => prefix.startsWith("-") ? "-" : "");
}

export function CoinAmount({
  value,
  className,
  iconClassName,
}: {
  value: string;
  className?: string;
  iconClassName?: string;
}) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 tabular-nums", className)}>
      <CoinMark className={cn("size-4", iconClassName)} />
      <span>{withoutCoinPrefix(value)}</span>
    </span>
  );
}

export function CoinPrice({ value, className }: { value: string; className?: string }) {
  return <CoinAmount value={value} className={className} />;
}

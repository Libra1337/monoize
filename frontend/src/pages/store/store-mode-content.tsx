import type { ReactNode } from "react";
import type { StoreTab } from "./store-tabs";

interface StoreModeContentProps {
  activeTab: StoreTab;
  purchaseContent: ReactNode;
  redemptionContent: ReactNode;
}

export function StoreModeContent({
  activeTab,
  purchaseContent,
  redemptionContent,
}: StoreModeContentProps) {
  return activeTab === "redeem" ? redemptionContent : purchaseContent;
}

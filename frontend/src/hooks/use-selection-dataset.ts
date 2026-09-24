import { useEffect, useState } from "react";

export function useSelectionDataset<T>({
  selectionKey,
  loading,
  dataset,
  animationDurationMs,
  enabled = true,
}: {
  selectionKey: string;
  loading: boolean;
  dataset: T;
  animationDurationMs: number;
  enabled?: boolean;
}) {
  const [state, setState] = useState({
    selectionKey,
    loading,
    inputDataset: dataset,
    enabled,
    displayedDataset: dataset,
    pendingSelection: false,
    animate: false,
  });

  if (
    state.selectionKey !== selectionKey || state.loading !== loading ||
    !Object.is(state.inputDataset, dataset) || state.enabled !== enabled
  ) {
    const pendingSelection = state.pendingSelection || state.selectionKey !== selectionKey;
    setState({
      selectionKey,
      loading,
      inputDataset: dataset,
      enabled,
      displayedDataset: loading ? state.displayedDataset : dataset,
      pendingSelection: loading && pendingSelection,
      animate: !loading && pendingSelection && enabled,
    });
  }

  useEffect(() => {
    if (!state.animate) return;
    const timer = setTimeout(() => {
      setState((current) => ({ ...current, animate: false }));
    }, animationDurationMs);
    return () => clearTimeout(timer);
  }, [animationDurationMs, state.animate, state.displayedDataset, state.selectionKey]);

  return { dataset: state.displayedDataset, animate: state.animate };
}

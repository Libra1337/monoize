import { useEffect, useRef, useState } from "react";

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
  const [displayedDataset, setDisplayedDataset] = useState(dataset);
  const [animate, setAnimate] = useState(false);
  const selectionKeyRef = useRef(selectionKey);
  const pendingSelectionRef = useRef(false);
  const animationTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (selectionKeyRef.current !== selectionKey) {
      selectionKeyRef.current = selectionKey;
      pendingSelectionRef.current = true;
      setAnimate(false);
    }

    if (pendingSelectionRef.current) {
      if (loading) return;
      pendingSelectionRef.current = false;
      setDisplayedDataset(dataset);
      setAnimate(enabled);
      if (animationTimerRef.current) clearTimeout(animationTimerRef.current);
      if (enabled) {
        animationTimerRef.current = setTimeout(() => {
          setAnimate(false);
          animationTimerRef.current = null;
        }, animationDurationMs);
      }
      return;
    }

    if (!loading) {
      setAnimate(false);
      setDisplayedDataset(dataset);
    }
  }, [animationDurationMs, dataset, enabled, loading, selectionKey]);

  useEffect(() => () => {
    if (animationTimerRef.current) clearTimeout(animationTimerRef.current);
  }, []);

  return { dataset: displayedDataset, animate };
}

import { describe, expect, test } from "bun:test";
import { useState } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { useSelectionDataset } from "../src/hooks/use-selection-dataset";

type Selection = {
  selectionKey: string;
  loading: boolean;
  dataset: string;
  enabled?: boolean;
};

function renderSelections(selections: Selection[]) {
  function Probe() {
    const [index, setIndex] = useState(0);
    const result = useSelectionDataset({ ...selections[index], animationDurationMs: 1200 });
    if (index < selections.length - 1) setIndex(index + 1);
    return <output data-animate={result.animate}>{result.dataset}</output>;
  }
  return renderToStaticMarkup(<Probe />);
}

const initial = { selectionKey: "today", loading: false, dataset: "today values" };
const pending = { selectionKey: "week", loading: true, dataset: "placeholder" };
const resolved = { selectionKey: "week", loading: false, dataset: "week values" };

describe("selection dataset (UA-22c)", () => {
  test("retains the last resolved data while a new selection loads", () => {
    expect(renderSelections([initial, pending])).toBe('<output data-animate="false">today values</output>');
  });

  test("animates the resolved data for an explicit selection", () => {
    expect(renderSelections([initial, pending, resolved])).toBe('<output data-animate="true">week values</output>');
  });

  test("animates an already cached selection without waiting for a loading state", () => {
    expect(renderSelections([initial, resolved])).toBe('<output data-animate="true">week values</output>');
  });

  test("keeps the selection animation active across unrelated renders", () => {
    expect(renderSelections([initial, resolved, resolved])).toBe('<output data-animate="true">week values</output>');
  });

  test("stops selection interpolation when polling replaces its dataset", () => {
    expect(renderSelections([initial, resolved, { ...resolved, dataset: "polled week" }])).toBe('<output data-animate="false">polled week</output>');
  });

  test("updates polling data without a selection animation", () => {
    expect(renderSelections([initial, { ...initial, dataset: "polled values" }])).toBe('<output data-animate="false">polled values</output>');
  });

  test("renders the selected final values immediately with reduced motion", () => {
    expect(renderSelections([initial, pending, { ...resolved, enabled: false }])).toBe('<output data-animate="false">week values</output>');
  });

  test("waits for the most recent selection when a pending selection is replaced", () => {
    expect(renderSelections([initial, pending, { ...pending, selectionKey: "month" }])).toBe('<output data-animate="false">today values</output>');
  });
});

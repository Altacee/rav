import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorBoundary } from "@/components/shared/ErrorBoundary";

function Bomb(): never {
  throw new Error("boom");
}

describe("ErrorBoundary", () => {
  it("renders the default fallback when a child throws", () => {
    render(
      <ErrorBoundary>
        <Bomb />
      </ErrorBoundary>,
    );
    expect(screen.getByText("Something went wrong")).toBeTruthy();
  });

  it("renders a custom fallback and can reset back to children", () => {
    let shouldThrow = true;
    function Maybe() {
      if (shouldThrow) throw new Error("boom");
      return <p>recovered</p>;
    }
    const { rerender } = render(
      <ErrorBoundary fallback={(reset) => <button onClick={reset}>retry</button>}>
        <Maybe />
      </ErrorBoundary>,
    );
    expect(screen.getByText("retry")).toBeTruthy();
    shouldThrow = false;
    fireEvent.click(screen.getByText("retry"));
    rerender(
      <ErrorBoundary fallback={(reset) => <button onClick={reset}>retry</button>}>
        <Maybe />
      </ErrorBoundary>,
    );
    expect(screen.getByText("recovered")).toBeTruthy();
  });

  it("keeps sibling content mounted when a sibling boundary's child throws", () => {
    render(
      <div>
        <p>sibling content</p>
        <ErrorBoundary fallback={() => <p>panel fallback</p>}>
          <Bomb />
        </ErrorBoundary>
      </div>,
    );
    expect(screen.getByText("sibling content")).toBeTruthy();
    expect(screen.getByText("panel fallback")).toBeTruthy();
  });
});

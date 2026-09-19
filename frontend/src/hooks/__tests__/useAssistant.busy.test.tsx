import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";

const { apiPostStream } = vi.hoisted(() => ({
  apiPostStream: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  apiGet: vi.fn(),
  apiPostStream,
}));

import { useAssistantChat } from "@/hooks/useAssistant";
import { useAssistantStore } from "@/stores/useAssistantStore";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

describe("useAssistantChat busy/abort lifecycle", () => {
  beforeEach(() => {
    useAssistantStore.setState({ open: true, turns: [], busy: false, abortController: null });
  });

  afterEach(() => {
    apiPostStream.mockReset();
  });

  it("keeps busy true across unmount/remount and stop() aborts the in-flight request", async () => {
    apiPostStream.mockImplementation(
      (_path: string, _body: unknown, signal: AbortSignal) =>
        new Promise((_resolve, reject) => {
          signal.addEventListener("abort", () => {
            const err = new Error("aborted");
            err.name = "AbortError";
            reject(err);
          });
        }),
    );

    const { result, unmount } = renderHook(() => useAssistantChat(), { wrapper });

    act(() => {
      void result.current.ask("hi", false);
    });

    expect(useAssistantStore.getState().busy).toBe(true);

    unmount();
    // Busy lives in the store, not component state: it survives the panel unmounting mid-stream.
    expect(useAssistantStore.getState().busy).toBe(true);

    const { result: result2 } = renderHook(() => useAssistantChat(), { wrapper });
    expect(result2.current.busy).toBe(true);

    act(() => {
      result2.current.stop();
    });

    await waitFor(() => expect(useAssistantStore.getState().busy).toBe(false));
    expect(useAssistantStore.getState().abortController).toBeNull();
  });

  it("refuses to start a second turn while one is already in flight", async () => {
    apiPostStream.mockImplementation(() => new Promise(() => {}));
    const { result } = renderHook(() => useAssistantChat(), { wrapper });

    act(() => {
      void result.current.ask("first", false);
    });
    expect(useAssistantStore.getState().turns).toHaveLength(1);

    act(() => {
      void result.current.ask("second", false);
    });
    expect(useAssistantStore.getState().turns).toHaveLength(1);
    expect(apiPostStream).toHaveBeenCalledTimes(1);
  });
});

import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const { mockAssistantEnabled } = vi.hoisted(() => ({ mockAssistantEnabled: { value: false } }));

vi.mock("@/hooks/useDisplayPreferences", async () => {
  const actual = await vi.importActual<typeof import("@/hooks/useDisplayPreferences")>("@/hooks/useDisplayPreferences");
  return { ...actual, useDisplayPreferences: () => ({ data: undefined }) };
});
vi.mock("@/hooks/useMobileNav", () => ({ useMobileNav: () => ({ navigateTo: vi.fn(), goBack: vi.fn(), mobilePanelView: "list", isMobile: true }) }));
vi.mock("@/hooks/useAssistant", () => ({ useAssistantStatus: () => ({ enabled: mockAssistantEnabled.value }) }));

import { BottomTabBar } from "@/components/shared/BottomTabBar";
import { useAssistantStore } from "@/stores/useAssistantStore";

describe("BottomTabBar assistant entry", () => {
  it("does not show an Assistant tab when the assistant is disabled", () => {
    mockAssistantEnabled.value = false;
    render(<BottomTabBar />);
    expect(screen.queryByRole("button", { name: "Assistant" })).toBeNull();
  });

  it("shows an Assistant tab that toggles the panel when enabled", () => {
    mockAssistantEnabled.value = true;
    useAssistantStore.setState({ open: false });
    render(<BottomTabBar />);
    const btn = screen.getByRole("button", { name: "Assistant" });
    fireEvent.click(btn);
    expect(useAssistantStore.getState().open).toBe(true);
  });
});

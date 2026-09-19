import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi, beforeEach } from "vitest";

const { mockUiState, mockIsMobile, setOpen, reset } = vi.hoisted(() => ({
  mockUiState: {
    viewMode: "mail" as "mail" | "contacts" | "calendar" | "settings",
    effectiveAnimationMode: "off" as "rich" | "medium" | "subtle" | "off",
  },
  mockIsMobile: { value: false },
  setOpen: vi.fn(),
  reset: vi.fn(),
}));

vi.mock("@/stores/useUiStore", () => ({
  useUiStore: (selector: (state: typeof mockUiState) => unknown) => selector(mockUiState),
}));

vi.mock("@/hooks/useKeyboardShortcuts", () => ({ useKeyboardShortcuts: vi.fn() }));
vi.mock("@/hooks/useFolders", () => ({ useFolders: () => ({ data: { folders: [] }, status: "success" }) }));
vi.mock("@/hooks/useWebSocket", () => ({ useWebSocket: vi.fn(() => ({ status: "connected", failCount: 0 })) }));
vi.mock("@/hooks/useNotifications", () => ({
  useNotifications: vi.fn(() => ({
    showBanner: false,
    requestPermission: vi.fn(),
    dismissBanner: vi.fn(),
    handleEvent: vi.fn(),
  })),
}));
vi.mock("@/hooks/useIsMobile", () => ({ useIsMobile: () => mockIsMobile.value }));
vi.mock("@/hooks/useAssistant", () => ({ useAssistantStatus: () => ({ enabled: true }) }));
vi.mock("@/stores/useAssistantStore", () => ({
  useAssistantStore: Object.assign((selector: (s: { open: boolean }) => unknown) => selector({ open: true }), {
    getState: () => ({ reset, setOpen }),
  }),
}));

vi.mock("@/components/shared/ThreePanelLayout", () => ({ ThreePanelLayout: vi.fn(() => <div data-testid="mail-layout" />) }));
vi.mock("@/components/shared/NavRail", () => ({ NavRail: vi.fn(() => <div data-testid="nav-rail" />) }));
vi.mock("@/components/mail/FolderTree", () => ({ FolderTree: vi.fn(() => <div data-testid="folder-tree" />) }));
vi.mock("@/components/mail/MessageList", () => ({ MessageList: vi.fn(() => <div data-testid="message-list" />) }));
vi.mock("@/components/mail/ReadingPane", () => ({ ReadingPane: vi.fn(() => <div data-testid="reading-pane" />) }));
vi.mock("@/components/calendar/CalendarPanel", () => ({ CalendarPanel: vi.fn(() => <div data-testid="calendar-panel" />) }));
vi.mock("@/components/contacts/ContactsPanel", () => ({ ContactsPanel: vi.fn(() => <div data-testid="contacts-panel" />) }));
vi.mock("@/components/settings/SettingsPanel", () => ({ SettingsPanel: vi.fn(() => <div data-testid="settings-panel" />) }));
vi.mock("@/components/shared/NotificationBanner", () => ({ NotificationBanner: vi.fn(() => <div data-testid="notification-banner" />) }));
vi.mock("@/components/shared/KeyboardShortcuts", () => ({ KeyboardShortcuts: vi.fn(() => <div data-testid="keyboard-shortcuts" />) }));
vi.mock("@/components/shared/CommandPalette", () => ({ CommandPalette: vi.fn(() => <div data-testid="command-palette" />) }));
vi.mock("@/components/PreferencesLoader", () => ({ PreferencesLoader: vi.fn(() => <div data-testid="preferences-loader" />) }));
vi.mock("@/components/shared/BottomTabBar", () => ({ BottomTabBar: vi.fn(() => <div data-testid="bottom-tab-bar" />) }));
vi.mock("@/components/shared/ComposeFab", () => ({ ComposeFab: vi.fn(() => <div data-testid="compose-fab" />) }));
vi.mock("@/components/assistant/AssistantPanel", () => ({
  AssistantPanel: () => {
    throw new Error("boom");
  },
  useAssistantShortcut: vi.fn(),
}));

import MailPage from "../page";

describe("Mail page assistant error boundary fallback", () => {
  beforeEach(() => {
    setOpen.mockReset();
    reset.mockReset();
    mockIsMobile.value = false;
  });

  it("shows a Close button that closes the panel, keeps the mail view mounted", () => {
    render(<MailPage />);
    expect(screen.getByTestId("mail-layout")).toBeTruthy();
    expect(screen.getByText("The assistant hit a problem.")).toBeTruthy();
    fireEvent.click(screen.getByText("Close"));
    expect(setOpen).toHaveBeenCalledWith(false);
  });

  it("uses the mobile full-screen classes when isMobile", () => {
    mockIsMobile.value = true;
    render(<MailPage />);
    const fallback = screen.getByText("The assistant hit a problem.").closest("div.fixed");
    expect(fallback?.className).toContain("inset-0");
  });

  it("uses the desktop 380px column classes when not mobile", () => {
    mockIsMobile.value = false;
    render(<MailPage />);
    const fallback = screen.getByText("The assistant hit a problem.").closest("div.w-\\[380px\\]");
    expect(fallback).not.toBeNull();
  });
});

import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { AnswerText } from "@/components/assistant/AnswerText";
import { useUiStore } from "@/stores/useUiStore";

describe("AnswerText", () => {
  it("numbers ref markers in order of appearance and opens the email", () => {
    const setActiveFolder = vi.fn();
    const selectMessage = vi.fn();
    useUiStore.setState({ setActiveFolder, selectMessage } as never);
    render(<AnswerText text="See [m2] and [m1], again [m2]." sources={[
      { ref: "m1", folder: "INBOX", uid: 1, subject: "A", from: "a", date: "d" },
      { ref: "m2", folder: "Archive", uid: 9, subject: "B", from: "b", date: "d" },
    ]} />);
    const chips = screen.getAllByRole("button");
    expect(chips.map((c) => c.textContent)).toEqual(["1", "2", "1"]);
    fireEvent.click(chips[0]);
    expect(setActiveFolder).toHaveBeenCalledWith("Archive");
    expect(selectMessage).toHaveBeenCalledWith(9);
  });

  it("leaves unknown refs as text", () => {
    render(<AnswerText text="Maybe [m7]." sources={[]} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("Maybe [m7].")).toBeTruthy();
  });
});

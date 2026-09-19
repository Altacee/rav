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

  it("renders **bold** as <strong>", () => {
    const { container } = render(<AnswerText text="This is **bold** text." sources={[]} />);
    const strong = container.querySelector("strong");
    expect(strong?.textContent).toBe("bold");
  });

  it("renders - bullets as a list", () => {
    const { container } = render(<AnswerText text={"- one\n- two"} sources={[]} />);
    const items = Array.from(container.querySelectorAll("ul li")).map((li) => li.textContent);
    expect(items).toEqual(["one", "two"]);
  });

  it("renders 1. numbered lines as an ordered list", () => {
    const { container } = render(<AnswerText text={"1. first\n2. second"} sources={[]} />);
    const items = Array.from(container.querySelectorAll("ol li")).map((li) => li.textContent);
    expect(items).toEqual(["first", "second"]);
  });

  it("numbers chips inside bullets by first appearance across the whole answer", () => {
    render(
      <AnswerText
        text={"Intro [m2].\n\n- see [m2]\n- also [m1]"}
        sources={[
          { ref: "m1", folder: "INBOX", uid: 1, subject: "A", from: "a", date: "d" },
          { ref: "m2", folder: "Archive", uid: 9, subject: "B", from: "b", date: "d" },
        ]}
      />,
    );
    const chips = screen.getAllByRole("button");
    expect(chips.map((c) => c.textContent)).toEqual(["1", "1", "2"]);
  });

  it("renders script-looking text as literal text, not markup", () => {
    render(<AnswerText text="<script>alert(1)</script>" sources={[]} />);
    expect(screen.getByText("<script>alert(1)</script>")).toBeTruthy();
    expect(document.querySelector("script")).toBeNull();
  });

  it("leaves an underscore email address verbatim", () => {
    render(<AnswerText text="Contact first_last@x.com and snake_case please." sources={[]} />);
    expect(screen.getByText("Contact first_last@x.com and snake_case please.")).toBeTruthy();
  });

  it("leaves 'a * b * c' verbatim (no spurious em)", () => {
    const { container } = render(<AnswerText text="a * b * c" sources={[]} />);
    expect(container.querySelector("em")).toBeNull();
    expect(screen.getByText("a * b * c")).toBeTruthy();
  });

  it("still italicises *em* and _em_", () => {
    const { container } = render(<AnswerText text="This is *em* and this is _also em_." sources={[]} />);
    const ems = Array.from(container.querySelectorAll("em")).map((e) => e.textContent);
    expect(ems).toEqual(["em", "also em"]);
  });

  it("renders a chip inside bold", () => {
    render(<AnswerText text="**See [m1] now**" sources={[
      { ref: "m1", folder: "INBOX", uid: 1, subject: "A", from: "a", date: "d" },
    ]} />);
    const chip = screen.getByRole("button");
    expect(chip.textContent).toBe("1");
    expect(chip.closest("strong")).not.toBeNull();
  });

  it("renders unclosed streaming markers without throwing", () => {
    expect(() => render(<AnswerText text={"This is **bold and still going"} sources={[]} />)).not.toThrow();
    expect(screen.getByText(/This is \*\*bold and still going/)).toBeTruthy();
  });
});

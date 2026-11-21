// @vitest-environment jsdom
// The builder driven in a page: the palette adds a block, Alt+Down moves one, a read-only canvas has neither, and the
// content tab asks before going back to HTML and never writes `content` while a design exists.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router";
import { ConfirmHost } from "@ferrum/kit";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Builder } from "./Builder";
import type { Campaign } from "./campaign";
import { Content } from "./Content";
import { blankBlock, blankRow, starterDesign, type Design } from "./design";
import type { Draft } from "./useDraft";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const design = (): Design => {
  const d = starterDesign();
  d.rows[0] = blankRow(1);
  d.rows[0].columns[0].blocks = [blankBlock("heading"), blankBlock("text")];
  return d;
};

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  // jsdom has no modal dialogs: the confirm dialog opens and closes by its attribute.
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
    this.removeAttribute("open");
  };
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const button = (name: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === name);
const kinds = (d: Design) => d.rows[0].columns[0].blocks.map((b) => b.type);

describe("the builder", () => {
  it("adds a block from the palette", () => {
    const changes: Design[] = [];
    act(() => root.render(<Builder design={design()} editable onChange={(d) => changes.push(d)} />));
    act(() => button("Button")!.click());
    expect(changes).toHaveLength(1);
    expect(kinds(changes[0])).toEqual(["heading", "text", "button"]);
  });

  it("moves a block with Alt+Down", () => {
    const changes: Design[] = [];
    act(() => root.render(<Builder design={design()} editable onChange={(d) => changes.push(d)} />));
    const first = host.querySelector<HTMLElement>('[data-at="0-0-0"]')!;
    act(() => {
      first.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true }));
    });
    expect(kinds(changes[0])).toEqual(["text", "heading"]);
  });

  it("is a read-only canvas without a palette or grips", () => {
    act(() => root.render(<Builder design={design()} editable={false} onChange={() => {}} />));
    expect(host.querySelector('[role="group"]')).toBeNull();
    expect(host.querySelector('[data-at="0-0-0"]')!.getAttribute("draggable")).not.toBe("true");
  });
});

describe("the content tab with a design", () => {
  const campaign = { id: "c1", version: 1, docstatus: 0, campaign_name: "A", status: "Draft", subject: "S", content: "<p>x</p>", audience: [], variants: [], ab_test: false, track_opens: true, track_clicks: true, design: design() } as unknown as Campaign;

  it("asks before going back to HTML, then clears the design and never writes content", async () => {
    const set = vi.fn();
    const draft = { doc: campaign, set, editable: true, fieldErrors: {}, rowErrors: [], dirty: false, save: async () => {} } as unknown as Draft;
    act(() =>
      root.render(
        <QueryClientProvider client={new QueryClient()}>
          <MemoryRouter>
            <Content draft={draft} editable />
            <ConfirmHost />
          </MemoryRouter>
        </QueryClientProvider>,
      ),
    );
    act(() => button("HTML")!.click());
    await act(async () => button("Edit as HTML")!.click());
    const confirm = [...document.querySelectorAll("button")].filter((b) => b.textContent?.trim() === "Edit as HTML").pop()!;
    await act(async () => confirm.click());
    expect(set).toHaveBeenCalledWith("design", null);
    expect(set).not.toHaveBeenCalledWith("content", expect.anything());
  });
});

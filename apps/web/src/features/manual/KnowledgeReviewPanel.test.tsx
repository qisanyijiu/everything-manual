import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { KnowledgeReviewPanel, type EditValues } from "./KnowledgeReviewPanel";
import type { EntityReviewView } from "../viewer/draft-view";

const original = { id: "s", title: "原步骤", orderedActions: ["原操作"], safetyNotes: ["原错误注意"], evidence: [] };
function show(edited: EntityReviewView["userEdited"] = null, save = vi.fn().mockResolvedValue(true)) {
  function Harness() {
    const [editing, setEditing] = useState<string | null>(null);
    const [buffers, setBuffers] = useState<Record<string, EditValues>>({});
    return <KnowledgeReviewPanel parts={[]} steps={[original]} specs={[]}
      editing={editing} onEditing={setEditing} buffers={buffers}
      onBuffer={(id, value) => setBuffers((previous) => { const next = { ...previous }; if (value) next[id] = value; else delete next[id]; return next; })}
      entityReviews={{ s: { reviewStatus: null, userEdited: edited, textOnly: false, editedAt: null, editedBy: null } }}
      busy={false} modelReview={null} model={null} modelLoaded={false} narrow={false}
      onDeclareModelReady={vi.fn()} onDeclareModelConfirmed={vi.fn()} onSetEntityReview={vi.fn()} onSaveEntityEdit={save} />;
  }
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "复制为本地修订" }));
  return save;
}

it("copies source notes into an explicitly labelled editor and saves trimmed lines without changing source", async () => {
  const save = show();
  const notes = screen.getByRole("textbox", { name: "注意事项（每行一条）" });
  expect(notes).toHaveValue("原错误注意");
  expect(notes).toHaveAccessibleDescription(/可留空删除注意事项，仅影响人工覆盖层/);
  fireEvent.change(notes, { target: { value: " 红灯亮起之后等待至少四秒 \n\n 再开机 " } });
  fireEvent.click(screen.getByRole("button", { name: "保存人工修订（并确认事实）" }));
  await waitFor(() => expect(save).toHaveBeenCalledExactlyOnceWith("s", {
    title: "原步骤", orderedActions: ["原操作"], safetyNotes: ["红灯亮起之后等待至少四秒", "再开机"],
  }));
  expect(screen.getByText(/原步骤：原操作；注意事项：原错误注意/)).toBeVisible();
  expect(original.safetyNotes).toEqual(["原错误注意"]);
});

it("preserves earlier title/action revisions while explicitly clearing all safety notes", async () => {
  const save = show({ title: "已修标题", orderedActions: ["已修操作"], safetyNotes: ["已修注意"] });
  expect(screen.getByRole("textbox", { name: "步骤标题" })).toHaveValue("已修标题");
  expect(screen.getByRole("textbox", { name: "操作（每行一步）" })).toHaveValue("已修操作");
  const notes = screen.getByRole("textbox", { name: "注意事项（每行一条）" });
  expect(notes).toHaveValue("已修注意");
  fireEvent.change(notes, { target: { value: " \n " } });
  fireEvent.click(screen.getByRole("button", { name: "保存人工修订（并确认事实）" }));
  await waitFor(() => expect(save).toHaveBeenCalledExactlyOnceWith("s", {
    title: "已修标题", orderedActions: ["已修操作"], safetyNotes: [],
  }));
});

it("keeps an existing empty override empty on reopening and labels the removal beside the retained source", () => {
  show({ safetyNotes: [] });
  expect(screen.getByRole("textbox", { name: "注意事项（每行一条）" })).toHaveValue("");
  expect(screen.getByText(/人工修订：；注意事项：已清空（人工）/)).toBeVisible();
  expect(screen.getByText(/原错误注意/)).toBeVisible();
});

it.each([Array(13).fill("注意").join("\n"), "注".repeat(601)])("blocks out-of-contract notes and allows correction before saving", (value) => {
  const save = show();
  const notes = screen.getByRole("textbox", { name: "注意事项（每行一条）" });
  fireEvent.change(notes, { target: { value } });
  expect(notes).toHaveAttribute("aria-invalid", "true");
  expect(screen.getByRole("alert")).toHaveTextContent("注意事项最多12条，每条不得超过600字符。");
  const button = screen.getByRole("button", { name: "保存人工修订（并确认事实）" });
  expect(button).toBeDisabled(); fireEvent.click(button); expect(save).not.toHaveBeenCalled();
  fireEvent.change(notes, { target: { value: Array(12).fill("注".repeat(600)).join("\n") } });
  expect(button).toBeEnabled(); expect(notes).toHaveAttribute("aria-invalid", "false");
});

it("cancelling a warning revision performs no save", () => {
  const save = show();
  fireEvent.change(screen.getByRole("textbox", { name: "注意事项（每行一条）" }), { target: { value: "更正" } });
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(save).not.toHaveBeenCalled();
  expect(screen.queryByRole("textbox", { name: "注意事项（每行一条）" })).not.toBeInTheDocument();
});

it("bulk-confirms only unreviewed facts in one action", () => {
  const confirmAll = vi.fn();
  const step = (id: string) => ({ ...original, id });
  const none = { userEdited: null, textOnly: false, editedAt: null, editedBy: null };
  render(<KnowledgeReviewPanel parts={[]} steps={[step("a"), step("b"), step("c")]} specs={[]}
    editing={null} onEditing={vi.fn()} buffers={{}} onBuffer={vi.fn()}
    entityReviews={{ a: { reviewStatus: "confirmed", ...none }, b: { reviewStatus: null, ...none } }}
    busy={false} modelReview={null} model={null} modelLoaded={false} narrow={false}
    onDeclareModelReady={vi.fn()} onDeclareModelConfirmed={vi.fn()} onSetEntityReview={vi.fn()}
    onConfirmAllEntities={confirmAll} onSaveEntityEdit={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "确认全部 2 条文字事实" }));
  expect(confirmAll).toHaveBeenCalledExactlyOnceWith(["b", "c"]);
});

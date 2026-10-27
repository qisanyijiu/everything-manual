/**
 * 说明书问答窗口（阅读器右下角浮动面板）。
 *
 * - 回答由服务端调用说明书 AI，依据该发布版本的知识与原文页文字；对话只保存在本页内存；
 * - 回答里的页码是按钮：点击把原文面板切到该页；
 * - 键盘可达：打开后焦点进入输入框，Esc 关闭并把焦点还给开关；Enter 发送、Shift+Enter 换行；
 * - 新回答用 `aria-live="polite"` 播报；发送中禁止重复提交。
 */

import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";

import { describeError } from "../../api/client";
import { chatRelease, type ChatMessageDto } from "../../api/endpoints";

import "./manual-chat.css";

/** 与服务端上限一致（http/chat.rs MAX_MESSAGES / MAX_MESSAGE_CHARS）。 */
const MAX_MESSAGES = 20;
const MAX_CHARS = 2000;

interface Entry {
  readonly role: "user" | "assistant";
  readonly content: string;
  readonly pages?: readonly number[];
  readonly documentId?: string | null;
}

export interface ManualChatProps {
  readonly itemId: string;
  readonly releaseId: string;
  readonly title: string;
  readonly onOpenPage: (documentId: string | null, pageNumber: number) => void;
}

const SUGGESTIONS = ["怎么充电？", "有哪些安全注意事项？", "主要规格参数是什么？"];

export function ManualChat({ itemId, releaseId, title, onOpenPage }: ManualChatProps) {
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<readonly Entry[]>([]);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const toggleRef = useRef<HTMLButtonElement | null>(null);
  const logRef = useRef<HTMLOListElement | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => () => abortRef.current?.abort(), []);
  useEffect(() => {
    if (open) {
      inputRef.current?.focus();
    }
  }, [open]);
  useEffect(() => {
    const log = logRef.current;
    if (log !== null) {
      log.scrollTop = log.scrollHeight;
    }
  }, [entries, pending]);

  const close = () => {
    setOpen(false);
    toggleRef.current?.focus();
  };

  const send = async (question: string) => {
    const content = question.trim();
    if (content === "" || pending) {
      return;
    }
    const history: Entry[] = [...entries, { role: "user", content }];
    // 超出上限时只保留最近的若干条（必须以用户问题结尾，服务端会校验）。
    const messages: ChatMessageDto[] = history
      .slice(-MAX_MESSAGES)
      .map((entry) => ({ role: entry.role, content: entry.content }));
    while (messages.length > 0 && messages[0]?.role !== "user") {
      messages.shift();
    }
    setEntries(history);
    setDraft("");
    setError(null);
    setPending(true);
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const { data } = await chatRelease(itemId, releaseId, messages, controller.signal);
      setEntries((current) => [
        ...current,
        { role: "assistant", content: data.answer, pages: data.citedPages, documentId: data.documentId ?? null },
      ]);
    } catch (caught) {
      if (controller.signal.aborted) {
        return;
      }
      const described = describeError(caught);
      setError(described.message);
      // 失败的问题放回输入框，方便重试。
      setEntries((current) => current.slice(0, -1));
      setDraft(content);
    } finally {
      if (abortRef.current === controller) {
        abortRef.current = null;
        setPending(false);
      }
    }
  };

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    void send(draft);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void send(draft);
    }
  };

  return (
    <div className="manual-chat">
      {open && (
        <section
          className="manual-chat__panel"
          role="dialog"
          aria-modal="false"
          aria-labelledby="manual-chat-title"
          data-testid="manual-chat"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              close();
            }
          }}
        >
          <header className="manual-chat__header">
            <h2 id="manual-chat-title">问说明书</h2>
            <button type="button" onClick={close} aria-label="关闭问答窗口">关闭</button>
          </header>
          <p className="manual-chat__scope">依据「{title}」的发布内容回答；结果仅供参考，请以原文为准。</p>
          <ol className="manual-chat__log" ref={logRef} aria-live="polite" data-testid="manual-chat-log">
            {entries.length === 0 && (
              <li className="manual-chat__empty">
                <p>可以问操作步骤、部件用途或规格参数，例如：</p>
                <div className="manual-chat__suggestions">
                  {SUGGESTIONS.map((suggestion) => (
                    <button key={suggestion} type="button" className="interaction-chip" onClick={() => void send(suggestion)}>
                      {suggestion}
                    </button>
                  ))}
                </div>
              </li>
            )}
            {entries.map((entry, index) => (
              <li
                key={index}
                className={`manual-chat__message manual-chat__message--${entry.role}`}
                data-testid={`manual-chat-${entry.role}`}
              >
                <span className="manual-chat__who">{entry.role === "user" ? "我" : "说明书助手"}</span>
                <p>{entry.content}</p>
                {entry.pages !== undefined && entry.pages.length > 0 && (
                  <p className="manual-chat__pages">
                    原文出处：
                    {entry.pages.map((page) => (
                      <button
                        key={page}
                        type="button"
                        className="step-parts__chip"
                        onClick={() => onOpenPage(entry.documentId ?? null, page)}
                        data-testid={`manual-chat-page-${page}`}
                      >
                        第 {page} 页
                      </button>
                    ))}
                  </p>
                )}
              </li>
            ))}
            {pending && (
              <li className="manual-chat__message manual-chat__message--assistant" role="status">
                <span className="manual-chat__who">说明书助手</span>
                <p>正在查阅说明书…</p>
              </li>
            )}
          </ol>
          {error !== null && (
            <p className="manual-chat__error" role="alert" data-testid="manual-chat-error">
              {error}
            </p>
          )}
          <form className="manual-chat__form" onSubmit={onSubmit}>
            <label htmlFor="manual-chat-input" className="visually-hidden">输入问题</label>
            <textarea
              id="manual-chat-input"
              ref={inputRef}
              value={draft}
              rows={2}
              maxLength={MAX_CHARS}
              placeholder="输入问题，Enter 发送，Shift+Enter 换行"
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={onKeyDown}
              data-testid="manual-chat-input"
            />
            <button type="submit" className="button-primary" aria-disabled={pending || draft.trim() === ""} disabled={pending || draft.trim() === ""}>
              {pending ? "回答中…" : "发送"}
            </button>
          </form>
        </section>
      )}
      <button
        ref={toggleRef}
        type="button"
        className="manual-chat__toggle button-primary"
        aria-expanded={open}
        onClick={() => (open ? close() : setOpen(true))}
        data-testid="manual-chat-toggle"
      >
        {open ? "收起问答" : "问说明书"}
      </button>
    </div>
  );
}

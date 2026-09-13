/**
 * 未实现路由的占位页（T08 卡：先建路由骨架与占位态）。
 * 明确写「尚未实现」与计划交付的切片，**不用 mock 数据假装业务完成**，
 * 也不提供会静默失败的操作按钮。
 *
 * 向导步骤条（`WizardSteps`/`WizardNav`）已迁到 `features/import/WizardSteps.tsx`
 * （T16 起由向导各步共用）。
 */

import { Link } from "react-router";
import type { ReactNode } from "react";

export interface PlaceholderPageProps {
  readonly title: string;
  readonly taskCard: string;
  readonly description: ReactNode;
}

export function PlaceholderPage({ title, taskCard, description }: PlaceholderPageProps) {
  return (
    <section className="page placeholder-page" aria-labelledby="placeholder-title">
      <h1 id="placeholder-title">{title}</h1>
      <div className="placeholder-page__notice">
        <p>
          <strong>该页面尚未实现。</strong>
          {description}
        </p>
        <p className="placeholder-page__meta">
          计划在 {taskCard} 交付；在此之前本页没有任何可提交的操作，也不会请求或写入业务数据。
        </p>
      </div>
      <p>
        <Link to="/">返回资料库</Link>
      </p>
    </section>
  );
}

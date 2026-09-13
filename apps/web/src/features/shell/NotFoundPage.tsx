import { Link } from "react-router";

/** 未知路由：明确 404 说明 + 安全返回路径（不渲染空白页）。 */
export function NotFoundPage() {
  return (
    <section className="page" aria-labelledby="not-found-title">
      <h1 id="not-found-title">页面不存在</h1>
      <p className="page__lead">该地址没有对应的页面；可能是链接已失效或 URL 输入有误。</p>
      <p>
        <Link to="/">返回资料库</Link>
      </p>
    </section>
  );
}

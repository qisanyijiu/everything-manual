# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: qa-t09-tmp-failure.spec.ts >> 临时失败用例：Playwright 必须保留失败证据
- Location: tests/e2e/qa-t09-tmp-failure.spec.ts:4:1

# Error details

```
Error: 故意失败：验证失败证据保留

expect(received).toBe(expected) // Object.is equality

Expected: 2
Received: 1
```

# Test source

```ts
  1 | /** 临时（QA 回合 9）：验证 test:e2e 失败时保留 trace/截图；跑完即删除。 */
  2 | import { expect, test } from "@playwright/test";
  3 | 
  4 | test("临时失败用例：Playwright 必须保留失败证据", async () => {
> 5 |   expect(1, "故意失败：验证失败证据保留").toBe(2);
    |                              ^ Error: 故意失败：验证失败证据保留
  6 | });
  7 | 
```
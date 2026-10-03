import fs from "node:fs";
import path from "node:path";
import { expect, test } from "@playwright/test";
import { ApiSettingsQaBackend, CheckedProviderFixture, QA_PASSWORD, attachQaBackend } from "./api-settings-qa-harness";
import { loginViaUi } from "./helpers";
import { REPO_ROOT } from "./runtime";

test("PC06 hidden legacy model, keyboard correction, masking and real restart at 375/1440", async ({ page }) => {
  const canary = "sk-pc06_browser_fake_0123456789";
  const deployment = new CheckedProviderFixture({ tripo: "pc06-tripo", manualAi: canary });
  const corrected = new CheckedProviderFixture({ tripo: "pc06-tripo", manualAi: "org/custom-model" });
  const backend = new ApiSettingsQaBackend(deployment, corrected);
  const evidence = path.join(REPO_ROOT, "artifacts/prd-completion/pc06-rd");
  await Promise.all([deployment.start(), corrected.start()]);
  try {
    await backend.start();
    const external = await attachQaBackend(page, backend);
    await loginViaUi(page, "", QA_PASSWORD);
    await page.goto("/settings");
    const model = page.getByLabel("说明书 AI 模型", { exact: true });
    await expect(model).toHaveValue("");
    await expect(page.getByRole("button", { name: "保存配置", exact: true })).toBeDisabled();
    fs.mkdirSync(evidence, { recursive: true });
    const geometry = [];
    for (const width of [375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      const values = await page.evaluate(() => ({ viewport: innerWidth, scrollWidth: document.documentElement.scrollWidth }));
      expect(values.scrollWidth - values.viewport).toBeLessThanOrEqual(1);
      const clear = page.getByRole("button", { name: "清空误填模型", exact: true });
      await clear.focus(); await page.keyboard.press("Enter");
      const cancel = page.getByRole("button", { name: "取消", exact: true });
      await expect(cancel).toBeFocused();
      const confirm = page.getByRole("button", { name: "确认清空模型", exact: true });
      for (const action of [cancel, confirm]) {
        const box = await action.boundingBox();
        expect(box?.height).toBeGreaterThanOrEqual(44);
        expect(box?.width).toBeGreaterThanOrEqual(44);
      }
      await page.screenshot({ path: path.join(evidence, `correction-${width}.png`), fullPage: true });
      await page.keyboard.press("Tab"); await expect(confirm).toBeFocused(); await page.keyboard.press("Space");
      await expect(model).toBeFocused();
      await page.getByRole("button", { name: "撤销清空" }).click();
      await expect(model).toBeFocused();
      geometry.push(values);
    }
    let puts = 0;
    page.on("request", (r) => { if (r.method() === "PUT" && r.url().includes("/settings/providers")) puts += 1; });
    await model.fill(canary);
    await expect(model).toHaveAttribute("type", "password");
    await expect(model).toHaveAttribute("aria-invalid", "true");
    expect(await page.evaluate((value) => [document.documentElement.outerHTML, location.href, JSON.stringify(localStorage), JSON.stringify(sessionStorage)].some((v) => v.includes(value)), canary)).toBe(false);
    await page.getByRole("button", { name: "保存配置", exact: true }).click();
    await expect(model).toBeFocused(); expect(puts).toBe(0);
    await model.fill(corrected.models.manualAi);
    await page.getByRole("button", { name: "保存配置", exact: true }).click();
    await expect(page.getByText("已保存，重启服务后生效", { exact: true }).first()).toBeVisible();
    await expect(page.getByText("模型需修正（疑似误填密钥，已隐藏）", { exact: true })).toBeVisible();
    await backend.restart();
    await page.reload();
    await expect(model).toHaveValue(corrected.models.manualAi);
    await expect(page.getByText("模型需修正（疑似误填密钥，已隐藏）", { exact: true })).toHaveCount(0);
    expect(puts).toBe(1);
    expect(deployment.checks.tripoRequests + deployment.checks.manualRequests + corrected.checks.tripoRequests + corrected.checks.manualRequests).toBe(0);
    expect(external).toEqual([]);
    await backend.stop();
    const leaked = backend.logsContainSecret() || fs.readFileSync(backend.logPath, "utf8").includes(canary);
    expect(leaked).toBe(false);
    fs.writeFileSync(path.join(evidence, "summary.json"), JSON.stringify({ geometry, browserLeak: false, logLeak: false, providerRequests: 0, writes: puts, restartVerified: true }, null, 2));
  } finally {
    await backend.cleanup();
    await Promise.all([deployment.stop(), corrected.stop()]);
  }
});

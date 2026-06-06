import { expect, test } from "@playwright/test";
import { seedLargeWorkspaceFixture } from "./helpers/workspace";

test("large workspace keeps session state light and loads large content on demand", async ({ page }) => {
  const seeded = await seedLargeWorkspaceFixture(page);
  const sessionStatePayloads: string[] = [];
  const promptResponses: string[] = [];
  const outputResponses: string[] = [];
  const artifactResponses: string[] = [];

  page.on("websocket", (socket) => {
    socket.on("framereceived", (frame) => {
      const payload = typeof frame.payload === "string" ? frame.payload : frame.payload.toString();
      if (payload.includes('"type":"session_state"')) {
        sessionStatePayloads.push(payload);
      }
    });
  });
  page.on("response", async (response) => {
    const url = response.url();
    if (!response.ok()) {
      return;
    }
    if (url.includes("/prompt")) {
      promptResponses.push(await response.text());
    } else if (url.includes("/events/") && url.includes("/output")) {
      outputResponses.push(await response.text());
    } else if (url.includes("/artifact-versions/")) {
      artifactResponses.push(await response.text());
    }
  });

  await page.goto(`/workbench/workspace/${seeded.sessionId}`);
  const chatEntryList = page.getByTestId("chat-entry-list");
  await expect(chatEntryList).toBeVisible();
  await expect(page.getByTestId("timeline-node-list")).toBeVisible();
  await page.getByTestId("timeline-node-author_run").first().click();
  await expect(chatEntryList).toContainText("Provider Prompt");
  await expect(chatEntryList).toContainText("Execution Output");

  await expect.poll(() => sessionStatePayloads.length).toBeGreaterThan(0);
  expect(sessionStatePayloads.join("\n")).not.toContain("完整提示词 large-prompt-0");
  expect(sessionStatePayloads.join("\n")).not.toContain("完整输出 large-output-0");

  const promptLoad = page.waitForResponse(
    (response) => response.url().includes("/timeline-node-details/timeline_node_034/prompt") && response.ok(),
  );
  await chatEntryList.getByRole("button", { name: /Provider Prompt/ }).first().click();
  const promptResponse = await promptLoad;
  expect(await promptResponse.text()).toContain("完整提示词 large-prompt-0");
  await expect(chatEntryList).toContainText("完整提示词 large-prompt-0", { timeout: 30_000 });
  await expect.poll(() => promptResponses.join("\n")).toContain("完整提示词 large-prompt-0");

  const outputLoad = page.waitForResponse(
    (response) => response.url().includes("/timeline_node_034_output/output") && response.ok(),
  );
  await chatEntryList.getByRole("button", { name: /Execution Output/ }).first().click();
  const outputResponse = await outputLoad;
  expect(await outputResponse.text()).toContain("完整输出 large-output-0");
  await expect(chatEntryList).toContainText("完整输出 large-output-0", { timeout: 30_000 });
  await expect.poll(() => outputResponses.join("\n")).toContain("完整输出 large-output-0");

  const domNodeCount = await page.evaluate(() => document.querySelectorAll("*").length);
  expect(domNodeCount).toBeLessThan(3000);

  await page.getByRole("button", { name: "Artifact" }).click();
  await expect(page.getByTestId("artifact-pane")).toContainText("Artifact");
  await expect.poll(() => artifactResponses.join("\n")).toContain("Large Artifact v5");
  expect(artifactResponses.join("\n")).toContain("Large Artifact v5");
  expect(sessionStatePayloads.join("\n")).not.toContain("Large Artifact v5");
});

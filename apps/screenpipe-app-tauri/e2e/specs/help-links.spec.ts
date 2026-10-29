// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

// The Help section points at the project's GitHub pages (docs, discussions,
// issues, releases). Upstream also linked a vendor Discord server, a booking
// page and a hosted ideas board; none of those may come back.

import { existsSync } from "node:fs";
import { saveScreenshot } from "../helpers/screenshot-utils.js";
import { openHomeWindow, t, waitForAppReady } from "../helpers/test-utils.js";

async function openHelpSection(): Promise<void> {
  const navHelp = await $('[data-testid="nav-help"]');
  await navHelp.waitForExist({ timeout: t(12_000) });
  await navHelp.click();

  const sectionHelp = await $('[data-testid="section-help"]');
  await sectionHelp.waitForExist({ timeout: t(20_000) });
}

describe("Help section: project links", function () {
  this.timeout(120_000);

  before(async () => {
    await waitForAppReady();
    await openHomeWindow();
    await openHelpSection();
  });

  it("offers docs, discussions, issues and the changelog", async () => {
    for (const id of ["docs", "discussions", "ideas", "issues", "changelog"]) {
      const button = await $(`[data-testid="help-${id}-link"]`);
      await button.waitForExist({ timeout: t(10_000) });
      expect(await button.isEnabled()).toBe(true);
    }

    const filepath = await saveScreenshot("help-links");
    expect(existsSync(filepath)).toBe(true);
  });

  it("shows no vendor community or booking links", async () => {
    expect(await $('[data-testid="help-discord-link"]').isExisting()).toBe(false);
    const text = ((await browser.execute(
      () => document.querySelector('[data-testid="section-help"]')?.textContent ?? "",
    )) as string).toLowerCase();
    for (const banned of ["discord", "book a call", "youtube", "screenpi.pe", "screenpipe.com"]) {
      expect(text).not.toContain(banned);
    }
  });
});

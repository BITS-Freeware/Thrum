/// End-to-end test of the web client against a real server: two users
/// register, chat live over WebSocket, form a group, and share an
/// encrypted attachment whose downloaded bytes must match the original.

import { expect, test, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

const ALICE = "alice";
const BOB = "bob";
const PASSWORD = "correct-horse-battery-staple";

/** Register a fresh account through the UI and land on the chat list. */
async function register(page: Page, username: string): Promise<void> {
  await page.goto("/");
  // Log console messages and network activity during registration
  const consoleMessages: string[] = [];
  page.on("console", (msg) => consoleMessages.push(`${msg.type()}: ${msg.text()}`));
  page.on("pageerror", (err) => consoleMessages.push(`pageerror: ${err.message}`));
  const networkLog: string[] = [];
  page.on("request", (req) => networkLog.push(`REQ ${req.method()} ${req.url()}`));
  page.on("response", (res) => networkLog.push(`RES ${res.status()} ${res.url()}`));
  page.on("requestfailed", (req) => networkLog.push(`FAIL ${req.method()} ${req.url()} :: ${req.failure()?.errorText ?? "?"}`));
  const logsBefore = consoleMessages.length;

  await page.getByRole("button", { name: "Create an account" }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();

  // Wait for either success (Chats heading) or error message
  const chatsHeading = page.getByRole("heading", { name: "Chats" });
  const errorMsg = page.locator(".error");

  try {
    await expect(
      Promise.race([
        chatsHeading.waitFor({ state: "visible", timeout: 30_000 }).then(() => "ok"),
        errorMsg.waitFor({ state: "visible", timeout: 30_000 }).then(async () => {
          const text = await errorMsg.textContent();
          throw new Error(`Registration error: ${text}`);
        }),
      ]),
    ).resolves.toBe("ok");
  } catch (caught) {
    console.log("Console logs during registration:", consoleMessages.slice(logsBefore));
    console.log("Network activity during registration:", networkLog);
    throw caught;
  }
}

async function openConversation(page: Page, label: string): Promise<void> {
  // Click the conversation link directly (no "Chats" nav link in current UI)
  await page.getByRole("link", { name: label, exact: true }).click();
  await expect(page.getByRole("heading", { name: label }).first()).toBeVisible();
}

test("two users chat live, form a group, and share an encrypted attachment", async ({
  browser,
}) => {
  const aliceContext = await browser.newContext();
  const bobContext = await browser.newContext();
  const alice = await aliceContext.newPage();
  const bob = await bobContext.newPage();

  await register(alice, ALICE);
  await register(bob, BOB);

  // --- direct messages ---

  await alice.getByRole("link", { name: "New chat" }).click();
  await alice
    .getByLabel("Members (comma-separated usernames)")
    .fill(BOB);
  await alice.getByRole("button", { name: "Create" }).click();
  // The DM is unnamed, so it is labeled by the other member's username.
  await expect(alice.getByRole("heading", { name: BOB, exact: true })).toBeVisible();

  await alice.getByLabel("message text").fill("hello bob");
  await alice.getByRole("button", { name: "Send" }).click();
  await expect(alice.getByText("hello bob")).toBeVisible();

  // Bob sees the conversation appear live and reads the message.
  await expect(
    bob.getByRole("link", { name: ALICE, exact: true }),
  ).toBeVisible({ timeout: 20_000 });
  await openConversation(bob, ALICE);
  await expect(bob.getByText("hello bob")).toBeVisible({ timeout: 20_000 });

  // Bob replies; alice sees the reply without reloading.
  await bob.getByLabel("message text").fill("hi alice");
  await bob.getByRole("button", { name: "Send" }).click();
  await expect(alice.getByText("hi alice")).toBeVisible({ timeout: 20_000 });

  // --- group ---

  await alice.getByRole("link", { name: "Chats", exact: true }).click();
  await alice.getByRole("link", { name: "New chat" }).click();
  await alice
    .getByLabel("Members (comma-separated usernames)")
    .fill(BOB);
  await alice.getByLabel("Group name (optional)").fill("group1");
  await alice.getByRole("button", { name: "Create" }).click();
  await expect(alice.getByRole("heading", { name: "group1", exact: true })).toBeVisible({ timeout: 10_000 });

  await alice.getByLabel("message text").fill("group hello");
  await alice.getByRole("button", { name: "Send" }).click();
  await expect(alice.getByText("group hello")).toBeVisible();

  // Bob, sitting on the chat list, sees the group appear live.
  await bob.getByRole("link", { name: "Chats", exact: true }).click();
  await expect(
    bob.getByRole("link", { name: "group1", exact: true }),
  ).toBeVisible({ timeout: 20_000 });
  await openConversation(bob, "group1");
  await expect(bob.getByText("group hello")).toBeVisible({ timeout: 20_000 });

  // Group details: the member list shows both users and renaming works.
  await alice
    .getByRole("link", { name: "Details", exact: true })
    .click();
  await expect(
    alice.getByRole("listitem").filter({ hasText: BOB }),
  ).toBeVisible();
  await alice.getByLabel("Rename group").fill("group2");
  await alice.getByRole("button", { name: "Rename" }).click();
  await expect(alice.getByRole("heading", { name: "group2", exact: true })).toBeVisible();

  // --- attachment ---

  const payload = `geist3 e2e attachment\nline ${"x".repeat(200)}\n`.repeat(50);
  const filePath = path.join(os.tmpdir(), "sample note.txt");
  fs.writeFileSync(filePath, payload, "utf8");

  // Back out of the group screens to the chat lists first.
  await alice.getByRole("link", { name: "Back", exact: true }).click();
  await alice.getByRole("link", { name: "Chats", exact: true }).click();
  await openConversation(alice, BOB);
  await bob.getByRole("link", { name: "Chats", exact: true }).click();
  await openConversation(bob, ALICE);
  await alice.setInputFiles('input[type="file"]', filePath);
  await expect(
    alice.getByRole("link", { name: /sample note\.txt/ }),
  ).toBeVisible({ timeout: 20_000 });

  // The attachment reaches bob live; he downloads it and the bytes match.
  await expect(
    bob.getByRole("link", { name: /sample note\.txt/ }),
  ).toBeVisible({ timeout: 20_000 });
  const downloadPromise = bob.waitForEvent("download");
  await bob.getByRole("link", { name: /sample note\.txt/ }).click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe("sample note.txt");
  const received = fs.readFileSync(await download.path(), "utf8");
  expect(received).toBe(payload);

  await aliceContext.close();
  await bobContext.close();
});

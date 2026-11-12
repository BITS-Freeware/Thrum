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
  await page.getByRole("button", { name: "Create an account" }).click();
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("heading", { name: "Chats" })).toBeVisible();
}

async function openConversation(page: Page, label: string): Promise<void> {
  await page.getByRole("link", { name: "Chats", exact: true }).click();
  await page.getByRole("link", { name: label, exact: true }).click();
  await expect(page.getByRole("heading", { name: label, exact: true })).toBeVisible();
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
  await expect(alice.getByRole("heading", { name: BOB, exact: true })).toBeVisible();

  await alice.getByLabel("message text").fill("hello bob");
  await alice.getByRole("button", { name: "Send" }).click();
  await expect(alice.getByText("hello bob")).toBeVisible();

  // Bob sees the conversation appear live and reads the message.
  await expect(
    bob.getByRole("link", { name: ALICE, exact: true }),
  ).toBeVisible({ timeout: 20_000 });
  await openConversation(bob, ALICE);
  await expect(bob.getByText("hello bob")).toBeVisible();

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
  await alice.getByLabel("Group name (optional)").fill("testers");
  await alice.getByRole("button", { name: "Create" }).click();
  await expect(alice.getByRole("heading", { name: "testers", exact: true })).toBeVisible();

  await alice.getByLabel("message text").fill("group hello");
  await alice.getByRole("button", { name: "Send" }).click();
  await expect(alice.getByText("group hello")).toBeVisible();

  // Bob, sitting on the chat list, sees the group appear live.
  await bob.getByRole("link", { name: "Chats", exact: true }).click();
  await expect(
    bob.getByRole("link", { name: "testers", exact: true }),
  ).toBeVisible({ timeout: 20_000 });
  await openConversation(bob, "testers");
  await expect(bob.getByText("group hello")).toBeVisible();

  // Group details: the member list shows both users and renaming works.
  await alice
    .getByRole("link", { name: "Details", exact: true })
    .click();
  await expect(
    alice.getByRole("listitem").filter({ hasText: BOB }),
  ).toBeVisible();
  await alice.getByLabel("Rename group").fill("testers2");
  await alice.getByRole("button", { name: "Rename" }).click();
  await expect(alice.getByRole("heading", { name: "testers2", exact: true })).toBeVisible();

  // --- attachment ---

  const payload = `thrum e2e attachment\nline ${"x".repeat(200)}\n`.repeat(50);
  const filePath = path.join(os.tmpdir(), "thrum-e2e-note.txt");
  fs.writeFileSync(filePath, payload, "utf8");

  await openConversation(alice, BOB);
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

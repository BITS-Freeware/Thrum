import { defineConfig, devices } from "@playwright/test";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Two processes back the e2e tests: the Geist3 server on 8080 (fresh
// database every run) and the Vite dev server on 5173, which proxies
// /v1 to the server. The server binary must be built first:
//   cargo build -p geist3-server
const serverDb = join(tmpdir(), "geist3-playwright.sqlite");

export default defineConfig({
  testDir: "./tests",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    {
      command: 'rm -f "$GEIST3_DB" && exec ../target/debug/geist3-server',
      port: 8080,
      env: { GEIST3_DB: serverDb, GEIST3_PORT: "8080" },
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      command: "npx vite --port 5173 --strictPort --host 127.0.0.1",
      port: 5173,
      env: { GEIST3_SERVER: "http://127.0.0.1:8080" },
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});

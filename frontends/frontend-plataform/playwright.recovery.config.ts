import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/realtime-recovery.spec.ts",
  fullyParallel: false,
  timeout: 120_000,
  reporter: "list",
  use: { baseURL: process.env.PAYVIA_E2E_FRONTEND_ORIGIN ?? "http://localhost:4173", ...devices["Desktop Chrome"] },
});

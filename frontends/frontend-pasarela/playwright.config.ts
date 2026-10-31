import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  timeout: 90_000,
  reporter: "list",
  use: { baseURL: process.env.PAYVIA_E2E_FRONTEND_ORIGIN ?? "http://localhost:4173", ...devices["Desktop Chrome"] },
});

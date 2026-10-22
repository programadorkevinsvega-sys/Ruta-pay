import { expect, test } from "@playwright/test";

test("comercio completa onboarding en el navegador a través del BFF real", async ({ page }) => {
  await page.addInitScript(() => {
    const state = window as Window & { payviaRealtimeEvents?: string[] };
    state.payviaRealtimeEvents = [];
    window.addEventListener("payvia:realtime-event", (event) => {
      state.payviaRealtimeEvents?.push((event as CustomEvent<{ event_type: string }>).detail.event_type);
    });
  });
  const responses: number[] = [];
  page.on("response", (response) => { if (response.url().includes("/v1/")) responses.push(response.status()); });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Iniciar sesión" })).toBeVisible();
  await page.getByRole("button", { name: "Crear cuenta de comercio" }).click();
  const suffix = crypto.randomUUID().replaceAll("-", "");
  await page.getByLabel("Nombre del comercio").fill(`E2E Comercio ${suffix}`);
  await page.getByLabel("Identificador").fill(`e2e-${suffix}`);
  await page.getByLabel("Correo").fill(`e2e-${suffix}@example.test`);
  await page.getByLabel("Contraseña").fill(`E2e-password-${suffix}Aa!`);
  await page.getByRole("button", { name: "Crear cuenta y comercio" }).click();
  await expect(page.getByRole("heading", { name: "Sesión autenticada" })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText(`E2E Comercio ${suffix}`)).toBeVisible({ timeout: 45_000 });
  await expect(page.getByText("tenant_owner · active")).toBeVisible({ timeout: 45_000 });
  await expect(page.getByText(/Tiempo real: conectado/)).toBeVisible({ timeout: 45_000 });
  await expect.poll(() => page.evaluate(() => (window as Window & { payviaRealtimeEvents?: string[] }).payviaRealtimeEvents?.includes("tenant.registered") ?? false), { timeout: 45_000 }).toBe(true);
  expect(responses).toContain(201);
  expect(responses).toContain(200);
});

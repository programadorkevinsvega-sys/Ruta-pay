import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";

const credentialsPath = process.env.E2E_OPERATOR_CREDENTIALS;

test("operador crea y administra tenants a través del BFF real", async ({ page, request }) => {
  if (!credentialsPath) throw new Error("E2E_OPERATOR_CREDENTIALS is required");
  await page.addInitScript(() => {
    const state = window as Window & { payviaRealtimeEvents?: string[] };
    state.payviaRealtimeEvents = [];
    window.addEventListener("payvia:realtime-event", (event) => {
      state.payviaRealtimeEvents?.push((event as CustomEvent<{ event_type: string }>).detail.event_type);
    });
  });
  const responses: number[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/v1/")) responses.push(response.status());
  });
  const suffix = crypto.randomUUID().replaceAll("-", "");
  const email = `operator-tenant-${suffix}@example.test`;
  const password = `E2e-password-${suffix}Aa!`;
  const tenantName = `E2E Admin ${suffix}`;
  const slug = `e2e-admin-${suffix}`;
  const legalIdentificationNumber = `9${Date.now()}${Math.floor(Math.random() * 1_000_000).toString().padStart(6, "0")}`.slice(0, 20);
  const origin = process.env.PAYVIA_E2E_FRONTEND_ORIGIN ?? "http://localhost:4173";

  const registration = await request.post("/v1/identity/register", {
    headers: { Origin: origin, "X-Correlation-ID": crypto.randomUUID() },
    data: { email, password },
  });
  expect(registration.status(), await registration.text()).toBe(201);
  const tenantLogin = await request.post("/v1/identity/session", {
    headers: { Origin: origin, "X-Correlation-ID": crypto.randomUUID() },
    data: { email, password },
  });
  expect(tenantLogin.status()).toBe(200);
  const tenantCookies = await request.storageState();
  const csrfCookie = tenantCookies.cookies.find((cookie) => cookie.name === "payvia_tenant_csrf");
  expect(csrfCookie).toBeDefined();
  const tenantCreation = await request.post("/v1/tenants", {
    headers: { Origin: origin, "X-CSRF-Token": csrfCookie!.value, "X-Correlation-ID": crypto.randomUUID() },
    data: {
      display_name: tenantName,
      slug,
      legal_identification_type: "nit",
      legal_identification_number: legalIdentificationNumber,
    },
  });
  expect(tenantCreation.status(), await tenantCreation.text()).toBe(201);
  const tenant = await tenantCreation.json() as { tenant_id: string };

  const operator = JSON.parse(await readFile(credentialsPath, "utf8")) as { email: string; password: string };
  await page.goto("/");
  await page.getByLabel("Correo").fill(operator.email);
  await page.getByLabel("Contraseña").fill(operator.password);
  await page.getByRole("button", { name: "Iniciar sesión" }).click();
  await expect(page.getByRole("heading", { name: "Sesión de operador activa" })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText(/Tiempo real: conectado/)).toBeVisible({ timeout: 45_000 });
  await expect(page.getByText(tenantName)).toBeVisible({ timeout: 10_000 });
  await page.getByLabel("Motivo de la operación administrativa").fill("Verificación E2E conforme a expediente generado por la prueba");
  await page.getByRole("button", { name: "Verificar y activar" }).click();
  await expect(page.getByText(tenantName).locator("..")).toContainText("active", { timeout: 10_000 });
  await expect.poll(() => page.evaluate(() => (window as Window & { payviaRealtimeEvents?: string[] }).payviaRealtimeEvents?.includes("tenant.status.changed") ?? false), { timeout: 45_000 }).toBe(true);
  const tenantRow = page.locator(".tenant-row").filter({ hasText: tenantName });
  const transitions = [
    { action: "Suspender", status: "suspended", reason: "Prueba de suspensión administrativa" },
    { action: "Reactivar", status: "active", reason: "Prueba de reactivación administrativa" },
    { action: "Iniciar cierre", status: "closing", reason: "Prueba de inicio de cierre administrativo" },
    { action: "Confirmar cierre", status: "closed", reason: "Prueba de cierre administrativo" },
  ];
  for (const [index, transition] of transitions.entries()) {
    await page.getByLabel("Motivo de la operación administrativa").fill(transition.reason);
    await tenantRow.getByRole("button", { name: transition.action }).click();
    await expect(tenantRow).toContainText(transition.status, { timeout: 10_000 });
    await expect.poll(() => page.evaluate(() => (window as Window & { payviaRealtimeEvents?: string[] }).payviaRealtimeEvents?.filter((event) => event === "tenant.status.changed").length ?? 0), { timeout: 45_000 }).toBe(index + 2);
  }
  expect(responses).toContain(200);
  expect(responses).toContain(401);
  expect(tenant.tenant_id).toMatch(/^[0-9a-f-]{36}$/i);
});

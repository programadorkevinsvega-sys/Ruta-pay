import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { expect, test } from "@playwright/test";

const credentialsPath = process.env.E2E_OPERATOR_CREDENTIALS;

test("el BFF recupera realtime tras reiniciar Redis y JetStream", async ({ browser, page, request }) => {
  if (!credentialsPath) throw new Error("E2E_OPERATOR_CREDENTIALS is required");
  console.info("recovery: register tenant through BFF");
  expect(process.env.PAYVIA_EVENT_RESTART_ACCEPTANCE).toBe("1");
  const responses: number[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/v1/")) responses.push(response.status());
  });
  await page.addInitScript(() => {
    const state = window as Window & { payviaRealtimeEvents?: string[] };
    state.payviaRealtimeEvents = [];
    window.addEventListener("payvia:realtime-event", (event) => {
      state.payviaRealtimeEvents?.push((event as CustomEvent<{ event_type: string }>).detail.event_type);
    });
  });
  const secondReplicaContext = await browser.newContext();
  const secondReplicaPage = await secondReplicaContext.newPage();
  await secondReplicaPage.addInitScript(() => {
    const state = window as Window & { payviaRealtimeEvents?: string[] };
    state.payviaRealtimeEvents = [];
    window.addEventListener("payvia:realtime-event", (event) => {
      state.payviaRealtimeEvents?.push((event as CustomEvent<{ event_type: string }>).detail.event_type);
    });
  });

  const suffix = crypto.randomUUID().replaceAll("-", "");
  const userEmail = `recovery-${suffix}@example.test`;
  const userPassword = `Recovery-password-${suffix}Aa!`;
  const tenantName = `Recovery tenant ${suffix}`;
  const tenantSlug = `recovery-${suffix}`;
  const origin = process.env.PAYVIA_E2E_FRONTEND_ORIGIN ?? "http://localhost:4173";
  const registration = await request.post("/v1/identity/register", {
    headers: { Origin: origin, "X-Correlation-ID": crypto.randomUUID() },
    data: { email: userEmail, password: userPassword },
  });
  expect(registration.status()).toBe(201);
  const login = await request.post("/v1/identity/session", {
    headers: { Origin: origin, "X-Correlation-ID": crypto.randomUUID() },
    data: { email: userEmail, password: userPassword },
  });
  expect(login.status()).toBe(200);
  const state = await request.storageState();
  const csrf = state.cookies.find((cookie) => cookie.name === "payvia_tenant_csrf");
  expect(csrf).toBeDefined();
  const creation = await request.post("/v1/tenants", {
    headers: { Origin: origin, "X-CSRF-Token": csrf!.value, "X-Correlation-ID": crypto.randomUUID() },
    data: { display_name: tenantName, slug: tenantSlug },
  });
  expect(creation.status(), await creation.text()).toBe(201);

  console.info("recovery: authenticate operator on both BFF replicas");
  const operator = JSON.parse(await readFile(credentialsPath, "utf8")) as { email: string; password: string };
  const websocket = page.waitForEvent("websocket");
  await page.goto("/");
  console.info("recovery: primary login screen loaded");
  await page.getByLabel("Correo").fill(operator.email);
  await page.getByLabel("Contraseña").fill(operator.password);
  await page.getByRole("button", { name: "Iniciar sesión" }).click();
  console.info("recovery: primary login submitted");
  await expect(page.getByRole("heading", { name: "Sesión de operador activa" })).toBeVisible({ timeout: 30_000 });
  console.info("recovery: primary session established");
  await expect(page.getByText(tenantName)).toBeVisible({ timeout: 30_000 });
  console.info("recovery: tenant loaded on primary");
  await websocket;
  console.info("recovery: primary websocket connected");
  const secondReplicaWebsocket = secondReplicaPage.waitForEvent("websocket");
  await secondReplicaPage.goto(process.env.PAYVIA_E2E_SECOND_FRONTEND_ORIGIN ?? "http://localhost:4174/");
  console.info("recovery: secondary login screen loaded");
  await secondReplicaPage.getByLabel("Correo").fill(operator.email);
  await secondReplicaPage.getByLabel("Contraseña").fill(operator.password);
  await secondReplicaPage.getByRole("button", { name: "Iniciar sesión" }).click();
  console.info("recovery: secondary login submitted");
  await expect(secondReplicaPage.getByRole("heading", { name: "Sesión de operador activa" })).toBeVisible({ timeout: 30_000 });
  console.info("recovery: secondary session established");
  await expect(secondReplicaPage.getByText(tenantName)).toBeVisible({ timeout: 30_000 });
  console.info("recovery: tenant loaded on secondary");
  await secondReplicaWebsocket;
  console.info("recovery: secondary websocket connected");

  console.info("recovery: restart Redis and verify cross-replica event delivery");
  restartDevelopmentContainer("payvia-shared-infra-redis-1");
  await waitForTcpEndpoint(process.env.PLATFORM_REDIS_URL ?? "");
  await page.waitForTimeout(3_000);
  await page.getByLabel("Motivo de la operación administrativa").fill("Validación de reconexión Redis en infraestructura local");
  await page.getByRole("button", { name: "Verificar y activar" }).click();
  await expect(page.getByText(tenantName).locator("..")).toContainText("active", { timeout: 30_000 });
  await expectRealtimeEvent(page, 1);
  await expect(secondReplicaPage.getByText(tenantName).locator("..")).toContainText("active", { timeout: 30_000 });
  await expectRealtimeEventOn(secondReplicaPage, 1);

  console.info("recovery: restart JetStream and verify durable event delivery");
  restartDevelopmentContainer("payvia-shared-infra-nats-1");
  await waitForTcpEndpoint(process.env.PLATFORM_NATS_URL ?? "");
  await page.waitForTimeout(5_000);
  await page.getByLabel("Motivo de la operación administrativa").fill("Validación de recuperación JetStream en infraestructura local");
  await page.getByRole("button", { name: "Suspender" }).click();
  await expect(page.getByText(tenantName).locator("..")).toContainText("suspended", { timeout: 30_000 });
  await expectRealtimeEvent(page, 2);
  await expect(secondReplicaPage.getByText(tenantName).locator("..")).toContainText("suspended", { timeout: 30_000 });
  await expectRealtimeEventOn(secondReplicaPage, 2);
  console.info("recovery: complete");
  expect(responses).toContain(200);
  await secondReplicaContext.close();
});

function restartDevelopmentContainer(containerName: string) {
  const output = execFileSync("docker", ["restart", containerName], {
    encoding: "utf8",
    timeout: 45_000,
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
  expect(output).toBe(containerName);
}

async function waitForTcpEndpoint(endpoint: string) {
  const parsed = new URL(endpoint);
  const hostname = parsed.hostname;
  const port = Number(parsed.port || (parsed.protocol === "rediss:" ? "6380" : parsed.protocol === "redis:" ? "6379" : "4222"));
  const deadline = Date.now() + 60_000;
  let connected = false;
  while (Date.now() < deadline && !connected) {
    connected = await new Promise<boolean>((resolve) => {
      const socket = createConnection({ host: hostname, port });
      socket.setTimeout(500);
      socket.once("connect", () => { socket.destroy(); resolve(true); });
      socket.once("error", () => resolve(false));
      socket.once("timeout", () => { socket.destroy(); resolve(false); });
    });
    if (!connected) await new Promise((resolve) => setTimeout(resolve, 500));
  }
  expect(connected, `development dependency ${parsed.protocol}//${hostname}:${port} did not accept TCP`).toBe(true);
}

async function expectRealtimeEvent(page: import("@playwright/test").Page, count: number) {
  await expect.poll(
    () => page.evaluate(() => (window as Window & { payviaRealtimeEvents?: string[] }).payviaRealtimeEvents?.filter((eventType) => eventType === "tenant.status.changed").length ?? 0),
    { timeout: 45_000 },
  ).toBe(count);
}

async function expectRealtimeEventOn(page: import("@playwright/test").Page, count: number) {
  await expect.poll(
    () => page.evaluate(() => (window as Window & { payviaRealtimeEvents?: string[] }).payviaRealtimeEvents?.filter((eventType) => eventType === "tenant.status.changed").length ?? 0),
    { timeout: 45_000 },
  ).toBe(count);
}

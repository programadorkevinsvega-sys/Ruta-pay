export type TenantSession = { user_id: string };
export type Tenant = { tenant_id: string; display_name: string; slug: string; status: string; status_version: number; created_at: string; updated_at: string };
export type TenantMembership = { tenant_id: string; role: string; status: string };
type ApiProblem = { code: string; message: string; correlation_id: string };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    readonly correlationId: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export function correlationId(): string {
  return crypto.randomUUID();
}

function csrfToken(): string | null {
  const cookie = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith("payvia_tenant_csrf="));
  return cookie?.slice("payvia_tenant_csrf=".length) ?? null;
}

export async function tenantSession(): Promise<TenantSession | null> {
  const response = await fetch("/v1/identity/session", {
    credentials: "include",
    headers: { "X-Correlation-ID": correlationId() },
  });
  if (response.status === 401) return null;
  if (!response.ok) throw await apiError(response);
  return (await response.json()) as TenantSession;
}

export async function login(email: string, password: string): Promise<void> {
  const response = await fetch("/v1/identity/session", {
    method: "POST",
    credentials: "include",
    headers: {
      "Content-Type": "application/json",
      "X-Correlation-ID": correlationId(),
    },
    body: JSON.stringify({ email, password }),
  });
  if (!response.ok) throw await apiError(response);
}

export async function register(email: string, password: string): Promise<void> {
  const response = await fetch("/v1/identity/register", { method: "POST", credentials: "include", headers: { "Content-Type": "application/json", "X-Correlation-ID": correlationId() }, body: JSON.stringify({ email, password }) });
  if (!response.ok) throw await apiError(response);
}

export async function createTenant(displayName: string, slug: string): Promise<Tenant> {
  const csrf = csrfToken();
  if (csrf === null) throw new ApiError(403, "csrf_validation_failed", correlationId(), "No se pudo validar la sesión.");
  const response = await fetch("/v1/tenants", { method: "POST", credentials: "include", headers: { "Content-Type": "application/json", "X-Correlation-ID": correlationId(), "X-CSRF-Token": csrf }, body: JSON.stringify({ display_name: displayName, slug }) });
  if (!response.ok) throw await apiError(response);
  return await response.json() as Tenant;
}

export async function tenantMemberships(): Promise<TenantMembership[]> {
  const response = await fetch("/v1/identity/memberships", { credentials: "include", headers: { "X-Correlation-ID": correlationId() } });
  if (!response.ok) throw await apiError(response);
  return (await response.json() as { memberships: TenantMembership[] }).memberships;
}

export async function tenantDetails(tenantId: string): Promise<Tenant> {
  const response = await fetch(`/v1/tenants/${encodeURIComponent(tenantId)}`, { credentials: "include", headers: { "X-Correlation-ID": correlationId() } });
  if (!response.ok) throw await apiError(response);
  return await response.json() as Tenant;
}

export async function logout(): Promise<void> {
  const csrf = csrfToken();
  if (csrf === null) {
    throw new ApiError(403, "csrf_validation_failed", correlationId(), "No se pudo validar la sesión.");
  }
  const response = await fetch("/v1/identity/session", {
    method: "DELETE",
    credentials: "include",
    headers: {
      "X-Correlation-ID": correlationId(),
      "X-CSRF-Token": csrf,
    },
  });
  if (!response.ok) throw await apiError(response);
}

async function apiError(response: Response): Promise<ApiError> {
  const body = (await response.json().catch(() => null)) as { error?: ApiProblem } | null;
  const problem = body?.error;
  return new ApiError(
    response.status,
    problem?.code ?? "request_failed",
    problem?.correlation_id ?? "",
    problem?.message ?? "No se pudo completar la solicitud.",
  );
}

export function userMessage(error: unknown): { message: string; correlationId: string } {
  if (error instanceof ApiError) return { message: error.message, correlationId: error.correlationId };
  return { message: "No se pudo conectar con la plataforma.", correlationId: "" };
}

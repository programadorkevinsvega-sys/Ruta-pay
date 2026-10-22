import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { changeTenantStatus, listTenants, login, logout, operatorSession, userMessage, type Tenant } from "./api";
import { requestPlatformRealtimeSync, usePlatformRealtime } from "./realtime";

export function App() {
  const queryClient = useQueryClient();
  const session = useQuery({ queryKey: ["operator-session"], queryFn: operatorSession });
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [tenantSearch, setTenantSearch] = useState("");
  const [statusReason, setStatusReason] = useState("");
  const [error, setError] = useState<{ message: string; correlationId: string } | null>(null);
  const loginMutation = useMutation({
    mutationFn: () => login(email, password),
    onSuccess: async () => {
      setPassword("");
      setError(null);
      await queryClient.invalidateQueries({ queryKey: ["operator-session"] });
    },
    onError: (cause: unknown) => {
      setPassword("");
      setError(userMessage(cause));
    },
  });
  const tenants = useQuery({ queryKey: ["platform-tenants", tenantSearch], queryFn: () => listTenants(tenantSearch), enabled: Boolean(session.data), refetchInterval: false });
  const realtime = usePlatformRealtime(Boolean(session.data));
  const connectionLabel = { disconnected: "desconectado", connecting: "conectando", connected: "conectado", reconnecting: "reconectando" }[realtime.connection];
  const statusMutation = useMutation({ mutationFn: ({ tenant, status }: { tenant: Tenant; status: string }) => changeTenantStatus(tenant, status, statusReason.trim()), onSuccess: async () => queryClient.invalidateQueries({ queryKey: ["platform-tenants"] }), onError: (cause: unknown) => setError(userMessage(cause)) });
  const logoutMutation = useMutation({
    mutationFn: logout,
    onSuccess: async () => {
      setError(null);
      queryClient.setQueryData(["operator-session"], null);
      await queryClient.invalidateQueries({ queryKey: ["operator-session"] });
    },
    onError: (cause: unknown) => setError(userMessage(cause)),
  });

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    loginMutation.mutate();
  }

  if (session.isPending) {
    return <main className="page"><p role="status">Verificando sesión…</p></main>;
  }

  if (session.isError) {
    const problem = userMessage(session.error);
    return <main className="page"><ErrorPanel {...problem} /></main>;
  }

  if (session.data !== null) {
    return (
      <main className="page">
        <section className="panel" aria-labelledby="dashboard-title">
          <p className="eyebrow">PAYVIA · PLATAFORMA</p>
          <h1 id="dashboard-title">Sesión de operador activa</h1>
          <p className="identity">Operador: <code>{session.data.operator_id}</code></p>
          <section aria-labelledby="tenants-title" className="tenant-admin">
            <h2 id="tenants-title">Comercios</h2>
            <label htmlFor="tenant-search">Buscar por nombre o identificador</label>
            <input id="tenant-search" value={tenantSearch} onChange={(event) => setTenantSearch(event.currentTarget.value)} />
            <label htmlFor="tenant-status-reason">Motivo de la operación administrativa</label>
            <input id="tenant-status-reason" required minLength={3} maxLength={500} value={statusReason} onChange={(event) => setStatusReason(event.currentTarget.value)} />
            <button className="secondary" onClick={requestPlatformRealtimeSync}>Sincronizar ahora</button>
            <p role="status">Tiempo real: {connectionLabel}. Última sincronización: {realtime.lastSynchronizedAt === null ? "Aún no sincronizado" : new Intl.DateTimeFormat("es-CO", { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(realtime.lastSynchronizedAt)}.</p>
            {tenants.isPending && <p role="status">Cargando comercios…</p>}
            {tenants.isError && <ErrorPanel {...userMessage(tenants.error)} />}
            {tenants.data && <><p>{tenants.data.total} comercios encontrados</p><div className="tenant-list">{tenants.data.tenants.map((tenant) => <TenantRow key={tenant.tenant_id} tenant={tenant} busy={statusMutation.isPending || statusReason.trim().length < 3} onChange={(status) => statusMutation.mutate({ tenant, status })} />)}</div></>}
          </section>
          {error && <ErrorPanel {...error} />}
          <button className="secondary" onClick={() => logoutMutation.mutate()} disabled={logoutMutation.isPending}>
            {logoutMutation.isPending ? "Cerrando sesión…" : "Cerrar sesión"}
          </button>
        </section>
      </main>
    );
  }

  return (
    <main className="page">
      <section className="panel" aria-labelledby="login-title">
        <p className="eyebrow">PAYVIA · PLATAFORMA</p>
        <h1 id="login-title">Administración</h1>
        <p className="hint">Acceso exclusivo para operadores autorizados.</p>
        <form onSubmit={submit}>
          <label htmlFor="operator-email">Correo</label>
          <input
            id="operator-email"
            type="email"
            autoComplete="username"
            required
            value={email}
            onChange={(event) => setEmail(event.currentTarget.value)}
          />
          <label htmlFor="operator-password">Contraseña</label>
          <input
            id="operator-password"
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(event) => setPassword(event.currentTarget.value)}
          />
          {error && <ErrorPanel {...error} />}
          <button type="submit" disabled={loginMutation.isPending}>
            {loginMutation.isPending ? "Validando…" : "Iniciar sesión"}
          </button>
        </form>
      </section>
    </main>
  );
}

function TenantRow({ tenant, busy, onChange }: { tenant: Tenant; busy: boolean; onChange: (status: string) => void }) {
  const actions: Record<string, { label: string; status: string }[]> = {
    pending_verification: [{ label: "Verificar y activar", status: "active" }],
    active: [{ label: "Suspender", status: "suspended" }, { label: "Iniciar cierre", status: "closing" }],
    suspended: [{ label: "Reactivar", status: "active" }, { label: "Iniciar cierre", status: "closing" }],
    closing: [{ label: "Confirmar cierre", status: "closed" }],
  };
  return <article className="tenant-row"><div><strong>{tenant.display_name}</strong><p><code>{tenant.slug}</code> · {tenant.status} · v{tenant.status_version}</p></div><div className="tenant-actions">{(actions[tenant.status] ?? []).map((action) => <button key={action.status} className="secondary" disabled={busy} onClick={() => onChange(action.status)}>{action.label}</button>)}</div></article>;
}

function ErrorPanel({ message, correlationId }: { message: string; correlationId: string }) {
  return (
    <div className="error" role="alert">
      <p>{message}</p>
      {correlationId !== "" && <small>Referencia: {correlationId}</small>}
    </div>
  );
}

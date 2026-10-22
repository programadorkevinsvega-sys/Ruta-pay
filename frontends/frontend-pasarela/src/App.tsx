import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { createTenant, login, logout, register, tenantDetails, tenantMemberships, tenantSession, userMessage } from "./api";
import { requestTenantRealtimeSync, useTenantRealtime, type RealtimeStatus } from "./realtime";

export function App() {
  const queryClient = useQueryClient();
  const session = useQuery({ queryKey: ["tenant-session"], queryFn: tenantSession });
  const memberships = useQuery({ queryKey: ["tenant-memberships"], queryFn: async () => Promise.all((await tenantMemberships()).map(async (membership) => ({ membership, tenant: await tenantDetails(membership.tenant_id) }))), enabled: Boolean(session.data), refetchInterval: (query) => query.state.data?.length ? false : 1000 });
  const realtime = useTenantRealtime(memberships.data?.map(({ membership }) => membership) ?? [], Boolean(session.data));
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [slug, setSlug] = useState("");
  const [onboarding, setOnboarding] = useState(false);
  const [error, setError] = useState<{ message: string; correlationId: string } | null>(null);
  const loginMutation = useMutation({
    mutationFn: () => login(email, password),
    onSuccess: async () => {
      setPassword("");
      setError(null);
      await queryClient.invalidateQueries({ queryKey: ["tenant-session"] });
    },
    onError: (cause: unknown) => {
      setPassword("");
      setError(userMessage(cause));
    },
  });
  const onboardingMutation = useMutation({
    mutationFn: async () => { await register(email, password); await login(email, password); return createTenant(displayName, slug); },
    onSuccess: async () => { setPassword(""); setError(null); await queryClient.invalidateQueries({ queryKey: ["tenant-session"] }); },
    onError: (cause: unknown) => { setPassword(""); setError(userMessage(cause)); },
  });
  const logoutMutation = useMutation({
    mutationFn: logout,
    onSuccess: async () => {
      queryClient.setQueryData(["tenant-session"], null);
      await queryClient.invalidateQueries({ queryKey: ["tenant-session"] });
    },
    onError: (cause: unknown) => setError(userMessage(cause)),
  });

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    if (onboarding) onboardingMutation.mutate(); else loginMutation.mutate();
  }

  if (session.isPending) return <main className="page"><p role="status">Verificando sesión…</p></main>;
  if (session.isError) return <main className="page"><ErrorPanel {...userMessage(session.error)} /></main>;

  if (session.data !== null) {
    return (
      <main className="page">
        <section className="panel" aria-labelledby="dashboard-title">
          <p className="eyebrow">PAYVIA · COMERCIOS</p>
          <h1 id="dashboard-title">Sesión autenticada</h1>
          <p className="identity">Usuario: <code>{session.data.user_id}</code></p>
          <TenantWorkspace memberships={memberships.data ?? []} loading={memberships.isPending} realtime={realtime} onSync={requestTenantRealtimeSync} />
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
        <p className="eyebrow">PAYVIA · COMERCIOS</p>
        <h1 id="login-title">Iniciar sesión</h1>
        <p className="hint">Accede a tu comercio o crea una cuenta y registra tu organización.</p>
        <form onSubmit={submit}>
          {onboarding && <><label htmlFor="tenant-name">Nombre del comercio</label><input id="tenant-name" required value={displayName} onChange={(event) => setDisplayName(event.currentTarget.value)} /><label htmlFor="tenant-slug">Identificador</label><input id="tenant-slug" required pattern="[a-z0-9-]+" value={slug} onChange={(event) => setSlug(event.currentTarget.value)} /></>}
          <label htmlFor="tenant-email">Correo</label>
          <input id="tenant-email" type="email" autoComplete="username" required value={email} onChange={(event) => setEmail(event.currentTarget.value)} />
          <label htmlFor="tenant-password">Contraseña</label>
          <input id="tenant-password" type="password" autoComplete="current-password" required value={password} onChange={(event) => setPassword(event.currentTarget.value)} />
          {error && <ErrorPanel {...error} />}
          <button type="submit" disabled={loginMutation.isPending || onboardingMutation.isPending}>{onboarding ? (onboardingMutation.isPending ? "Creando cuenta…" : "Crear cuenta y comercio") : (loginMutation.isPending ? "Validando…" : "Iniciar sesión")}</button>
          <button className="secondary" type="button" onClick={() => { setOnboarding(!onboarding); setError(null); }}>{onboarding ? "Ya tengo una cuenta" : "Crear cuenta de comercio"}</button>
        </form>
      </section>
    </main>
  );
}

function ErrorPanel({ message, correlationId }: { message: string; correlationId: string }) {
  return <div className="error" role="alert"><p>{message}</p>{correlationId !== "" && <small>Referencia: {correlationId}</small>}</div>;
}

function TenantWorkspace({ memberships, loading, realtime, onSync }: { memberships: { membership: { role: string; status: string }; tenant: { tenant_id: string; display_name: string; slug: string; status: string } }[]; loading: boolean; realtime: RealtimeStatus; onSync: () => void }) {
  const lastSync = realtime.lastSynchronizedAt === null ? "Aún no sincronizado" : new Intl.DateTimeFormat("es-CO", { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(realtime.lastSynchronizedAt);
  const connectionLabel = { disconnected: "desconectado", connecting: "conectando", connected: "conectado", reconnecting: "reconectando" }[realtime.connection];
  return <section aria-labelledby="membership-title"><h2 id="membership-title">Mis comercios</h2><p role="status">Tiempo real: {connectionLabel}. Última sincronización: {lastSync}.</p><button className="secondary" onClick={onSync}>Sincronizar ahora</button>{loading && <p role="status">Sincronizando membresías…</p>}{memberships.length === 0 && !loading && <p className="hint">Todavía no tienes comercios asociados. Crea uno para iniciar el proceso de verificación.</p>}<div className="tenant-list">{memberships.map(({ membership, tenant }) => <article className="tenant-row" key={tenant.tenant_id}><div><strong>{tenant.display_name}</strong><p><code>{tenant.slug}</code> · {tenant.status}</p></div><span>{membership.role} · {membership.status}</span></article>)}</div></section>;
}

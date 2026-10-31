import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { createTenant, login, logout, register, tenantDetails, tenantMemberships, tenantSession, userMessage } from "./api";
import { requestTenantRealtimeSync, useTenantRealtime, type RealtimeStatus } from "./realtime";
import { isLegalIdentificationNumberValid, isPasswordValid, merchantSlugFromName, passwordRequirements } from "./registration";

export function App() {
  const queryClient = useQueryClient();
  const [pathname, setPathname] = useState(() => {
    if (window.location.pathname === "/") {
      window.history.replaceState(null, "", "/login");
      return "/login";
    }
    if (window.location.pathname !== "/login" && window.location.pathname !== "/register") {
      window.history.replaceState(null, "", "/login");
      return "/login";
    }
    return window.location.pathname;
  });
  useEffect(() => {
    const updatePathname = () => setPathname(window.location.pathname);
    window.addEventListener("popstate", updatePathname);
    return () => window.removeEventListener("popstate", updatePathname);
  }, []);
  const session = useQuery({ queryKey: ["tenant-session"], queryFn: tenantSession });
  const memberships = useQuery({ queryKey: ["tenant-memberships"], queryFn: async () => Promise.all((await tenantMemberships()).map(async (membership) => ({ membership, tenant: await tenantDetails(membership.tenant_id) }))), enabled: Boolean(session.data), refetchInterval: (query) => query.state.data?.length ? false : 1000 });
  const realtime = useTenantRealtime(memberships.data?.map(({ membership }) => membership) ?? [], Boolean(session.data));
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [passwordConfirmation, setPasswordConfirmation] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [identificationType, setIdentificationType] = useState<"nit" | "cc">("nit");
  const [identificationNumber, setIdentificationNumber] = useState("");
  const registration = pathname === "/register";
  const slug = merchantSlugFromName(displayName);
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
    mutationFn: async () => {
      await register(email, password);
      await login(email, password);
      return createTenant(displayName.trim(), slug, identificationType, identificationNumber);
    },
    onSuccess: async () => { setPassword(""); setPasswordConfirmation(""); setError(null); await queryClient.invalidateQueries({ queryKey: ["tenant-session"] }); },
    onError: (cause: unknown) => { setPassword(""); setPasswordConfirmation(""); setError(userMessage(cause)); },
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
    if (!registration) {
      loginMutation.mutate();
      return;
    }
    if (slug.length < 3) {
      setError({ message: "Escribe un nombre de comercio con letras o números para generar su identificador.", correlationId: "" });
      return;
    }
    if (!isLegalIdentificationNumberValid(identificationNumber)) {
      setError({ message: "Escribe un NIT o una cédula con entre 5 y 20 dígitos.", correlationId: "" });
      return;
    }
    if (!isPasswordValid(password)) {
      setError({ message: "La contraseña debe cumplir todos los requisitos indicados.", correlationId: "" });
      return;
    }
    if (password !== passwordConfirmation) {
      setError({ message: "Las contraseñas no coinciden.", correlationId: "" });
      return;
    }
    onboardingMutation.mutate();
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
      <section className="panel" aria-labelledby="identity-title">
        <p className="eyebrow">PAYVIA · COMERCIOS</p>
        <h1 id="identity-title">{registration ? "Crear cuenta de comercio" : "Iniciar sesión"}</h1>
        <p className="hint">{registration ? "Registra tu cuenta y los datos de tu comercio." : "Accede a tu comercio."}</p>
        <form onSubmit={submit}>
          {registration && <>
            <label htmlFor="tenant-name">Nombre del comercio</label>
            <input id="tenant-name" autoComplete="organization" maxLength={120} required value={displayName} onChange={(event) => setDisplayName(event.currentTarget.value)} />
            <label htmlFor="tenant-slug">Identificador generado</label>
            <input id="tenant-slug" value={slug} readOnly aria-describedby="tenant-slug-hint" />
            <p className="hint" id="tenant-slug-hint">Se genera automáticamente a partir del nombre del comercio.</p>
            <label htmlFor="legal-identification-type">Tipo de documento</label>
            <select id="legal-identification-type" required value={identificationType} onChange={(event) => setIdentificationType(event.currentTarget.value as "nit" | "cc")}>
              <option value="nit">NIT</option>
              <option value="cc">Cédula de ciudadanía (CC)</option>
            </select>
            <label htmlFor="legal-identification-number">Número de NIT o CC</label>
            <input id="legal-identification-number" inputMode="numeric" autoComplete="off" maxLength={20} pattern="[0-9 .-]{5,24}" required value={identificationNumber} onChange={(event) => setIdentificationNumber(event.currentTarget.value.replace(/[^0-9 .-]/g, ""))} />
          </>}
          <label htmlFor="tenant-email">Correo</label>
          <input id="tenant-email" type="email" autoComplete="username" required value={email} onChange={(event) => setEmail(event.currentTarget.value)} />
          <label htmlFor="tenant-password">Contraseña</label>
          <input id="tenant-password" type="password" autoComplete={registration ? "new-password" : "current-password"} minLength={registration ? 12 : undefined} maxLength={registration ? 128 : undefined} required value={password} onChange={(event) => setPassword(event.currentTarget.value)} />
          {registration && <>
            <ul className="hint password-requirements" aria-label="Requisitos de contraseña">
              <PasswordRequirement valid={passwordRequirements(password).length}>Entre 12 y 128 caracteres</PasswordRequirement>
              <PasswordRequirement valid={passwordRequirements(password).uppercase}>Al menos una letra mayúscula</PasswordRequirement>
              <PasswordRequirement valid={passwordRequirements(password).lowercase}>Al menos una letra minúscula</PasswordRequirement>
              <PasswordRequirement valid={passwordRequirements(password).number}>Al menos un número</PasswordRequirement>
              <PasswordRequirement valid={passwordRequirements(password).symbol}>Al menos un símbolo, por ejemplo # o !</PasswordRequirement>
            </ul>
            <label htmlFor="tenant-password-confirmation">Confirmar contraseña</label>
            <input id="tenant-password-confirmation" type="password" autoComplete="new-password" minLength={12} maxLength={128} required value={passwordConfirmation} onChange={(event) => setPasswordConfirmation(event.currentTarget.value)} />
          </>}
          {error && <ErrorPanel {...error} />}
          <button type="submit" disabled={loginMutation.isPending || onboardingMutation.isPending}>{registration ? (onboardingMutation.isPending ? "Creando cuenta…" : "Crear cuenta y comercio") : (loginMutation.isPending ? "Validando…" : "Iniciar sesión")}</button>
        </form>
        <p className="hint">{registration ? <>¿Ya tienes cuenta? <a href="/login">Inicia sesión</a></> : <>¿Aún no tienes cuenta? <a href="/register">Registra tu comercio</a></>}</p>
      </section>
    </main>
  );
}

function PasswordRequirement({ children, valid }: { children: string; valid: boolean }) {
  return <li aria-live="polite">{valid ? "✓" : "○"} {children}</li>;
}

function ErrorPanel({ message, correlationId }: { message: string; correlationId: string }) {
  return <div className="error" role="alert"><p>{message}</p>{correlationId !== "" && <small>Referencia: {correlationId}</small>}</div>;
}

function TenantWorkspace({ memberships, loading, realtime, onSync }: { memberships: { membership: { role: string; status: string }; tenant: { tenant_id: string; display_name: string; slug: string; status: string } }[]; loading: boolean; realtime: RealtimeStatus; onSync: () => void }) {
  const lastSync = realtime.lastSynchronizedAt === null ? "Aún no sincronizado" : new Intl.DateTimeFormat("es-CO", { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(realtime.lastSynchronizedAt);
  const connectionLabel = { disconnected: "desconectado", connecting: "conectando", connected: "conectado", reconnecting: "reconectando" }[realtime.connection];
  return <section aria-labelledby="membership-title"><h2 id="membership-title">Mis comercios</h2><p role="status">Tiempo real: {connectionLabel}. Última sincronización: {lastSync}.</p><button className="secondary" onClick={onSync}>Sincronizar ahora</button>{loading && <p role="status">Sincronizando membresías…</p>}{memberships.length === 0 && !loading && <p className="hint">Todavía no tienes comercios asociados. Crea uno para iniciar el proceso de verificación.</p>}<div className="tenant-list">{memberships.map(({ membership, tenant }) => <article className="tenant-row" key={tenant.tenant_id}><div><strong>{tenant.display_name}</strong><p><code>{tenant.slug}</code> · {tenant.status}</p></div><span>{membership.role} · {membership.status}</span></article>)}</div></section>;
}

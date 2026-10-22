import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { login, logout, tenantSession, userMessage } from "./api";

export function App() {
  const queryClient = useQueryClient();
  const session = useQuery({ queryKey: ["tenant-session"], queryFn: tenantSession });
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
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
    loginMutation.mutate();
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
          <p className="hint">El tenant y sus permisos aparecerán cuando el flujo de registro y membresías esté disponible.</p>
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
        <p className="hint">Acceso para usuarios de comercios registrados.</p>
        <form onSubmit={submit}>
          <label htmlFor="tenant-email">Correo</label>
          <input id="tenant-email" type="email" autoComplete="username" required value={email} onChange={(event) => setEmail(event.currentTarget.value)} />
          <label htmlFor="tenant-password">Contraseña</label>
          <input id="tenant-password" type="password" autoComplete="current-password" required value={password} onChange={(event) => setPassword(event.currentTarget.value)} />
          {error && <ErrorPanel {...error} />}
          <button type="submit" disabled={loginMutation.isPending}>{loginMutation.isPending ? "Validando…" : "Iniciar sesión"}</button>
        </form>
      </section>
    </main>
  );
}

function ErrorPanel({ message, correlationId }: { message: string; correlationId: string }) {
  return <div className="error" role="alert"><p>{message}</p>{correlationId !== "" && <small>Referencia: {correlationId}</small>}</div>;
}

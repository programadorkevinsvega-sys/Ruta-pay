import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { login, logout, operatorSession, userMessage } from "./api";

export function App() {
  const queryClient = useQueryClient();
  const session = useQuery({ queryKey: ["operator-session"], queryFn: operatorSession });
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
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
          <p className="hint">La administración de tenants estará disponible cuando su API esté habilitada.</p>
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

function ErrorPanel({ message, correlationId }: { message: string; correlationId: string }) {
  return (
    <div className="error" role="alert">
      <p>{message}</p>
      {correlationId !== "" && <small>Referencia: {correlationId}</small>}
    </div>
  );
}

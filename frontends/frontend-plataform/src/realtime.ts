import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

type RealtimeEvent = { cursor: number; event_id: string; event_type: string; aggregate_id: string; aggregate_version: number };
type Replay = { events: RealtimeEvent[]; next_cursor: number; has_more: boolean };
export type RealtimeConnectionState = "disconnected" | "connecting" | "connected" | "reconnecting";
export type RealtimeStatus = { connection: RealtimeConnectionState; lastSynchronizedAt: number | null };

export function requestPlatformRealtimeSync() {
  window.dispatchEvent(new Event("payvia:realtime-sync"));
}

export function usePlatformRealtime(enabled: boolean): RealtimeStatus {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<RealtimeStatus>({ connection: "disconnected", lastSynchronizedAt: null });
  useEffect(() => {
    if (!enabled) {
      localStorage.removeItem("payvia:realtime:platform:cursor");
      setStatus({ connection: "disconnected", lastSynchronizedAt: null });
      return;
    }
    let stopped = false;
    let reconnectTimer: number | undefined;
    let socket: WebSocket | undefined;
    let attempt = 0;
    let cursor = Number(localStorage.getItem("payvia:realtime:platform:cursor") ?? "0");
    if (!Number.isSafeInteger(cursor) || cursor < 0) cursor = 0;
    const handled = new Set<string>();
    const versions = new Map<string, number>();

    async function recover() {
      let more = true;
      while (more && !stopped) {
        const response = await fetch(`/v1/realtime/events?audience=platform&cursor=${cursor}`, { credentials: "include" });
        if (response.status === 401 || response.status === 403) throw new Error("realtime_authorization_failed");
        if (!response.ok) throw new Error(`realtime_replay_${response.status}`);
        const replay = await response.json() as Replay;
        for (const event of replay.events) await consume(event);
        cursor = Math.max(cursor, replay.next_cursor);
        more = replay.has_more;
      }
      localStorage.setItem("payvia:realtime:platform:cursor", String(cursor));
      setStatus((current) => ({ ...current, lastSynchronizedAt: Date.now() }));
    }

    async function consume(event: RealtimeEvent) {
      if (handled.has(event.event_id)) return;
      handled.add(event.event_id);
      versions.set(event.aggregate_id, Math.max(versions.get(event.aggregate_id) ?? 0, event.aggregate_version));
      cursor = Math.max(cursor, event.cursor);
      setStatus((current) => ({ ...current, lastSynchronizedAt: Date.now() }));
      if (event.event_type.startsWith("tenant.")) await queryClient.invalidateQueries({ queryKey: ["platform-tenants"] });
      window.dispatchEvent(new CustomEvent("payvia:realtime-event", { detail: { event_type: event.event_type, event_id: event.event_id } }));
    }

    function connect() {
      if (stopped) return;
      setStatus((current) => ({ ...current, connection: "connecting" }));
      const url = new URL("/v1/realtime/stream", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      url.searchParams.set("audience", "platform");
      url.searchParams.set("cursor", String(cursor));
      socket = new WebSocket(url);
      socket.onopen = () => {
        attempt = 0;
        setStatus((current) => ({ ...current, connection: "connected" }));
      };
      socket.onmessage = async (message) => {
        try {
          const event = JSON.parse(String(message.data)) as RealtimeEvent;
          if (event.cursor > cursor + 1) await recover();
          if (event.aggregate_version <= (versions.get(event.aggregate_id) ?? 0)) return;
          await consume(event);
          localStorage.setItem("payvia:realtime:platform:cursor", String(cursor));
        } catch {
          try { await recover(); } catch { socket?.close(); }
        }
      };
      socket.onclose = (event) => {
        if (event.code === 1008) {
          localStorage.removeItem("payvia:realtime:platform:cursor");
          queryClient.setQueryData(["operator-session"], null);
          setStatus({ connection: "disconnected", lastSynchronizedAt: null });
          return;
        }
        if (stopped) return;
        setStatus((current) => ({ ...current, connection: "reconnecting" }));
        const delay = Math.min(30_000, 500 * 2 ** Math.min(attempt++, 6)) + Math.floor(Math.random() * 300);
        reconnectTimer = window.setTimeout(connect, delay);
      };
      socket.onerror = () => socket?.close();
    }

    const sync = () => { void recover().catch(() => undefined); };
    window.addEventListener("payvia:realtime-sync", sync);
    void recover().then(connect).catch(() => { if (!stopped) connect(); });
    return () => {
      stopped = true;
      window.removeEventListener("payvia:realtime-sync", sync);
      if (reconnectTimer !== undefined) window.clearTimeout(reconnectTimer);
      localStorage.removeItem("payvia:realtime:platform:cursor");
      socket?.close(1000, "session ended");
    };
  }, [enabled, queryClient]);
  return status;
}

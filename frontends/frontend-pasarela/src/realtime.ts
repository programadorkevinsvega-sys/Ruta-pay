import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { TenantMembership } from "./api";

type RealtimeEvent = {
  cursor: number;
  event_id: string;
  event_type: string;
  aggregate_id: string;
  aggregate_version: number;
  tenant_id: string;
};
type Replay = { events: RealtimeEvent[]; next_cursor: number; has_more: boolean };
export type RealtimeConnectionState = "disconnected" | "connecting" | "connected" | "reconnecting";
export type RealtimeStatus = { connection: RealtimeConnectionState; lastSynchronizedAt: number | null };

export function requestTenantRealtimeSync() {
  window.dispatchEvent(new Event("payvia:realtime-sync"));
}

export function useTenantRealtime(memberships: TenantMembership[], enabled: boolean): RealtimeStatus {
  const queryClient = useQueryClient();
  const scopes = memberships.map((membership) => membership.tenant_id).sort().join(",");
  const [status, setStatus] = useState<RealtimeStatus>({ connection: "disconnected", lastSynchronizedAt: null });
  useEffect(() => {
    const tenantIds = scopes ? scopes.split(",") : [];
    if (!enabled || tenantIds.length === 0) {
      const cursorKeys = Array.from({ length: localStorage.length }, (_, index) => localStorage.key(index))
        .filter((key): key is string => key !== null && key.startsWith("payvia:realtime:tenant:") && key.endsWith(":cursor"));
      for (const key of cursorKeys) localStorage.removeItem(key);
      setStatus({ connection: "disconnected", lastSynchronizedAt: null });
      return;
    }
    let stopped = false;
    const sockets = new Map<string, WebSocket>();
    const reconnectTimers = new Map<string, number>();
    const reconnectAttempts = new Map<string, number>();
    const statusByTenant = new Map(tenantIds.map((tenantId) => [tenantId, "connecting" as RealtimeConnectionState]));
    const handled = new Set<string>();
    const versions = new Map<string, number>();
    const cursors = new Map<string, number>();
    const membershipsByTenant = new Set(tenantIds);

    function publishConnectionState() {
      const states = [...statusByTenant.values()];
      if (states.every((state) => state === "connected")) setStatus((current) => ({ ...current, connection: "connected" }));
      else if (states.some((state) => state === "reconnecting" || state === "disconnected")) setStatus((current) => ({ ...current, connection: "reconnecting" }));
      else setStatus((current) => ({ ...current, connection: "connecting" }));
    }

    function cursorStorageKey(tenantId: string) {
      return `payvia:realtime:tenant:${tenantId}:cursor`;
    }

    async function consumeReplay(tenantId: string): Promise<number> {
      let cursor = cursors.get(tenantId) ?? 0;
      let more = true;
      while (more && !stopped) {
        const response = await fetch(`/v1/realtime/events?audience=tenant&tenant_id=${encodeURIComponent(tenantId)}&cursor=${cursor}`, { credentials: "include" });
        if (response.status === 401 || response.status === 403) throw new Error("realtime_authorization_failed");
        if (!response.ok) throw new Error(`realtime_replay_${response.status}`);
        const replay = await response.json() as Replay;
        for (const event of replay.events) {
          if (!membershipsByTenant.has(event.tenant_id) || handled.has(event.event_id)) continue;
          handled.add(event.event_id);
          versions.set(event.aggregate_id, Math.max(versions.get(event.aggregate_id) ?? 0, event.aggregate_version));
          await invalidateFor(event);
          cursor = Math.max(cursor, event.cursor);
        }
        cursor = Math.max(cursor, replay.next_cursor);
        more = replay.has_more;
      }
      cursors.set(tenantId, cursor);
      localStorage.setItem(cursorStorageKey(tenantId), String(cursor));
      setStatus((current) => ({ ...current, lastSynchronizedAt: Date.now() }));
      return cursor;
    }

    async function invalidateFor(event: RealtimeEvent) {
      if (event.event_type.startsWith("tenant.")) await queryClient.invalidateQueries({ queryKey: ["tenant-details", event.aggregate_id] });
      if (event.event_type.startsWith("membership.")) await queryClient.invalidateQueries({ queryKey: ["tenant-memberships"] });
      await queryClient.invalidateQueries({ queryKey: ["tenant-session"] });
      window.dispatchEvent(new CustomEvent("payvia:realtime-event", { detail: { event_type: event.event_type, event_id: event.event_id } }));
    }

    async function consumeMessage(tenantId: string, message: MessageEvent<string>) {
      try {
        const event = JSON.parse(message.data) as RealtimeEvent;
        if (!membershipsByTenant.has(event.tenant_id)) return;
        let cursor = cursors.get(tenantId) ?? 0;
        if (handled.has(event.event_id)) {
          cursor = Math.max(cursor, event.cursor);
          cursors.set(tenantId, cursor);
          localStorage.setItem(cursorStorageKey(tenantId), String(cursor));
          return;
        }
        if (event.cursor > cursor + 1) {
          await consumeReplay(tenantId);
          return;
        }
        const previousVersion = versions.get(event.aggregate_id) ?? 0;
        cursor = Math.max(cursor, event.cursor);
        cursors.set(tenantId, cursor);
        localStorage.setItem(cursorStorageKey(tenantId), String(cursor));
        if (event.aggregate_version <= previousVersion) return;
        handled.add(event.event_id);
        versions.set(event.aggregate_id, event.aggregate_version);
        await invalidateFor(event);
      } catch {
        try {
          await consumeReplay(tenantId);
        } catch {
          sockets.get(tenantId)?.close();
        }
      }
    }

    function connect(tenantId: string) {
      if (stopped) return;
      const storageKey = cursorStorageKey(tenantId);
      const storedCursor = Number(localStorage.getItem(storageKey) ?? "0");
      const cursor = Number.isSafeInteger(storedCursor) && storedCursor >= 0 ? storedCursor : 0;
      cursors.set(tenantId, cursor);
      statusByTenant.set(tenantId, "connecting");
      publishConnectionState();
      const url = new URL("/v1/realtime/stream", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      url.searchParams.set("audience", "tenant");
      url.searchParams.set("tenant_id", tenantId);
      url.searchParams.set("cursor", String(cursor));
      const socket = new WebSocket(url);
      sockets.set(tenantId, socket);
      let messageQueue = Promise.resolve();
      socket.onopen = () => {
        if (sockets.get(tenantId) !== socket) return;
        reconnectAttempts.set(tenantId, 0);
        statusByTenant.set(tenantId, "connected");
        publishConnectionState();
      };
      socket.onmessage = (message) => {
        messageQueue = messageQueue.then(() => consumeMessage(tenantId, message as MessageEvent<string>));
      };
      socket.onclose = (event) => {
        if (sockets.get(tenantId) !== socket) return;
        if (event.code === 1008) {
          statusByTenant.set(tenantId, "disconnected");
          localStorage.removeItem(storageKey);
          queryClient.setQueryData(["tenant-session"], null);
          setStatus({ connection: "disconnected", lastSynchronizedAt: null });
          publishConnectionState();
          return;
        }
        if (stopped) return;
        statusByTenant.set(tenantId, "reconnecting");
        publishConnectionState();
        const attempt = reconnectAttempts.get(tenantId) ?? 0;
        const delay = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6)) + Math.floor(Math.random() * 300);
        reconnectAttempts.set(tenantId, attempt + 1);
        const timer = window.setTimeout(() => connect(tenantId), delay);
        reconnectTimers.set(tenantId, timer);
      };
      socket.onerror = () => {
        if (sockets.get(tenantId) === socket) socket.close();
      };
    }

    const sync = () => {
      for (const tenantId of tenantIds) void consumeReplay(tenantId).catch(() => {
        statusByTenant.set(tenantId, "reconnecting");
        publishConnectionState();
      });
    };
    window.addEventListener("payvia:realtime-sync", sync);
    for (const tenantId of tenantIds) connect(tenantId);
    return () => {
      stopped = true;
      window.removeEventListener("payvia:realtime-sync", sync);
      for (const timer of reconnectTimers.values()) window.clearTimeout(timer);
      for (const tenantId of tenantIds) localStorage.removeItem(cursorStorageKey(tenantId));
      for (const socket of sockets.values()) socket.close(1000, "scope changed or session ended");
    };
  }, [enabled, scopes, queryClient]);
  return status;
}

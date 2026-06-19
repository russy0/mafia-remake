import type { ActionRequest, GameState } from "./types";

let sessionToken = "";
let guildId = "";

export function activityUrl(path: string): string {
  const moduleUrl = new URL(import.meta.url);
  const baseUrl = moduleUrl.pathname.includes("/assets/")
    ? new URL("../", moduleUrl)
    : new URL(import.meta.env.BASE_URL || "./", document.baseURI);
  return new URL(path.replace(/^\/+/, ""), baseUrl).toString();
}

export function setSession(token: string, guild: string) {
  sessionToken = token;
  guildId = guild;
}

function headers(): HeadersInit {
  return {
    "Content-Type": "application/json",
    "X-Session-Token": sessionToken,
  };
}

export async function fetchState(): Promise<GameState> {
  const res = await fetch(
    activityUrl(`activity/api/state?guild_id=${guildId}`),
    { headers: headers() }
  );
  if (!res.ok) throw new Error(`state fetch failed: ${res.status}`);
  return res.json();
}

export async function sendAction(req: Omit<ActionRequest, "guild_id">): Promise<{ ok: boolean; message?: string }> {
  const res = await fetch(activityUrl("activity/api/action"), {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({ ...req, guild_id: guildId }),
  });
  return res.json();
}

export async function sendChat(content: string): Promise<{ ok: boolean; error?: string }> {
  const res = await fetch("/activity/api/chat", {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({ guild_id: guildId, content }),
  });
  return res.json();
}

export function createWebSocket(onMessage: (state: GameState) => void): WebSocket {
  const wsUrl = new URL(activityUrl("activity/api/ws"));
  wsUrl.protocol = location.protocol === "https:" ? "wss:" : "ws:";
  wsUrl.searchParams.set("guild_id", guildId);
  wsUrl.searchParams.set("token", sessionToken);
  // 브라우저는 WebSocket 업그레이드 시 커스텀 헤더를 보낼 수 없으므로 토큰을 쿼리 파라미터로 전달
  const ws = new WebSocket(wsUrl.toString());

  ws.addEventListener("message", (event) => {
    try {
      const state: GameState = JSON.parse(event.data);
      onMessage(state);
    } catch {}
  });

  return ws;
}

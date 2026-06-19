import { DiscordSDK, DiscordSDKMock } from "@discord/embedded-app-sdk";
import { activityUrl } from "./api";

const isEmbedded = new URLSearchParams(window.location.search).has("frame_id");

let discordSdk: DiscordSDK | DiscordSDKMock | null = null;

export interface AuthResult {
  sessionToken: string;
  userId: string;
  username: string;
  guildId: string;
}

async function fetchClientId(): Promise<string> {
  const res = await fetch(activityUrl("activity/api/client-config"));
  if (!res.ok) throw new Error(`client config failed: ${res.status}`);
  const body = await res.json();
  return body.client_id ?? "";
}

async function getDiscordSdk(clientId: string): Promise<DiscordSDK | DiscordSDKMock> {
  if (discordSdk) return discordSdk;
  if (isEmbedded) {
    discordSdk = new DiscordSDK(clientId);
  } else {
    discordSdk = new DiscordSDKMock(
      clientId,
      import.meta.env.VITE_MOCK_GUILD_ID ?? null,
      null,
      null,
    );
  }
  return discordSdk;
}

export async function authenticateWithDiscord(): Promise<AuthResult> {
  const clientId = await fetchClientId();
  const sdk = await getDiscordSdk(clientId);
  await sdk.ready();

  const { code } = await sdk.commands.authorize({
    client_id: clientId,
    response_type: "code",
    state: "",
    prompt: "none",
    scope: ["identify"],
  });

  const guild = sdk.guildId ?? "";
  const res = await fetch(activityUrl(`activity/api/auth?code=${code}&guild_id=${guild}`));
  if (!res.ok) throw new Error("Authentication failed");

  const { session_token, user_id, username } = await res.json();

  return {
    sessionToken: session_token,
    userId: user_id,
    username,
    guildId: guild,
  };
}

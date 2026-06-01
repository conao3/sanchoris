import { env } from './env';

const COGNITO_DOMAIN = env.cognitoDomain;
const CLIENT_ID = env.cognitoClientId;

// Redirect URI is derived from the current origin (e.g. http://sanchoris.localhost:1355
// in dev). It is NOT an env var; it must exactly match the Cognito app client CallbackURL.
const REDIRECT_URI = `${window.location.origin}/auth/callback`;

const ID_TOKEN_KEY = 'sanchoris_id_token';
const ACCESS_TOKEN_KEY = 'sanchoris_access_token';
const REFRESH_TOKEN_KEY = 'sanchoris_refresh_token';

export type CognitoTokens = {
  id_token: string;
  access_token: string;
  refresh_token: string;
  expires_in: number;
  token_type: string;
};

export function getLoginUrl(): string {
  return `https://${COGNITO_DOMAIN}/login?response_type=code&client_id=${CLIENT_ID}&redirect_uri=${encodeURIComponent(
    REDIRECT_URI,
  )}&scope=email+openid+profile`;
}

export function getSignupUrl(): string {
  return `https://${COGNITO_DOMAIN}/signup?response_type=code&client_id=${CLIENT_ID}&redirect_uri=${encodeURIComponent(
    REDIRECT_URI,
  )}&scope=email+openid+profile`;
}

export function getLogoutUrl(): string {
  return `https://${COGNITO_DOMAIN}/logout?client_id=${CLIENT_ID}&logout_uri=${encodeURIComponent(
    `${window.location.origin}/login`,
  )}`;
}

export async function exchangeCodeForTokens(code: string): Promise<CognitoTokens> {
  const res = await fetch(`https://${COGNITO_DOMAIN}/oauth2/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'authorization_code',
      client_id: CLIENT_ID,
      redirect_uri: REDIRECT_URI,
      code,
    }),
  });
  if (!res.ok) {
    throw new Error(`token exchange failed: ${res.status}`);
  }
  return (await res.json()) as CognitoTokens;
}

export function saveTokens(tokens: CognitoTokens): void {
  localStorage.setItem(ID_TOKEN_KEY, tokens.id_token);
  localStorage.setItem(ACCESS_TOKEN_KEY, tokens.access_token);
  if (tokens.refresh_token) {
    localStorage.setItem(REFRESH_TOKEN_KEY, tokens.refresh_token);
  }
}

export function getIdToken(): string | null {
  return localStorage.getItem(ID_TOKEN_KEY);
}

export function clearTokens(): void {
  localStorage.removeItem(ID_TOKEN_KEY);
  localStorage.removeItem(ACCESS_TOKEN_KEY);
  localStorage.removeItem(REFRESH_TOKEN_KEY);
}

export function isLoggedIn(): boolean {
  return !!getIdToken();
}

// Dedupe concurrent refreshes so multiple in-flight 401s share one network call.
let refreshPromise: Promise<void> | null = null;

export async function refreshTokens(): Promise<void> {
  if (refreshPromise) {
    return refreshPromise;
  }

  const refreshToken = localStorage.getItem(REFRESH_TOKEN_KEY);
  if (!refreshToken) {
    throw new Error('no refresh token available');
  }

  refreshPromise = (async () => {
    try {
      const res = await fetch(`https://${COGNITO_DOMAIN}/oauth2/token`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({
          grant_type: 'refresh_token',
          client_id: CLIENT_ID,
          refresh_token: refreshToken,
        }),
      });
      if (!res.ok) {
        throw new Error(`token refresh failed: ${res.status}`);
      }
      const tokens = (await res.json()) as Omit<CognitoTokens, 'refresh_token'>;
      localStorage.setItem(ID_TOKEN_KEY, tokens.id_token);
      localStorage.setItem(ACCESS_TOKEN_KEY, tokens.access_token);
    } finally {
      refreshPromise = null;
    }
  })();

  return refreshPromise;
}

import { env } from './env';

const COGNITO_DOMAIN = env.cognitoDomain;
const CLIENT_ID = env.cognitoClientId;

// Redirect URI is derived from the current origin (e.g. https://sanchoris.localhost
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

export function samePath(value: unknown): string | undefined {
  if (typeof value !== 'string' || !value.startsWith('/') || value.startsWith('//') || value.startsWith('/\\')) {
    return undefined;
  }
  const url = new URL(value, window.location.origin);
  if (url.origin !== window.location.origin || url.pathname.startsWith('/login') || url.pathname.startsWith('/auth/')) {
    return undefined;
  }
  return `${url.pathname}${url.search}${url.hash}`;
}

export function loginPath(next: string | undefined): string {
  const path = samePath(next);
  return path ? `/login?next=${encodeURIComponent(path)}` : '/login';
}

export function getLoginUrl(next: string | undefined): string {
  const path = samePath(next);
  return `https://${COGNITO_DOMAIN}/login?response_type=code&client_id=${CLIENT_ID}&redirect_uri=${encodeURIComponent(
    REDIRECT_URI,
  )}&scope=email+openid+profile${path ? `&state=${encodeURIComponent(path)}` : ''}`;
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

function withIdToken(init: RequestInit | undefined): RequestInit {
  const headers = new Headers(init?.headers);
  const token = getIdToken();
  if (token) {
    headers.set('Authorization', `Bearer ${token}`);
  }
  return { ...init, headers };
}

export async function authorizedFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const response = await fetch(input, withIdToken(init));
  if (response.status !== 401) {
    return response;
  }
  const refreshed = await refreshTokens().then(
    () => fetch(input, withIdToken(init)),
    () => response,
  );
  if (refreshed.status === 401) {
    clearTokens();
    window.location.assign(loginPath(`${window.location.pathname}${window.location.search}${window.location.hash}`));
  }
  return refreshed;
}

export function signOut(): void {
  clearTokens();
  window.location.assign(getLogoutUrl());
}

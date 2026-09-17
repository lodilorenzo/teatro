import { apiRequest } from './api.js';
import { basicAuth } from './shared.js';

const AUTH_KEY = 'teatro.auth.v2';

export async function createSession(username, password, rememberMe = false) {
  const session = await apiRequest('/api/auth/session', basicAuth(username, password), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ remember_me: rememberMe }),
  });
  return {
    user: session.user,
    auth: {
      username: session.user.username,
      header: `Bearer ${session.token}`,
      expiresAt: session.expires_at * 1000,
    },
  };
}

export async function revokeSession(auth) {
  if (!auth?.header?.startsWith('Bearer teatro_session_')) return;
  try {
    await apiRequest('/api/auth/session', auth.header, { method: 'DELETE' });
  } catch (error) {
    if (error.status !== 401) throw error;
  }
}

export function loadSavedAuth() {
  for (const name of ['sessionStorage', 'localStorage']) {
    try {
      const storage = globalThis[name];
      storage?.removeItem('teatro.auth.v1'); // Discard legacy password-based sign-ins.
      const auth = JSON.parse(storage?.getItem(AUTH_KEY) || 'null');
      if (typeof auth?.username === 'string'
        && typeof auth?.header === 'string' && auth.header.startsWith('Bearer teatro_session_')
        && Number.isFinite(auth.expiresAt) && auth.expiresAt > Date.now()) return auth;
      storage?.removeItem(AUTH_KEY);
    } catch (_) {
      // Malformed or unavailable storage must not prevent signing in.
    }
  }
  return null;
}

export function saveAuth(auth, rememberMe = false) {
  clearSavedAuth();
  if (rememberMe) {
    try {
      localStorage.setItem(AUTH_KEY, JSON.stringify(auth));
      return;
    } catch (_) {
      // If persistence is blocked, fall back to this tab's session.
    }
  }
  try {
    sessionStorage.setItem(AUTH_KEY, JSON.stringify(auth));
  } catch (_) {
    // Private browser modes may reject storage; in-memory sign-in still works.
  }
}

export function clearSavedAuth() {
  for (const name of ['sessionStorage', 'localStorage']) {
    try {
      globalThis[name]?.removeItem(AUTH_KEY);
      globalThis[name]?.removeItem('teatro.auth.v1');
    } catch (_) {
      // Attempt each store independently when one is unavailable.
    }
  }
}

import { apiRequest } from '../public/api.js';
import { clearAuth } from './auth.js';
import { state } from './state.js';

export async function api(path, options = {}) {
  if (!state.auth?.header) throw new Error('Not signed in');

  const headers = new Headers(options.headers || {});
  const hasBody = options.body !== undefined && options.body !== null;
  if (hasBody && !(options.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }

  try {
    return await apiRequest(path, state.auth.header, { ...options, headers });
  } catch (error) {
    if (error?.status === 401) clearAuth();
    throw error;
  }
}

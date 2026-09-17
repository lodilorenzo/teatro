export async function apiRequest(path, authorization, options = {}) {
  if (!authorization) throw new ApiRequestError('Sign in to continue.', 401, 'unauthorized');

  const headers = new Headers(options.headers || {});
  headers.set('Authorization', authorization);
  if (!headers.has('Accept')) headers.set('Accept', options.raw ? '*/*' : 'application/json');

  const response = await fetch(path, {
    method: options.method || 'GET',
    body: options.body,
    headers,
  });

  if (!response.ok) throw await responseError(response);
  if (options.raw) return response;

  const contentType = response.headers.get('content-type') || '';
  return contentType.includes('application/json') ? response.json() : response;
}

async function responseError(response) {
  const contentType = response.headers.get('content-type') || '';
  let message = `${response.status} ${response.statusText}`.trim();
  let code = null;
  if (contentType.includes('application/json')) {
    const body = await response.json().catch(() => null);
    message = body?.error?.message || message;
    code = body?.error?.code || null;
  } else {
    message = await response.text().catch(() => '') || message;
  }
  return new ApiRequestError(message, response.status, code);
}

export class ApiRequestError extends Error {
  constructor(message, status = 0, code = null) {
    super(message);
    this.name = 'ApiRequestError';
    this.status = status;
    this.code = code;
  }
}

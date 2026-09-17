import { createSession, saveAuth } from '/public/auth.js';
import { VERSION_LABEL } from '/public/shared.js';

const form = document.getElementById('setup-form');
const errorBox = document.getElementById('setup-error');
const button = form.querySelector('button[type="submit"]');
document.getElementById('version-label').textContent = VERSION_LABEL;
document.getElementById('server-address').textContent = window.location.origin;

function showError(message) {
  errorBox.textContent = message;
  errorBox.hidden = false;
}

form.addEventListener('submit', async (event) => {
  event.preventDefault();
  errorBox.hidden = true;
  const fields = new FormData(form);
  const username = String(fields.get('username') || '').trim();
  const password = String(fields.get('password') || '');
  if (password !== String(fields.get('password_confirmation') || '')) {
    showError('Passwords do not match.');
    form.elements.password_confirmation.focus();
    return;
  }

  button.disabled = true;
  button.textContent = 'Creating administrator…';
  try {
    const response = await fetch('/api/setup', {
      method: 'POST',
      headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password }),
    });
    const body = await response.json().catch(() => null);
    if (!response.ok) throw new Error(body?.error?.message || `${response.status} ${response.statusText}`);

    try {
      const rememberMe = fields.has('remember_me');
      const { auth } = await createSession(username, password, rememberMe);
      saveAuth(auth, rememberMe);
    } catch (_) {
      // Setup has succeeded even if sign-in fails; the admin login can retry it.
    }
    window.location.replace('/admin');
  } catch (error) {
    showError(error?.message || 'Setup failed.');
    button.disabled = false;
    button.textContent = 'Create administrator';
  }
});

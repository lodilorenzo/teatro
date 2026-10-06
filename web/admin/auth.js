import { clearSavedAuth, saveAuth as saveSharedAuth } from '../public/auth.js';
import { clearCoverImages } from '../public/shared.js';
import { clearRommCovers } from './features/romm-covers.js';
import { invalidateRomSelection, setAuth, state } from './state.js';

export function saveAuth(auth, rememberMe = false) {
  setAuth(auth);
  saveSharedAuth(auth, rememberMe);
}

export function clearAuth() {
  invalidateRomSelection();
  setAuth(null);
  state.user = null;
  clearSavedAuth();
  clearCoverImages();
  clearRommCovers();
}

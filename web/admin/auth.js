import {
  clearSavedAuth, loadSavedAuth as loadSharedAuth, saveAuth as saveSharedAuth,
} from '../public/auth.js';
import { clearCoverImages } from '../public/shared.js';
import { clearRommCovers } from './features/romm-covers.js';
import { invalidateRomSelection, setAuth, setUser } from './state.js';

export function loadSavedAuth() {
  return loadSharedAuth();
}

export function saveAuth(auth, rememberMe = false) {
  setAuth(auth);
  saveSharedAuth(auth, rememberMe);
}

export function clearAuth() {
  invalidateRomSelection();
  setAuth(null);
  setUser(null);
  clearSavedAuth();
  clearCoverImages();
  clearRommCovers();
}

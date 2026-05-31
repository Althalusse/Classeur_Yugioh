"""
async_image_loader.py — Moteur de traitement d'images HORS THREAD UI.

Problème résolu
───────────────
Jusqu'ici, le redimensionnement LANCZOS + les filtres ImageEnhance (cartes)
et le decode+thumbnail+paste (couvertures d'accueil) s'exécutaient SUR LE
THREAD UI, au moment de construire chaque widget. Avec 9-18 cartes par spread
(reconstruites à chaque changement de page) et N couvertures à l'accueil,
l'interface se figeait visiblement pendant chaque rendu.

Solution
────────
Tout le travail PIL coûteux est déporté dans un pool de threads. Le widget
affiche IMMÉDIATEMENT un placeholder (sans le moindre calcul lourd), et l'image
finale lui est poussée via `owner.after(0, ...)` dès qu'elle est prête. L'UI
ne bloque jamais ; les images « tombent » au fil de l'eau.

Niveaux de cache
────────────────
  1. PIL brut          : module.gestion_img.cache_images.get_or_load_pil_image
                         (Image.open + convert, une fois par fichier).
  2. CTkImage finale   : ce module — clé (path, possessed, w, h, hover).
                         Évite de refaire resize + enhance à chaque rendu.

Thread-safety
─────────────
Un unique `_LOCK` protège les caches et la table des requêtes en cours
(`_PENDING`). La construction de `ctk.CTkImage` est faite dans le worker :
son `__init__` ne touche PAS à Tk (le `PhotoImage` sous-jacent est créé
paresseusement par CustomTkinter lors du rendu, donc sur le thread UI).
Seul `owner.after(0, ...)` traverse la frontière de thread — exactement le
même pattern que `_safe_after` utilisé partout ailleurs dans le projet.

Génération
──────────
`clear_image_cache()` incrémente `_GENERATION`. Les workers démarrés avant un
clear ne réinsèrent pas leur résultat (potentiellement périmé) et ne notifient
pas leurs callbacks. Indispensable car un clear accompagne un changement de
source d'images / une fin de téléchargement (le contenu disque a changé).
"""

# Yu-Gi-Oh! Collection Manager
# Copyright (C) 2026  Althalusse
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

import os
import threading
from concurrent.futures import ThreadPoolExecutor
from typing import Callable, Iterable, Optional, Tuple

import customtkinter as ctk
from PIL import Image, ImageEnhance

from module.theme import C
from module.gestion_img.cache_images import get_or_load_pil_image
from module.logger_app import log


# ── Configuration ────────────────────────────────────────────────────────────

# PIL libère partiellement le GIL pendant resize/enhance → quelques workers
# accélèrent réellement. On reste modeste pour ne pas saturer un petit CPU.
_MAX_WORKERS = max(2, min(4, (os.cpu_count() or 2)))
_MAX_CACHE_ITEMS = 400

_EXECUTOR = ThreadPoolExecutor(
    max_workers=_MAX_WORKERS, thread_name_prefix="img-loader"
)

# Couleur du placeholder : alignée sur le fond des cartes (C["bg3"]) pour que
# l'état « en cours de chargement » se fonde dans le slot sans flash blanc.
_PLACEHOLDER_COLOR = C.get("bg3", "#12141D")


# ── État partagé ─────────────────────────────────────────────────────────────

_LOCK = threading.Lock()
_CTK_CACHE: dict[tuple, ctk.CTkImage] = {}
_PENDING: dict[tuple, list[tuple]] = {}          # key -> [(owner, on_ready), ...]
_PLACEHOLDER_CACHE: dict[tuple[int, int], ctk.CTkImage] = {}
_GENERATION = 0


# ── Helpers internes ──────────────────────────────────────────────────────────

def _key(path: str, possessed: bool, w: int, h: int, hover: bool) -> tuple:
    return (path, bool(possessed), int(w), int(h), bool(hover))


def placeholder_ctk(w: int, h: int) -> ctk.CTkImage:
    """Placeholder uni, mis en cache par taille (création quasi gratuite)."""
    k = (int(w), int(h))
    p = _PLACEHOLDER_CACHE.get(k)
    if p is None:
        try:
            p = ctk.CTkImage(
                Image.new("RGB", k, _PLACEHOLDER_COLOR), size=k
            )
        except Exception:
            # Dernier recours : taille 1×1 pour ne jamais lever ici.
            p = ctk.CTkImage(Image.new("RGB", (1, 1), _PLACEHOLDER_COLOR), size=(1, 1))
        _PLACEHOLDER_CACHE[k] = p
    return p


def _process(path: str, possessed: bool, w: int, h: int, hover: bool) -> ctk.CTkImage:
    """Travail lourd (thread worker) : resize LANCZOS + filtres → CTkImage.

    Réplique exactement l'ancienne logique de _load_card_image, mais hors UI.
    """
    pil = get_or_load_pil_image(path)
    if pil is None:
        return placeholder_ctk(w, h)
    pil = pil.resize((int(w), int(h)), Image.LANCZOS)
    if not possessed:
        if hover:
            pil = ImageEnhance.Color(pil).enhance(0.70)
            pil = ImageEnhance.Brightness(pil).enhance(0.70)
        else:
            pil = ImageEnhance.Color(pil).enhance(0.20)
            pil = ImageEnhance.Brightness(pil).enhance(0.35)
    return ctk.CTkImage(pil, size=(int(w), int(h)))


def _dispatch(owner, cb: Optional[Callable], result) -> None:
    """Replanifie `cb(result)` sur la boucle d'événements du widget `owner`.

    Unique appel cross-thread (owner.after) — même contrat que _safe_after.
    L'existence du widget est revérifiée sur le thread UI dans `_safe_cb`.
    """
    if cb is None or owner is None:
        return
    try:
        owner.after(0, lambda: _safe_cb(owner, cb, result))
    except Exception:
        # owner détruit / pas de mainloop actif → on abandonne silencieusement.
        pass


def _safe_cb(owner, cb: Callable, result) -> None:
    """Exécuté sur le thread UI : vérifie la survie du widget puis appelle cb."""
    try:
        if not owner.winfo_exists():
            return
    except Exception:
        return
    try:
        cb(result)
    except Exception as e:
        log.warning(f"async_image_loader callback: {e}")


def _worker(key: tuple, gen: int, path: str, possessed: bool,
            w: int, h: int, hover: bool) -> None:
    """Tâche de pool : calcule l'image, met en cache, notifie les waiters."""
    try:
        img = _process(path, possessed, w, h, hover)
    except Exception as e:
        log.warning(f"async_image_loader worker: {e}")
        img = placeholder_ctk(w, h)

    with _LOCK:
        waiters = _PENDING.pop(key, [])
        if gen != _GENERATION:
            # Cache vidé entre-temps → résultat potentiellement périmé.
            waiters = []
        elif len(_CTK_CACHE) < _MAX_CACHE_ITEMS:
            _CTK_CACHE[key] = img

    for owner, cb in waiters:
        _dispatch(owner, cb, img)


# ── API publique ──────────────────────────────────────────────────────────────

def get_cached_ctk_image(path: str, possessed: bool, w: int, h: int,
                         hover: bool) -> Optional[ctk.CTkImage]:
    """Retourne la CTkImage déjà calculée, ou None. Aucun effet de bord."""
    with _LOCK:
        return _CTK_CACHE.get(_key(path, possessed, w, h, hover))


def request_ctk_image(path: str, possessed: bool, w: int, h: int, hover: bool,
                      owner, on_ready: Callable) -> tuple:
    """Demande une image carte. Retourne ``(image, ready)``.

    - Déjà en cache : ``(image_réelle, True)`` — rien à attendre, aucun callback.
    - Sinon : planifie le calcul en arrière-plan (dédupliqué par clé), et
      retourne ``(placeholder, False)``. ``on_ready(ctk_image)`` sera appelé
      sur le thread UI dès que l'image réelle est prête (si ``owner`` existe).
    """
    key = _key(path, possessed, w, h, hover)
    with _LOCK:
        cached = _CTK_CACHE.get(key)
        if cached is not None:
            return cached, True
        if key in _PENDING:
            _PENDING[key].append((owner, on_ready))
        else:
            _PENDING[key] = [(owner, on_ready)]
            _EXECUTOR.submit(_worker, key, _GENERATION, path,
                             possessed, w, h, hover)
    return placeholder_ctk(w, h), False


def prefetch_images(specs: Iterable[Tuple[str, bool, int, int, bool]]) -> None:
    """Pré-calcule des images en arrière-plan, sans callback ni placeholder.

    Sert à réchauffer le cache pour les pages voisines (navigation instantanée).
    `specs` : itérable de (path, possessed, w, h, hover).
    """
    for path, possessed, w, h, hover in specs:
        key = _key(path, possessed, w, h, hover)
        with _LOCK:
            if key in _CTK_CACHE or key in _PENDING:
                continue
            _PENDING[key] = []   # aucun waiter : on remplit juste le cache
            _EXECUTOR.submit(_worker, key, _GENERATION, path,
                             possessed, w, h, hover)


def run_async(work: Callable, owner, on_done: Callable) -> None:
    """Exécute `work()` (PIL/IO, renvoie une valeur quelconque ou None) hors UI,
    puis appelle `on_done(result)` sur la boucle d'événements de `owner`.

    Utilisé pour les couvertures d'accueil (thumbnail + paste hors thread UI).
    """
    def _job():
        try:
            res = work()
        except Exception as e:
            log.warning(f"async_image_loader run_async: {e}")
            res = None
        _dispatch(owner, on_done, res)

    _EXECUTOR.submit(_job)


def clear_image_cache() -> None:
    """Vide les CTkImage en cache et invalide les workers en vol.

    À appeler quand le contenu disque change (changement de source d'images,
    fin de téléchargement) ou quand la taille de carte change.
    """
    global _GENERATION
    with _LOCK:
        _GENERATION += 1
        _CTK_CACHE.clear()
        _PENDING.clear()
    # NB : les placeholders (uniquement fonction de la taille) ne sont pas
    # invalidés — leur contenu ne dépend pas du disque.

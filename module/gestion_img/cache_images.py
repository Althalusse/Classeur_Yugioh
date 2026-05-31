"""
cache_images.py — Cache PIL thread-safe pour accélérer l'affichage du classeur.

Problème résolu :
  PIL.Image.open() + .convert("RGB") étaient appelés à chaque rendu de widget
  carte (9-18 appels par changement de page). Avec 500 ms par page perçue,
  l'interface paraissait gelée.

Solution :
  Garder en mémoire les objets PIL.Image après leur premier chargement disque.
  Le resize et les filtres (ImageEnhance) restent appliqués à chaque rendu car
  ils dépendent de la taille et de l'état possédé/survolé de la carte.

Thread safety :
  Un lock protège toutes les mutations du dict _PIL_CACHE pour supporter les
  chargements concurrents (thread de téléchargement + thread UI).

Invalidation :
  - clear_cache() vidange manuellement (appel conseillé lors d'un changement
    de source d'image, car les chemins disque restent mais l'objet PIL mémoire
    pointe sur l'ancienne image).
  - Pas d'éviction LRU : on arrête simplement d'insérer au-delà de
    _MAX_CACHE_ITEMS.  Pour un classeur de 500 cartes uniques, la mémoire
    restera bornée (~50-100 MB).
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

import threading
from typing import Optional
from PIL import Image

# Cache des images PIL chargées (clé = chemin absolu)
_PIL_CACHE: dict[str, Image.Image] = {}
_CACHE_LOCK = threading.Lock()
_MAX_CACHE_ITEMS = 500  # ~50-100 MB RAM pour des images cards_small YGOPRODeck


def get_or_load_pil_image(file_path: str) -> Optional[Image.Image]:
    """
    Charge une image PIL avec cache thread-safe.

    Args:
        file_path: Chemin absolu vers le fichier image sur disque.

    Returns:
        L'objet PIL.Image en mode RGB si succès (et le met en cache),
        None si le fichier est absent ou illisible.
    """
    # Lecture cache — verrou court
    with _CACHE_LOCK:
        cached = _PIL_CACHE.get(file_path)
        if cached is not None:
            return cached

    # Absent du cache → charger depuis le disque (hors verrou pour ne pas
    # bloquer les autres threads pendant l'I/O).
    try:
        pil = Image.open(file_path).convert("RGB")
    except Exception:
        return None

    # Insertion cache sous verrou avec double-check
    with _CACHE_LOCK:
        if file_path not in _PIL_CACHE and len(_PIL_CACHE) < _MAX_CACHE_ITEMS:
            _PIL_CACHE[file_path] = pil
    return pil


def clear_cache() -> None:
    """Vide le cache (ex : changement de source d'images)."""
    with _CACHE_LOCK:
        _PIL_CACHE.clear()


"""
app_config.py — Accès centralisé à bdd/app_config.json.

Remplace les 19 open() éparpillés dans config_langue.py, config_image_source.py
et i18n/__init__.py par un point d'entrée unique.

API publique :
    get(key, default=None)  → valeur lue depuis le fichier (ou default)
    set(key, value)         → écrit la clé en préservant toutes les autres
    invalidate_cache()      → force la relecture du fichier au prochain get()

OPTIMISATION : cache en mémoire du JSON pour éviter les I/O disque répétés.
Le cache est peuplé au premier get() et mis à jour à chaque set().
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

import json
import os
import threading

from module.centralisation_dossier import APP_CONFIG_FILE
from module.logger_app import log

# Cache en mémoire du contenu de app_config.json
_cache: dict | None = None
_cache_lock = threading.Lock()


def _load_from_disk() -> dict:
    """Lit le fichier JSON depuis le disque. Retourne {} si absent/corrompu."""
    if not os.path.exists(APP_CONFIG_FILE):
        return {}
    try:
        with open(APP_CONFIG_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def _ensure_cache() -> dict:
    """S'assure que le cache est peuplé. Retourne le dict cache."""
    global _cache
    if _cache is None:
        with _cache_lock:
            if _cache is None:  # double-check pattern
                _cache = _load_from_disk()
    return _cache


def invalidate_cache() -> None:
    """Force la relecture du fichier au prochain get()."""
    global _cache
    with _cache_lock:
        _cache = None


def get(key: str, default=None):
    """
    Lit la valeur associée à key dans app_config.json (via cache).
    Retourne default si la clé manque.
    """
    data = _ensure_cache()
    return data.get(key, default)


def set(key: str, value) -> None:
    """
    Écrit key=value dans app_config.json en préservant toutes les autres clés.
    Crée le fichier (et les dossiers intermédiaires) s'il n'existe pas.
    Met à jour le cache en mémoire.
    """
    global _cache
    # Charger data (depuis cache si possible, sinon disque) — copie pour
    # ne pas muter le cache avant que l'écriture disque soit confirmée.
    data = dict(_ensure_cache())
    data[key] = value

    try:
        os.makedirs(os.path.dirname(APP_CONFIG_FILE), exist_ok=True)
        with open(APP_CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        # Écriture OK → mettre à jour le cache
        with _cache_lock:
            _cache = data
    except Exception as e:
        log.warning(f"app_config.set({key!r}) : impossible de sauvegarder : {e}")
        # En cas d'échec, invalide pour forcer une relecture cohérente
        invalidate_cache()

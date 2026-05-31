"""
fr_names_service.py — Utilitaires liés à YGOJSON et aux noms français.

Fonctions exportées :
  get_ygojson_remote_updated_at() → str | None   (API GitHub)
  get_ygojson_version()           → str | None   (cache local)
  save_fr_names_cache(...)        → None
  load_fr_names_cache()           → (dict, set)
  get_cartes_missing_fr()         → list[tuple]
  _cache_needs_refresh(remote)    → bool          (utilisé par BDD_creation)

Les fonctions fetch_fr_names_ygojson() et get_fr_names() ont été supprimées :
leurs données (noms FR, données sets) sont produites directement dans
BDD_creation.run_init() via ygojson_parser.py, qui est la source canonique.
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
import json
import requests
from datetime import datetime

from module.centralisation_dossier import FR_NAMES_CACHE, CARDINFO_DB

YGOJSON_API_URL = "https://api.github.com/repos/iconmaster5326/YGOJSON/releases/tags/v1"
_HEADERS        = {"User-Agent": "YugiohCollectionManager/1.0"}


# ─────────────────────────────────────────────────────────────────────────────
# Utilitaires datetime
# ─────────────────────────────────────────────────────────────────────────────

def _parse_iso_utc(ts: str) -> datetime:
    """Parse une date ISO 8601 (avec ou sans Z) en datetime UTC aware."""
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


# ─────────────────────────────────────────────────────────────────────────────
# API GitHub
# ─────────────────────────────────────────────────────────────────────────────

def get_ygojson_remote_updated_at() -> str | None:
    """
    Requête légère sur l'API GitHub releases pour lire updated_at de aggregate.zip.
    Retourne la chaîne ISO 8601 ou None si erreur.
    """
    try:
        r = requests.get(YGOJSON_API_URL, timeout=10, headers=_HEADERS)
        r.raise_for_status()
        for asset in r.json().get("assets", []):
            if asset.get("name") == "aggregate.zip":
                return asset.get("updated_at")
        return None
    except Exception:
        return None


# ─────────────────────────────────────────────────────────────────────────────
# Cache local fr_names_cache.json
# ─────────────────────────────────────────────────────────────────────────────




def save_fr_names_cache(
    fr_names: dict,
    confirmed_ids: set,
    ygojson_updated_at: str | None = None,
) -> None:
    """
    Sauvegarde {password_str: name_fr} + confirmed_ids dans le cache.
    Stocke ygojson_updated_at dans _meta pour les comparaisons futures.
    """
    data = dict(fr_names)
    meta: dict = {"fetched_at": datetime.now().isoformat()}
    if ygojson_updated_at:
        meta["ygojson_updated_at"] = ygojson_updated_at
    data["_meta"]          = meta
    data["_confirmed_ids"] = list(confirmed_ids)
    try:
        with open(FR_NAMES_CACHE, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False)
    except Exception:
        pass


# ─────────────────────────────────────────────────────────────────────────────
# Cartes sans traduction FR (cardinfo.db)
# ─────────────────────────────────────────────────────────────────────────────

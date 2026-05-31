"""
booster_service.py — Récupération et cache local des images de booster.

Chaque classeur (RA02, RA05, VASM, LOB...) a une image officielle de booster
stockée dans cardinfo.db.set_locales.booster_image_url. Ce service :

  1. Lit l'URL depuis cardinfo.db via le prefix du classeur
  2. Télécharge dans img/boosters/<PREFIX>.<ext>
  3. Retourne le chemin local si le fichier est sur disque, sinon None
     (le caller peut afficher un fallback ou déclencher le DL async)

Le téléchargement est optionnel et asynchrone : l'UI ne doit pas bloquer
en attendant le DL. La première ouverture de l'accueil après création d'un
classeur affichera donc le fallback, et les suivantes auront l'image booster.
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
from urllib.parse import urlparse

from module.centralisation_dossier import (
    CARDINFO_DB, IMAGES_BOOSTERS_FOLDER, sqlite_ctx,
)
from module.logger_app import log

# Lock pour limiter les téléchargements simultanés sur le même prefix
_dl_locks: dict[str, threading.Lock] = {}
_locks_mutex = threading.Lock()


def _get_lock(prefix: str) -> threading.Lock:
    with _locks_mutex:
        lk = _dl_locks.get(prefix)
        if lk is None:
            lk = threading.Lock()
            _dl_locks[prefix] = lk
        return lk


def _extension_from_url(url: str) -> str:
    """Retourne l'extension (.png, .jpg, .webp) avec fallback .png."""
    try:
        path = urlparse(url).path
        ext = os.path.splitext(path)[1].lower()
        if ext in (".png", ".jpg", ".jpeg", ".webp", ".gif"):
            return ext
    except Exception:
        pass
    return ".png"


def _booster_local_path(prefix: str, ext: str = ".png") -> str:
    return os.path.join(IMAGES_BOOSTERS_FOLDER, f"{prefix.upper()}{ext}")


def find_local_booster(prefix: str) -> str | None:
    """Retourne le chemin local de l'image du booster si elle existe déjà.

    Teste plusieurs extensions car l'URL source peut être en png/jpg/webp.
    """
    prefix = prefix.upper()
    for ext in (".png", ".jpg", ".jpeg", ".webp", ".gif"):
        p = _booster_local_path(prefix, ext)
        if os.path.exists(p):
            return p
    return None


def get_booster_url(prefix: str) -> str | None:
    """Lit cardinfo.db pour trouver l'URL du booster correspondant au prefix.

    Retourne None si :
      - cardinfo.db absent
      - tables set_locales / sets non peuplées (BDD non initialisée)
      - aucune locale correspondante avec image non vide pour ce prefix

    Comportement selon la nature du préfixe :
      - Préfixe TCG nu (`CROS`, `RA02`)        → recherche dans les locales
        'en' et 'eu' avec strip SUBSTR+INSTR du suffixe (cas historique).
      - Préfixe OCG complet (`LOCH-JP`, `CROS-JP`, etc.) → recherche
        ÉGALITÉ EXACTE sur le prefix complet, dans la locale OCG
        correspondante (`ja` pour JP/JA, `ko` pour KR/KO, etc.).

    On ne renvoie jamais une URL vide ou blanche.
    """
    if not os.path.exists(CARDINFO_DB):
        return None
    prefix = prefix.upper()
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            cursor.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='set_locales'"
            )
            if not cursor.fetchone():
                return None

            # Détection préfixe OCG
            from module.config.preferences import a_suffixe_ocg as _is_ocg
            if _is_ocg(prefix):
                # Mapping suffixe → langue YGOJSON
                # JP/JA → 'ja' ; KR/KO → 'ko' ; AE → 'ae' ; SC → 'zh' ; TC → 'zh'
                suffixe = prefix.rsplit("-", 1)[1] if "-" in prefix else ""
                _MAP_LANG = {
                    "JP": "ja", "JA": "ja",
                    "KR": "ko", "KO": "ko",
                    "AE": "ae",
                    "SC": "zh", "TC": "zh",
                }
                langue_ocg = _MAP_LANG.get(suffixe)
                if not langue_ocg:
                    return None
                cursor.execute("""
                    SELECT sl.booster_image_url
                    FROM set_locales sl
                    WHERE UPPER(sl.prefix) = ?
                      AND sl.language = ?
                      AND sl.booster_image_url IS NOT NULL
                      AND sl.booster_image_url != ''
                    LIMIT 1
                """, (prefix, langue_ocg))
            else:
                # Comportement TCG historique : strip suffixe + filtre 'en'/'eu'.
                cursor.execute("""
                    SELECT sl.booster_image_url
                    FROM set_locales sl
                    WHERE SUBSTR(sl.prefix, 1,
                                 INSTR(sl.prefix || '-', '-') - 1) = ?
                      AND sl.language IN ('en', 'eu')
                      AND sl.booster_image_url IS NOT NULL
                      AND sl.booster_image_url != ''
                    LIMIT 1
                """, (prefix,))
            row = cursor.fetchone()
            if row and row[0]:
                return row[0].strip() or None
    except Exception as e:
        log.warning(f"get_booster_url({prefix}): {e}")
    return None


def download_booster_if_needed(prefix: str) -> str | None:
    """Télécharge l'image booster si absente, retourne le chemin local.

    Bloquant (à appeler depuis un thread worker, pas depuis l'UI thread).
    Retourne None si :
      - URL introuvable en DB
      - Erreur HTTP / fichier vide

    Thread-safe : un lock par prefix évite les téléchargements redondants.
    """
    prefix = prefix.upper()

    # Déjà local ?
    existing = find_local_booster(prefix)
    if existing:
        return existing

    url = get_booster_url(prefix)
    if not url:
        # Fallback Yugipedia : YGOPRODeck ne fournit pas toujours
        # booster_image_url (ex CH01, CH02, LOCR-JP). Yugipedia nomme ses
        # covers d'après le set_code → on les retrouve par préfixe.
        # Priorité de langue : FR puis EN, puis n'importe laquelle.
        try:
            from module.img_dl.yugipedia_booster import get_cover_url
            url = get_cover_url(prefix, ("FR", "EN"))
            if url:
                log.info(f"Cover {prefix} récupérée via Yugipedia (fallback)")
        except Exception as e:
            log.warning(f"Fallback Yugipedia cover ({prefix}): {e}")
    if not url:
        return None

    with _get_lock(prefix):
        # Double-check après acquisition du lock (un autre thread a pu
        # télécharger entre-temps)
        existing = find_local_booster(prefix)
        if existing:
            return existing

        ext = _extension_from_url(url)
        dest = _booster_local_path(prefix, ext)

        try:
            import requests
            os.makedirs(IMAGES_BOOSTERS_FOLDER, exist_ok=True)
            resp = requests.get(url, stream=True, timeout=15,
                                headers={"User-Agent": "YugiohCollectionManager/1.0"})
            resp.raise_for_status()
            with open(dest, "wb") as f:
                for chunk in resp.iter_content(8192):
                    if chunk:
                        f.write(chunk)
            # Vérif sanity : fichier non vide
            if os.path.getsize(dest) < 200:
                os.remove(dest)
                return None
            return dest
        except Exception as e:
            log.warning(f"download_booster_if_needed({prefix}): {e}")
            # Nettoie un éventuel fichier partiel
            if os.path.exists(dest):
                try:
                    os.remove(dest)
                except Exception:
                    pass
            return None


def get_booster_image_path(prefix: str, auto_download: bool = False) -> str | None:
    """API principale : retourne le chemin local du booster.

    auto_download=False : retourne le chemin si déjà local, None sinon.
                          Non bloquant, utilisable depuis l'UI thread.
    auto_download=True  : télécharge si absent (bloquant, thread worker).
    """
    if auto_download:
        return download_booster_if_needed(prefix)
    return find_local_booster(prefix)

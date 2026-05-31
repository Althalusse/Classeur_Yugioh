"""
yugipedia_booster.py — Récupération de l'image de cover (booster / deck / tin)
d'un set depuis Yugipedia, en fallback quand YGOPRODeck ne fournit pas
`booster_image_url`.

Yugipedia nomme ses fichiers de cover d'après le set_code :
    {SETCODE}-Booster{LANG}.png   (ex RA04-BoosterEN.png)
    {SETCODE}-Deck{LANG}.png      (ex CH02-DeckEN.png)
    {SETCODE}-BoosterBox{REGION}.png, {SETCODE}-Tin*.png, ...

On liste donc toutes les images dont le nom commence par « {PREFIX}- » via
l'API MediaWiki (list=allimages&aiprefix=), puis on choisit la meilleure
selon la langue préférée (FR d'abord, puis EN, puis n'importe laquelle) et le
type (cover produit simple privilégié sur les variantes box/réimpression).

API publique :
    get_cover_url(set_code, langues=("FR", "EN")) -> str | None

Best-effort : retourne None si rien trouvé ou en cas d'erreur réseau (le
caller — booster_service — gère le fallback / l'absence de cover).
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
import re
import time

import requests

from module.logger_app import log

_API = "https://yugipedia.com/api.php"
_HEADERS = {"User-Agent": "YGO-Collection-Manager/1.0 (cover fetch)"}
_TIMEOUT = 20

# Cache mémoire (process) : prefix -> url (ou None). Vidé à chaque lancement.
_URL_CACHE: dict = {}

# Throttle de politesse vers Yugipedia.
_MIN_INTERVAL = 0.4
_last_call = 0.0

# Codes langue/région connus, pour extraire la langue d'un nom de fichier.
_LANGS = [
    "FR", "EN", "DE", "IT", "PT", "SP", "ES", "JP", "JA", "KR", "KO",
    "AE", "SC", "TC", "EU", "NA", "AU", "OC",
]
# Mots-clés indiquant une cover de produit (vs une image de carte isolée).
_TYPES = ["Booster", "Deck", "Box", "Tin", "Case", "Pack"]
# Suffixes pénalisés (variantes / réimpressions).
_SUFFIX_PENALISES = ("VER2", "VER3", "REPRINT", "ALT", "PROMO")


def _throttle():
    global _last_call
    delta = time.monotonic() - _last_call
    if delta < _MIN_INTERVAL:
        time.sleep(_MIN_INTERVAL - delta)
    _last_call = time.monotonic()


def _prefix(set_code: str) -> str:
    """Extrait le préfixe de recherche d'un set_code.

    'RA04-EN001' -> 'RA04' ; 'LOCR-JP' -> 'LOCR' ; 'VASM' -> 'VASM'.
    (Le 1er segment avant '-' fonctionne pour les codes TCG complets ET les
    préfixes OCG du type 'LOCR-JP'.)
    """
    return (set_code or "").split("-", 1)[0].strip().upper()


def _lister_images(prefix: str):
    """Liste (nom, url) des fichiers Yugipedia commençant par « {prefix}- »."""
    params = {
        "action": "query",
        "list": "allimages",
        "aiprefix": f"{prefix}-",
        "ailimit": "100",
        "aiprop": "url",
        "format": "json",
        "formatversion": "2",
    }
    try:
        _throttle()
        resp = requests.get(_API, params=params, headers=_HEADERS, timeout=_TIMEOUT)
        resp.raise_for_status()
        imgs = resp.json().get("query", {}).get("allimages", [])
        return [(i.get("name", ""), i.get("url", "")) for i in imgs if i.get("url")]
    except Exception as e:
        log.warning(f"Yugipedia _lister_images({prefix}): {e}")
        return []


def _langue_du_nom(reste: str) -> str:
    """Devine le code langue d'un nom de fichier (sans préfixe ni extension).

    'BoosterEN' -> 'EN' ; 'DeckEN-Ver2' -> 'EN' ; 'BoosterBoxEU' -> 'EU'.
    """
    segment = reste.split("-", 1)[0].upper()   # ignore le suffixe -Ver2, etc.
    for lg in _LANGS:
        if segment.endswith(lg):
            return lg
    return ""


def _score(nom: str, prefix: str, langues: list) -> int:
    """Score un nom de fichier candidat. Plus haut = meilleur cover."""
    base = os.path.splitext(nom)[0]              # retire .png
    if not base.upper().startswith(prefix + "-"):
        return -1
    reste = base[len(prefix) + 1:]               # après '{PREFIX}-'
    reste_up = reste.upper()

    # Type de produit
    type_score = 0
    for t in _TYPES:
        if t.upper() in reste_up:
            if t in ("Booster", "Deck"):
                type_score = 20
            elif t == "Tin":
                type_score = 12
            else:                                # Box / Case / Pack
                type_score = 6
            break
    if type_score == 0:
        # Pas un mot-clé cover connu : peut être une carte isolée -> faible.
        type_score = 1

    # Langue préférée
    lg = _langue_du_nom(reste)
    if lg in langues:
        lang_score = 100 - langues.index(lg) * 10   # FR=100, EN=90, ...
    elif lg:
        lang_score = 30                             # langue connue non préférée
    else:
        lang_score = 10                             # langue indéterminée

    # Pénalité variantes / réimpressions
    penalite = -5 if any(s in reste_up for s in _SUFFIX_PENALISES) else 0

    return type_score + lang_score + penalite


def get_cover_url(set_code: str, langues=("FR", "EN")):
    """URL de la meilleure image de cover pour ce set, via Yugipedia.

    Priorise la langue (FR puis EN par défaut), puis le type de cover.
    Retourne None si aucune image ou erreur réseau.
    """
    prefix = _prefix(set_code)
    if not prefix:
        return None
    if prefix in _URL_CACHE:
        return _URL_CACHE[prefix]

    langues = [l.upper() for l in langues]
    images = _lister_images(prefix)
    if not images:
        _URL_CACHE[prefix] = None
        return None

    meilleur_url = None
    meilleur_score = 0
    for nom, url in images:
        s = _score(nom, prefix, langues)
        if s > meilleur_score:
            meilleur_score = s
            meilleur_url = url

    _URL_CACHE[prefix] = meilleur_url
    return meilleur_url

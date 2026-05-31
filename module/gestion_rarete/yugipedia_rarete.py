"""
yugipedia_rarete.py — Récupération des raretés réelles d'une carte/set
depuis Yugipedia (source de vérité).

Sert à corriger les annotations parasites de YGOPRODeck dans le champ
set_rarity. Exemple connu : CH02-EN001 a, côté YGOPRODeck, une entrée
set_rarity='New artwork' qui a remplacé la vraie rareté 'Ultra Rare' ;
Yugipedia liste correctement « Ultra Rare, Secret Rare, Starlight Rare ».

API publique :
    get_raretes_carte_set(card_name, set_code) -> list[str] | None

Retourne la liste des raretés RECONNUES (normalisées via le référentiel
local) pour ce set_code, ou None si la page/section est introuvable ou en
cas d'erreur réseau (le caller doit alors prévoir un fallback sûr).

Best-effort : interroge la page Yugipedia de la carte (par nom anglais),
parse la set-list du wikitext, isole la ligne du set_code. Un cache mémoire
(process) évite de re-télécharger le wikitext d'une même carte.
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

import re
import time
import urllib.parse

import requests

from module.gestion_rarete.raretes_reference import name_to_code
from module.logger_app import log

_API = "https://yugipedia.com/api.php"
_HEADERS = {"User-Agent": "YGO-Collection-Manager/1.0 (rarity correction)"}
_TIMEOUT = 20

# Cache mémoire : nom de carte -> wikitext (ou None si échec). Vidé à chaque
# lancement de l'application. Évite de spammer Yugipedia.
_WIKITEXT_CACHE: dict = {}

# Délai minimal entre deux requêtes Yugipedia (politesse / anti rate-limit).
_MIN_INTERVAL = 0.4
_last_call = 0.0


def _throttle():
    global _last_call
    delta = time.monotonic() - _last_call
    if delta < _MIN_INTERVAL:
        time.sleep(_MIN_INTERVAL - delta)
    _last_call = time.monotonic()


def _fetch_wikitext(card_name: str):
    """Télécharge le wikitext de la page Yugipedia (suit les redirections)."""
    if card_name in _WIKITEXT_CACHE:
        return _WIKITEXT_CACHE[card_name]

    params = {
        "action": "parse",
        "page": card_name,
        "prop": "wikitext",
        "redirects": "1",
        "format": "json",
        "formatversion": "2",
    }
    try:
        _throttle()
        resp = requests.get(_API, params=params, headers=_HEADERS, timeout=_TIMEOUT)
        resp.raise_for_status()
        data = resp.json()
        wt = data.get("parse", {}).get("wikitext")
        if isinstance(wt, dict):       # formatversion=1 fallback
            wt = wt.get("*")
        _WIKITEXT_CACHE[card_name] = wt
        return wt
    except Exception as e:
        log.warning(f"Yugipedia _fetch_wikitext({card_name!r}): {e}")
        _WIKITEXT_CACHE[card_name] = None
        return None


def _clean_rarity_token(tok: str) -> str:
    """Nettoie un libellé de rareté issu du wikitext ([[lien]], templates)."""
    tok = tok.strip()
    # [[Ultra Rare]] ou [[Ultra Rare|UR]] -> garder le 1er segment
    m = re.match(r"\[\[([^\]|]+)", tok)
    if m:
        tok = m.group(1)
    tok = tok.replace("[[", "").replace("]]", "").replace("{{", "").replace("}}", "")
    return tok.strip()


def _parse_raretes_pour_set(wikitext: str, set_code: str):
    """Extrait la liste de raretés de la set-list pour le set_code donné.

    Les lignes de set-list ont la forme :
        SET-CODE; Set Name; Rarity1, Rarity2, ...
    (parfois préfixées par « | »). On isole la ligne, on prend le 3e champ.
    """
    cible = set_code.strip().upper()
    for raw in wikitext.splitlines():
        line = raw.strip().lstrip("|").strip()
        head = line.split(";", 1)[0].strip().upper()
        if head != cible:
            continue
        parts = [p.strip() for p in line.split(";")]
        if len(parts) < 3:
            return None
        raretes = []
        for tok in parts[2].split(","):
            nom = _clean_rarity_token(tok)
            if nom:
                raretes.append(nom)
        return raretes or None
    return None


def get_raretes_carte_set(card_name: str, set_code: str):
    """Raretés RECONNUES d'une carte pour un set_code, via Yugipedia.

    - Retourne une liste de noms de raretés (telles que reconnues par le
      référentiel local), sans doublon, en préservant l'ordre Yugipedia.
    - Retourne None si la carte/section est introuvable ou erreur réseau.
    - Les libellés Yugipedia non reconnus comme raretés (annotations) sont
      écartés, comme le fait déjà le parser YGOJSON.
    """
    if not card_name or not set_code:
        return None
    wt = _fetch_wikitext(card_name)
    if not wt:
        return None
    brutes = _parse_raretes_pour_set(wt, set_code)
    if not brutes:
        return None

    out = []
    vus = set()
    for nom in brutes:
        code = name_to_code(nom)
        if code is None:          # libellé non-rareté (annotation) -> ignoré
            continue
        if code in vus:           # même rareté déjà listée -> dédoublonnée
            continue
        vus.add(code)
        out.append(nom)
    return out or None

"""
config_image_source.py — Gestion de la source des images de cartes.

Deux sources disponibles :
  - "YGOPRODECK" : images HD depuis images.ygoprodeck.com  (JPEG, ~400 px)
  - "YUGIPEDIA"  : images spécifiques au print depuis ms.yugipedia.com
                   (PNG haute qualité, artwork alternatif exact par rareté)

Sauvegarde dans bdd/app_config.json via module.app_config.
Source par défaut : YGOPRODECK (disponibilité maximale, pas de 404).
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

import module.app_config as _cfg

_SOURCES_DISPONIBLES = ["YGOPRODECK", "YUGIPEDIA"]
_SOURCE_DEFAUT       = "YGOPRODECK"

# URL de base YGOPRODeck — suffixe = password Konami + ".jpg"
YGOPRODECK_IMG_BASE = "https://images.ygoprodeck.com/images/cards/{}.jpg"

# Cache module-level — évite de relire app_config.json à chaque carte affichée.
# Invalidé et mis à jour par save_image_source().
_source_cache: str | None = None


def load_image_source() -> str:
    """Retourne 'YGOPRODECK' ou 'YUGIPEDIA'. Défaut : 'YGOPRODECK'.
    Utilise un cache module-level pour éviter les lectures répétées."""
    global _source_cache
    if _source_cache is not None:
        return _source_cache
    src = _cfg.get("image_source", _SOURCE_DEFAUT)
    _source_cache = src if src in _SOURCES_DISPONIBLES else _SOURCE_DEFAUT
    return _source_cache


def save_image_source(source: str) -> None:
    """Sauvegarde la source dans app_config.json et met à jour le cache."""
    global _source_cache
    if source not in _SOURCES_DISPONIBLES:
        source = _SOURCE_DEFAUT
    _cfg.set("image_source", source)
    _source_cache = source  # mise à jour immédiate du cache


def build_image_url(card_image_url: str | None, card_image_id: int | None) -> str | None:
    """
    Retourne l'URL à télécharger selon la source configurée.

    - YGOPRODECK : construit l'URL depuis card_image_id (password Konami).
                   Fallback sur card_image_url si card_image_id absent.
    - YUGIPEDIA  : utilise card_image_url tel quel (peut être Yugipedia ou
                   YGOPRODeck si le print n'a pas d'URL Yugipedia).
    """
    source = load_image_source()

    if source == "YGOPRODECK":
        if card_image_id:
            return YGOPRODECK_IMG_BASE.format(card_image_id)
        return card_image_url or None

    # YUGIPEDIA : utilise l'URL stockée dans card_image_url
    return card_image_url or None


# ─────────────────────────────────────────────────────────────────────────────
# URL de fallback (source alternative)
# ─────────────────────────────────────────────────────────────────────────────

def _lookup_yugipedia_url(card_image_id: int | None) -> str | None:
    """
    Cherche dans cardinfo.db la card_url (URL Yugipedia) correspondant à
    ygoprodeck_image_id == card_image_id.

    Retourne None si :
      - card_image_id absent
      - cardinfo.db introuvable
      - pas de correspondance
      - card_url vide ou NULL

    Lookup O(log n) grâce à l'index idx_card_images_ygo_id.
    Ne log pas les erreurs : le caller décide quoi faire en cas d'échec.
    """
    if not card_image_id:
        return None
    try:
        # Import local pour éviter les imports circulaires au chargement du module
        from module.centralisation_dossier import CARDINFO_DB, sqlite_ctx
        import os
        if not os.path.exists(CARDINFO_DB):
            return None
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.execute(
                "SELECT card_url FROM card_images "
                "WHERE ygoprodeck_image_id = ? "
                "  AND card_url IS NOT NULL AND card_url != '' "
                "LIMIT 1",
                (int(card_image_id),)
            )
            row = cursor.fetchone()
            return row[0] if row else None
    except Exception:
        return None


def build_fallback_url(card_image_url: str | None,
                       card_image_id: int | None) -> str | None:
    """
    Retourne l'URL de la source ALTERNATIVE (opposée à la source active).

    Utilisée par le worker de téléchargement pour tenter une seconde source
    quand la première a échoué.

    - Source active = YGOPRODECK → retourne l'URL Yugipedia trouvée dans
      cardinfo.db.card_images.card_url (lookup via ygoprodeck_image_id).
    - Source active = YUGIPEDIA  → retourne l'URL YGOPRODeck construite
      depuis card_image_id.

    Retourne None si aucune URL alternative n'est disponible (par exemple
    lookup Yugipedia sans match dans cardinfo.db).
    """
    source = load_image_source()

    if source == "YGOPRODECK":
        # Fallback = Yugipedia : récupérer card_url via card_image_id
        return _lookup_yugipedia_url(card_image_id)

    # Source = YUGIPEDIA : fallback = YGOPRODECK construit depuis l'ID
    if card_image_id:
        return YGOPRODECK_IMG_BASE.format(card_image_id)
    return None

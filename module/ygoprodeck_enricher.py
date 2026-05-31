"""
ygoprodeck_enricher.py — Téléchargement et enrichissement via l'API YGOPRODeck.

Responsabilité unique : récupérer les données YGOPRODeck et les appliquer
aux structures issues du parsing YGOJSON (stats, frame_type, banlist,
résolution des artworks alternatifs).

Fonctions exportées :
  fetch_ygoprodeck(log)                           → cards_raw
  build_ygoprodeck_index(cards_raw)               → {password: card_dict}
  enrich_cards_rows(cards_rows, index)            → cards_rows enrichies
  expand_prints_for_multi_art(prints, cards, imgs) → prints résolus/expandus
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

import requests

YGOPRODECK_URL = "https://db.ygoprodeck.com/api/v7/cardinfo.php?includeAliased=true"
_HEADERS       = {"User-Agent": "YugiohCollectionManager/1.0"}


# ─────────────────────────────────────────────────────────────────────────────
# Téléchargement
# ─────────────────────────────────────────────────────────────────────────────

def fetch_ygoprodeck(log=None) -> list:
    """
    Télécharge toutes les cartes depuis l'API YGOPRODeck.
    Retourne la liste des cartes, ou [] en cas d'échec.
    """
    def _log(msg, color="blue"):
        if log:
            log(msg, color)

    try:
        _log("Téléchargement YGOPRODeck API (includeAliased)...")
        response = requests.get(YGOPRODECK_URL, stream=True, timeout=60)
        if response.status_code != 200:
            raise Exception(f"HTTP {response.status_code}")

        import json
        total, downloaded, chunks = int(response.headers.get("Content-Length", 0)), 0, []
        for chunk in response.iter_content(chunk_size=131072):
            if chunk:
                chunks.append(chunk)
                downloaded += len(chunk)
                if total:
                    pct = int(downloaded / total * 100)
                    _log(f"Téléchargement YGOPRODeck... {pct}% ({downloaded // 1048576}/{total // 1048576} MB)")

        data  = json.loads(b"".join(chunks))
        cards = data.get("data", [])
        _log(f"✅ YGOPRODeck : {len(cards)} entrées", "green")
        return cards

    except Exception as e:
        _log(f"❌ Erreur YGOPRODeck : {e}", "red")
        return []


# ─────────────────────────────────────────────────────────────────────────────
# Enrichissement stats
# ─────────────────────────────────────────────────────────────────────────────

def build_ygoprodeck_index(ygoprodeck_cards: list) -> dict:
    """Construit { password_int: card_dict } depuis la liste YGOPRODeck."""
    index = {}
    for card in ygoprodeck_cards:
        card_id = card.get("id")
        if card_id is not None:
            try:
                index[int(card_id)] = card
            except (ValueError, TypeError):
                pass
    return index


def enrich_cards_rows(cards_rows: list, ygoprodeck_index: dict) -> list:
    """
    Pour chaque ligne de `cards`, complète les colonnes stats via YGOPRODeck.
    Colonnes enrichies : frame_type, atk, def, level, attribute, race,
                         banlist_tcg, banlist_ocg.
    """
    enriched = []
    for row in cards_rows:
        (uuid, ygoprodeck_id, card_type, subcategory,
         _frame, _atk, _def, _level, _attr, _race,
         _ban_tcg, _ban_ocg, name_fr_confirmed) = row

        if ygoprodeck_id and ygoprodeck_id in ygoprodeck_index:
            ygo     = ygoprodeck_index[ygoprodeck_id]
            banlist = ygo.get("banlist_info", {}) or {}

            def _int(v):
                try:
                    return int(v) if v is not None else None
                except (ValueError, TypeError):
                    return None

            enriched.append((
                uuid, ygoprodeck_id, card_type, subcategory,
                ygo.get("frameType", ""),
                _int(ygo.get("atk")),
                _int(ygo.get("def")),
                _int(ygo.get("level")),
                ygo.get("attribute", ""),
                ygo.get("race", ""),
                banlist.get("ban_tcg", ""),
                banlist.get("ban_ocg", ""),
                name_fr_confirmed,
            ))
        else:
            enriched.append(row)

    return enriched


# ─────────────────────────────────────────────────────────────────────────────
# Résolution et expansion des artworks
# ─────────────────────────────────────────────────────────────────────────────

def expand_prints_for_multi_art(
    prints_rows_raw: list,
    ygoprodeck_cards: list,
    images_rows: list,
) -> list:
    """
    Deux corrections en une passe :

    1. RÉSOLUTION card_image_uuid
       Les print UUIDs de YGOJSON (contents[].cards[].id) sont distincts des
       artwork UUIDs de card_images. On résout chaque print_uuid vers le vrai
       card_images.uuid grâce aux passwords YGOPRODeck.

    2. EXPANSION MULTI-ARTWORKS
       Quand YGOPRODeck confirme plusieurs passwords pour le même
       (set_code, rarity, card), on génère une ligne set_print par artwork.
       Ex : DUSA-EN072 Rescue Cat → 2 lignes (art1 + art2).

    Si YGOPRODeck est indisponible (liste vide) : prints retournés inchangés.
    """
    if not ygoprodeck_cards:
        return prints_rows_raw

    # Index 1 : password → img_uuid
    pw_to_img: dict[int, str] = {}
    for (img_uuid, card_uuid, password, _art, _card) in images_rows:
        if password is None:
            continue
        try:
            pw = int(password)
        except (ValueError, TypeError):
            continue
        if pw not in pw_to_img:
            pw_to_img[pw] = img_uuid

    # Index 2 : card_uuid → liste de passwords
    card_to_pws: dict[str, list[int]] = {}
    for (img_uuid, card_uuid, password, _art, _card) in images_rows:
        if password is None or not card_uuid:
            continue
        try:
            pw = int(password)
        except (ValueError, TypeError):
            continue
        lst = card_to_pws.setdefault(card_uuid, [])
        if pw not in lst:
            lst.append(pw)

    # Index 3 : (set_code, rarity) → [passwords confirmés]
    sc_rarity_pws: dict[tuple, list[int]] = {}
    for card in ygoprodeck_cards:
        pw_raw = card.get("id")
        if pw_raw is None:
            continue
        try:
            pw = int(pw_raw)
        except (ValueError, TypeError):
            continue
        for cs in card.get("card_sets", []):
            sc     = cs.get("set_code", "")
            rarity = cs.get("set_rarity", "")
            if not sc:
                continue
            lst = sc_rarity_pws.setdefault((sc, rarity), [])
            if pw not in lst:
                lst.append(pw)

    # Résolution + expansion
    expanded: list[dict] = []

    for p in prints_rows_raw:
        card_uuid = p.get("card_uuid", "")
        set_code  = p.get("set_code")
        rarity    = p.get("rarity")
        card_pws  = card_to_pws.get(card_uuid, [])

        if not card_pws:
            expanded.append(p)
            continue

        if not set_code or not rarity:
            new_p = dict(p)
            new_p["card_image_uuid"] = pw_to_img.get(card_pws[0])
            expanded.append(new_p)
            continue

        all_confirmed = sc_rarity_pws.get((set_code, rarity), [])
        art_pws = [pw for pw in all_confirmed if pw in card_pws]

        if not art_pws:
            new_p = dict(p)
            new_p["card_image_uuid"] = pw_to_img.get(card_pws[0])
            expanded.append(new_p)
        elif len(art_pws) == 1:
            new_p = dict(p)
            new_p["card_image_uuid"] = pw_to_img.get(art_pws[0])
            expanded.append(new_p)
        else:
            for pw in art_pws:
                img_uuid = pw_to_img.get(pw)
                if img_uuid is None:
                    continue
                new_p = dict(p)
                new_p["card_image_uuid"] = img_uuid
                expanded.append(new_p)

    return expanded

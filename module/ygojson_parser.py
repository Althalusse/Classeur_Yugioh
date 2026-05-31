"""
ygojson_parser.py — Téléchargement et parsing des données YGOJSON.

Responsabilité unique : transformer les données brutes YGOJSON
(cards.json, sets.json) en structures Python prêtes à être insérées en base.

Fonctions exportées :
  fetch_ygojson(log)             → (cards_raw, sets_raw)
  parse_ygojson_cards(cards_raw) → (cards_rows, texts_rows, images_rows, confirmed_uuids)
  parse_ygojson_sets(sets_raw)   → (sets_rows, locales_rows, prints_rows_raw)
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

import io
import json
import zipfile

import requests

# ─────────────────────────────────────────────────────────────────────────────
# Constantes
# ─────────────────────────────────────────────────────────────────────────────

YGOJSON_URL = "https://github.com/iconmaster5326/YGOJSON/releases/download/v1/aggregate.zip"
_HEADERS    = {"User-Agent": "YugiohCollectionManager/1.0"}

_RARITY_MAP = {
    "ultra":               "Ultra Rare",
    "super":               "Super Rare",
    "rare":                "Rare",
    "common":              "Common",
    "collectors":          "Collector's Rare",
    "secret":              "Secret Rare",
    "starlight":           "Starlight Rare",
    "25thsecret":          "Quarter Century Secret Rare",
    "20thsecret":          "20th Secret Rare",
    "10000secret":         "10000 Secret Rare",
    "ultimate":            "Ultimate Rare",
    "ghost":               "Ghost Rare",
    "gold":                "Gold Rare",
    "goldsecret":          "Gold Secret Rare",
    "goldghost":           "Ghost/Gold Parallel Rare",
    "premiumgold":         "Premium Gold Rare",
    "platinum":            "Platinum Rare",
    "platinumsecret":      "Platinum Secret Rare",
    "prismaticsecret":     "Prismatic Secret Rare",
    "extrasecret":         "Extra Secret Rare",
    "extrasecretparallel": "Extra Secret Parallel Rare",
    "millenium":           "Millennium Rare",
    "milleniumultra":      "Millennium Ultra Rare",
    "milleniumgold":       "Millennium Gold Rare",
    "milleniumsecret":     "Millennium Secret Rare",
    "mosaic":              "Mosaic Rare",
    "shatterfoil":         "Shatterfoil Rare",
    "starfoil":            "Starfoil Rare",
    "shortprint":          "Short Print",
    "commonparallel":      "Common Parallel Rare",
    "rareparallel":        "Rare Parallel Rare",
    "superparallel":       "Super Parallel Rare",
    "ultraparallel":       "Ultra Parallel Rare",
    "secretparallel":      "Secret Parallel Rare",
    "ghostparallel":       "Ghost/Gold Parallel Rare",
    "pharaohs":            "Pharaoh's Rare",
    "ultrasecret":         "Ultra Secret Rare",
    "kcrare":              "Rare",
    "kcultra":             "Ultra Rare",
    "kccommon":            "Common",
    "dtpc":                "Duel Terminal Common Parallel Rare",
    "dtspr":               "Duel Terminal Super Parallel Rare",
    "dtupr":               "Duel Terminal Ultra Parallel Rare",
    "dtscpr":              "Duel Terminal Secret Parallel Rare",
    "dtrpr":               "Duel Terminal Rare Parallel Rare",
    "dtpsp":               "Duel Terminal Normal Parallel Rare",
    "rare-blue":           "Rare",
    "rare-copper":         "Rare",
    "rare-green":          "Rare",
    "rare-purple":         "Rare",
    "rare-red":            "Rare",
    "rare-wedgewood":      "Rare",
    "ultra-blue":          "Ultra Rare",
    "ultra-green":         "Ultra Rare",
    "ultra-purple":        "Ultra Rare",
    "secret-blue":         "Secret Rare",
    "secret-red":          "Secret Rare",
}

_LANG_SUFFIXES = {
    "EN", "FR", "DE", "IT", "PT", "SP", "KR", "JP",
    "AE", "SC", "NA", "EU", "AU", "AS", "A",
}


# ─────────────────────────────────────────────────────────────────────────────
# Téléchargement
# ─────────────────────────────────────────────────────────────────────────────

def fetch_ygojson(log=None) -> tuple[list, list]:
    """
    Télécharge YGOJSON aggregate.zip et retourne (cards_data, sets_data).
    Retourne ([], []) en cas d'échec.
    """
    def _log(msg, color="blue"):
        if log:
            log(msg, color)

    try:
        _log("Téléchargement YGOJSON aggregate.zip...")
        response = requests.get(YGOJSON_URL, timeout=120, stream=True,
                                allow_redirects=True, headers=_HEADERS)
        response.raise_for_status()

        total, downloaded, chunks = int(response.headers.get("Content-Length", 0)), 0, []
        for chunk in response.iter_content(chunk_size=131072):
            if chunk:
                chunks.append(chunk)
                downloaded += len(chunk)
                if total:
                    pct = int(downloaded / total * 100)
                    _log(f"Téléchargement YGOJSON... {pct}% ({downloaded // 1048576}/{total // 1048576} MB)")

        raw_zip = b"".join(chunks)
        _log(f"Téléchargé : {len(raw_zip) / 1048576:.1f} MB", "blue")

        with zipfile.ZipFile(io.BytesIO(raw_zip)) as z:
            names = z.namelist()
            cards_path = next((n for n in names if n.endswith("cards.json")), None)
            sets_path  = next((n for n in names if n.endswith("sets.json")),  None)
            cards_data = json.loads(z.open(cards_path).read().decode("utf-8")) if cards_path else []
            sets_data  = json.loads(z.open(sets_path).read().decode("utf-8"))  if sets_path  else []

        _log(f"✅ YGOJSON : {len(cards_data)} cartes, {len(sets_data)} sets", "green")
        return cards_data, sets_data

    except Exception as e:
        _log(f"❌ Erreur YGOJSON : {e}", "red")
        return [], []


# ─────────────────────────────────────────────────────────────────────────────
# Parsing
# ─────────────────────────────────────────────────────────────────────────────


def parse_ygojson_cards(cards_raw: list) -> tuple[list, list, list, set]:
    """
    Transforme la liste brute YGOJSON cards.json en :
      - cards_rows      : lignes pour la table `cards`
      - texts_rows      : lignes pour la table `card_texts`
      - images_rows     : lignes pour la table `card_images`
      - confirmed_uuids : set des UUIDs avec text.fr confirmé
    """
    cards_rows, texts_rows, images_rows = [], [], []
    confirmed_uuids = set()

    for card in cards_raw:
        uuid = card.get("id", "")
        if not uuid:
            continue

        texts  = card.get("text", {})
        has_fr = bool(texts.get("fr", {}).get("name"))
        if has_fr:
            confirmed_uuids.add(uuid)

        for lang, tdata in texts.items():
            if not isinstance(tdata, dict):
                continue
            name   = tdata.get("name", "") or ""
            effect = tdata.get("effect", "") or tdata.get("pendulum_effect", "") or ""
            if name:
                texts_rows.append((uuid, lang, name, effect))

        first_password = None
        for img in card.get("images", []):
            img_uuid = img.get("id", "")
            password = img.get("password")
            art_url  = img.get("art", "")
            card_url = img.get("card", "")
            if img_uuid:
                images_rows.append((img_uuid, uuid, password, art_url, card_url))
                if first_password is None and password is not None:
                    try:
                        first_password = int(password)
                    except (ValueError, TypeError):
                        pass

        if first_password is None:
            for pw in card.get("passwords", []):
                try:
                    first_password = int(pw)
                    break
                except (ValueError, TypeError):
                    pass

        cards_rows.append((
            uuid,
            first_password,
            card.get("cardType", ""),
            card.get("subcategory", ""),
            None, None, None, None, None, None, None, None,
            1 if has_fr else 0,
        ))

    return cards_rows, texts_rows, images_rows, confirmed_uuids


def parse_ygojson_sets(sets_raw: list) -> tuple[list, list, list]:
    """
    Transforme la liste brute YGOJSON sets.json en :
      - sets_rows       : lignes pour la table `sets`
      - locales_rows    : lignes pour la table `set_locales`
      - prints_rows_raw : dicts bruts pour `set_prints` (set_locale_id à résoudre)
    """
    sets_rows, locales_rows, prints_rows_raw = [], [], []

    for s in sets_raw:
        uuid    = s.get("id", "")
        names   = s.get("name", {}) or {}
        locales = s.get("locales", {}) or {}

        if not uuid:
            continue

        sets_rows.append((
            uuid,
            names.get("en", "") or "",
            names.get("fr", "") or names.get("en", "") or "",
            names.get("de", "") or "",
            names.get("it", "") or "",
            names.get("es", "") or "",
            names.get("ja", "") or "",
            names.get("ko", "") or "",
        ))

        for lang_key, ldata in locales.items():
            if not isinstance(ldata, dict):
                continue
            locales_rows.append((
                uuid,
                lang_key,
                ldata.get("prefix", "") or "",
                ldata.get("date", "")   or "",
                ldata.get("image", "")  or "",
            ))

        for content in s.get("contents", []):
            content_locales = content.get("locales", [])
            editions        = content.get("editions", ["unlimited"])
            edition         = editions[0] if editions else "unlimited"

            for card_entry in content.get("cards", []):
                card_image_uuid = card_entry.get("id", "")
                card_uuid       = card_entry.get("card", "")
                suffix          = card_entry.get("suffix", "")
                rarity_raw      = card_entry.get("rarity", "")
                qty             = card_entry.get("qty", 1)

                if not card_uuid or not rarity_raw:
                    continue
                rarity = _RARITY_MAP.get(rarity_raw)
                if not rarity:
                    continue

                for locale_key in content_locales:
                    locale_data   = locales.get(locale_key, {})
                    locale_prefix = locale_data.get("prefix", "")
                    full_set_code = (locale_prefix + suffix) if (locale_prefix and suffix) else (suffix or None)

                    # URL Yugipedia spécifique au print (cardImages > cardInfo)
                    print_image_url = None
                    if card_image_uuid:
                        url = locale_data.get("cardImages", {}).get(edition, {}).get(card_image_uuid)
                        if url and isinstance(url, str):
                            print_image_url = url
                        else:
                            info = locale_data.get("cardInfo", {}).get(edition, {}).get(card_image_uuid)
                            if isinstance(info, dict):
                                u = info.get("image")
                                if u and isinstance(u, str):
                                    print_image_url = u

                    prints_rows_raw.append({
                        "set_uuid":        uuid,
                        "locale_key":      locale_key,
                        "card_uuid":       card_uuid,
                        "card_image_uuid": card_image_uuid or None,
                        "set_code":        full_set_code,
                        "rarity":          rarity,
                        "edition":         edition,
                        "qty":             qty if qty else 1,
                        "print_image_url": print_image_url,
                    })

    return sets_rows, locales_rows, prints_rows_raw

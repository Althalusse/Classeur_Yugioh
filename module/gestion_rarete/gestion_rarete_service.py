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
from module.centralisation_dossier import CONFIG_FILE, CARDINFO_DB, sqlite_ctx
from module.logger_app import log

_RARITIES_FALLBACK = [
    "Common", "Rare", "Super Rare", "Ultra Rare", "Secret Rare",
    "Platinum Rare", "Platinum Secret Rare", "Ultra Secret Rare",
    "Collector's Rare", "Prismatic Secret Rare", "Ultimate Rare",
    "Quarter Century Secret Rare", "Short Print", "Super Short Print",
    "Extra Secret Rare", "Gold Rare", "Gold Secret Rare", "Premium Gold Rare",
    "Ghost Rare", "Ghost/Gold Rare", "Starlight Rare",
]

_DEFAULT_ORDER = [
    "Common", "Rare", "Super Rare", "Ultra Rare", "Secret Rare",
    "Platinum Rare", "Platinum Secret Rare", "Ultra Secret Rare",
    "Collector's Rare", "Prismatic Secret Rare", "Ultimate Rare",
    "Quarter Century Secret Rare", "Short Print", "Super Short Print",
    "Extra Secret Rare", "Gold Rare", "Gold Secret Rare", "Premium Gold Rare",
    "Ghost Rare", "Ghost/Gold Rare", "Starlight Rare",
    "Mosaic Rare", "Shatterfoil Rare",
    "Duel Terminal Normal Parallel Rare", "Duel Terminal Rare Parallel Rare",
    "Duel Terminal Super Parallel Rare", "Duel Terminal Ultra Parallel Rare",
    "Normal Parallel Rare", "Super Parallel Rare", "Ultra Parallel Rare",
    "10000 Secret Rare", "Duel Terminal Normal Rare Parallel Rare",
    "Extra Secret", "Starfoil", "Starfoil Rare", "Ultra Rare (Pharaoh's Rare)",
]


def get_all_rarities_from_db() -> list:
    """Lit toutes les raretés distinctes depuis set_prints de cardinfo.db."""
    if not os.path.exists(CARDINFO_DB):
        return list(_RARITIES_FALLBACK)
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            cursor.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='set_prints'"
            )
            if not cursor.fetchone():
                return list(_RARITIES_FALLBACK)
            cursor.execute("""
                SELECT DISTINCT rarity FROM set_prints
                WHERE rarity IS NOT NULL AND rarity != ''
                ORDER BY rarity
            """)
            return [row[0] for row in cursor.fetchall()]
    except Exception as e:
        log.warning(f"get_all_rarities : {e}")
        return list(_RARITIES_FALLBACK)


def load_rarity_priorities() -> dict:
    if os.path.exists(CONFIG_FILE):
        try:
            with open(CONFIG_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception as e:
            log.warning(f"load_rarity_priorities : {e}")
            return {}
    return {}


def save_rarity_priorities(priorities: dict):
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(priorities, f, indent=4, ensure_ascii=False)


def get_default_priorities() -> dict:
    all_rarities = get_all_rarities_from_db()
    priorities = {}
    next_rank = len(_DEFAULT_ORDER) + 1
    for rarity in all_rarities:
        if rarity in _DEFAULT_ORDER:
            priorities[rarity] = _DEFAULT_ORDER.index(rarity) + 1
    unknown = sorted(r for r in all_rarities if r not in _DEFAULT_ORDER)
    for rarity in unknown:
        priorities[rarity] = next_rank
        next_rank += 1
    return priorities



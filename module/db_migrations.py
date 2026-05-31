"""
db_migrations.py — Migrations de schéma SQLite centralisées.

Garantit la présence de toutes les colonnes applicatives dans chaque classeur.
Appelé au démarrage (main.py) sur chaque classeur existant.

Gère deux générations de classeurs :
  - G1 (YGOJSON)      : card_uuid, card_image_uuid, card_image_url, card_image_id
  - G2 (YGOPRODeck)   : card_image_id, sort_order, rarity_code, card_type, atk…
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

import sqlite3

_REQUIRED_COLUMNS: dict[str, str] = {
    # Colonnes G1 (possession)
    "possessed":        "INTEGER DEFAULT 0",
    "quantite":         "INTEGER DEFAULT 0",
    "qualite":          "TEXT DEFAULT NULL",
    "is_custom":        "INTEGER DEFAULT 0",
    # Colonnes G2 (YGOPRODeck API)
    "sort_order":       "INTEGER DEFAULT 0",
    "rarity_code":      "TEXT DEFAULT ''",
    "card_image_small": "TEXT DEFAULT ''",
    "card_type":        "TEXT DEFAULT ''",
    "atk":              "INTEGER",
    "def_val":          "INTEGER",
    "level":            "INTEGER",
    "attribute":        "TEXT DEFAULT ''",
    "race":             "TEXT DEFAULT ''",
    # name_fr pour classeurs G1 qui ne l'auraient pas
    "name_fr":          "TEXT DEFAULT ''",
    # Édition Scanflip pour round-trip CSV (Lot 1 import/export)
    # Valeurs : '1st' / 'unlimited' / 'limited' / NULL (non spécifiée).
    # Stockée par carte physique ; permet de différencier 1st Edition vs
    # Unlimited dans l'export CSV (3 colonnes mutuellement exclusives chez
    # Scanflip).
    "edition":          "TEXT DEFAULT NULL",
    # Overframe (art étendu OCG) : 1 = print Overframe, 0 = cadre normal.
    # Traité comme un artwork distinct par tri_carte (tri + filtre N raretés).
    "extended_art":     "INTEGER DEFAULT 0",
}


def ensure_columns(conn: sqlite3.Connection, table: str = "cards") -> list[str]:
    """
    Vérifie que toutes les colonnes de _REQUIRED_COLUMNS existent dans `table`.
    Ajoute celles qui manquent via ALTER TABLE.
    Retourne la liste des colonnes ajoutées.
    """
    cursor = conn.cursor()
    cursor.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name=?", (table,)
    )
    if not cursor.fetchone():
        return []

    cursor.execute(f"PRAGMA table_info({table})")
    existing = {row[1] for row in cursor.fetchall()}

    added = []
    for col_name, col_def in _REQUIRED_COLUMNS.items():
        if col_name not in existing:
            cursor.execute(f"ALTER TABLE {table} ADD COLUMN {col_name} {col_def}")
            added.append(col_name)

    return added

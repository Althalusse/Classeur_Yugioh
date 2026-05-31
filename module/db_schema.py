"""
db_schema.py — Schéma SQLite et insertions pour cardinfo.db.

Responsabilité unique : définir les tables, les index et insérer
les données structurées issues du parsing.

Note sur les transactions
─────────────────────────
Aucune fonction de ce module ne fait de `conn.commit()` en interne.
Le commit est délégué au caller via le context manager `sqlite_ctx`
(cf. module.centralisation_dossier), ce qui garantit l'atomicité
complète de `run_init` dans BDD_creation.py : soit toutes les étapes
réussissent et commit ensemble, soit une exception déclenche un
rollback global (pas d'état partiel persistant en base).

Fonctions exportées :
  create_schema(conn)
  create_indexes(conn)
  insert_sets(conn, rows)
  insert_set_locales(conn, rows)
  insert_cards(conn, rows)
  insert_card_texts(conn, rows)
  insert_card_images(conn, rows)
  build_locale_id_index(conn)       → dict
  insert_prints(conn, rows, index)  → int
  populate_missing_fr(conn, cards_rows, images_rows) → int
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


# ─────────────────────────────────────────────────────────────────────────────
# Schéma
# ─────────────────────────────────────────────────────────────────────────────

def create_schema(conn: sqlite3.Connection) -> None:
    """
    Crée toutes les tables du schéma normalisé v2.
    Les tables volatiles sont recréées à chaque init.
    cards_overrides n'est JAMAIS supprimée.
    """
    cursor = conn.cursor()

    for table in ("set_prints", "set_locales", "sets", "card_texts",
                  "card_images", "cards", "cards_missing_fr"):
        cursor.execute(f"DROP TABLE IF EXISTS {table}")

    cursor.execute("""
        CREATE TABLE sets (
            uuid     TEXT PRIMARY KEY,
            name_en  TEXT,
            name_fr  TEXT,
            name_de  TEXT,
            name_it  TEXT,
            name_es  TEXT,
            name_ja  TEXT,
            name_ko  TEXT
        )
    """)

    cursor.execute("""
        CREATE TABLE set_locales (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            set_uuid          TEXT NOT NULL,
            language          TEXT NOT NULL,
            prefix            TEXT,
            release_date      TEXT,
            booster_image_url TEXT,
            FOREIGN KEY (set_uuid) REFERENCES sets(uuid),
            UNIQUE (set_uuid, language)
        )
    """)

    cursor.execute("""
        CREATE TABLE cards (
            uuid              TEXT PRIMARY KEY,
            ygoprodeck_id     INTEGER,
            card_type         TEXT,
            subcategory       TEXT,
            frame_type        TEXT,
            atk               INTEGER,
            def               INTEGER,
            level             INTEGER,
            attribute         TEXT,
            race              TEXT,
            banlist_tcg       TEXT,
            banlist_ocg       TEXT,
            name_fr_confirmed INTEGER DEFAULT 0
        )
    """)

    cursor.execute("""
        CREATE TABLE card_texts (
            card_uuid TEXT NOT NULL,
            language  TEXT NOT NULL,
            name      TEXT,
            effect    TEXT,
            PRIMARY KEY (card_uuid, language),
            FOREIGN KEY (card_uuid) REFERENCES cards(uuid)
        )
    """)

    cursor.execute("""
        CREATE TABLE card_images (
            uuid                TEXT PRIMARY KEY,
            card_uuid           TEXT NOT NULL,
            ygoprodeck_image_id INTEGER,
            art_url             TEXT,
            card_url            TEXT,
            FOREIGN KEY (card_uuid) REFERENCES cards(uuid)
        )
    """)

    cursor.execute("""
        CREATE TABLE set_prints (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            set_uuid        TEXT NOT NULL,
            set_locale_id   INTEGER NOT NULL,
            card_uuid       TEXT NOT NULL,
            card_image_uuid TEXT,
            set_code        TEXT,
            rarity          TEXT,
            edition         TEXT,
            qty             INTEGER DEFAULT 1,
            print_image_url TEXT,
            extended_art    INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (set_uuid)        REFERENCES sets(uuid),
            FOREIGN KEY (set_locale_id)   REFERENCES set_locales(id),
            FOREIGN KEY (card_uuid)       REFERENCES cards(uuid),
            FOREIGN KEY (card_image_uuid) REFERENCES card_images(uuid)
        )
    """)

    cursor.execute("""
        CREATE TABLE IF NOT EXISTS cards_overrides (
            override_id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            base_card_id                 INTEGER,
            name                         TEXT NOT NULL,
            card_images_id               INTEGER,
            card_images_image_url        TEXT,
            card_images_image_url_small  TEXT,
            card_sets_set_name           TEXT,
            card_sets_set_code           TEXT NOT NULL,
            card_sets_set_rarity         TEXT NOT NULL,
            card_sets_set_rarity_code    TEXT,
            reason                       TEXT,
            created_at                   TEXT DEFAULT (datetime('now')),
            UNIQUE(base_card_id, card_sets_set_code, card_sets_set_rarity)
        )
    """)

    cursor.execute("""
        CREATE TABLE cards_missing_fr (
            id        INTEGER PRIMARY KEY,
            card_uuid TEXT,
            name      TEXT NOT NULL,
            card_type TEXT,
            image_url TEXT
        )
    """)
    # Pas de commit : délégué au caller (sqlite_ctx) pour atomicité globale.


# ─────────────────────────────────────────────────────────────────────────────
# Index
# ─────────────────────────────────────────────────────────────────────────────

def create_indexes(conn: sqlite3.Connection) -> None:
    """Crée les index après les insertions en masse."""
    for stmt in [
        "CREATE INDEX IF NOT EXISTS idx_set_prints_set_locale  ON set_prints(set_locale_id)",
        "CREATE INDEX IF NOT EXISTS idx_set_prints_card_uuid   ON set_prints(card_uuid)",
        "CREATE INDEX IF NOT EXISTS idx_set_prints_set_code    ON set_prints(set_code)",
        "CREATE INDEX IF NOT EXISTS idx_set_prints_rarity      ON set_prints(rarity)",
        "CREATE INDEX IF NOT EXISTS idx_card_texts_card_lang   ON card_texts(card_uuid, language)",
        "CREATE INDEX IF NOT EXISTS idx_card_images_card_uuid  ON card_images(card_uuid)",
        "CREATE INDEX IF NOT EXISTS idx_card_images_ygo_id     ON card_images(ygoprodeck_image_id)",
        "CREATE INDEX IF NOT EXISTS idx_cards_ygoprodeck_id    ON cards(ygoprodeck_id)",
        "CREATE INDEX IF NOT EXISTS idx_set_locales_set_uuid   ON set_locales(set_uuid)",
    ]:
        conn.execute(stmt)
    # Pas de commit : délégué au caller (sqlite_ctx).


# ─────────────────────────────────────────────────────────────────────────────
# Insertions
# ─────────────────────────────────────────────────────────────────────────────

def insert_sets(conn: sqlite3.Connection, sets_rows: list) -> None:
    conn.executemany("""
        INSERT OR IGNORE INTO sets
            (uuid, name_en, name_fr, name_de, name_it, name_es, name_ja, name_ko)
        VALUES (?,?,?,?,?,?,?,?)
    """, sets_rows)
    # Pas de commit : délégué au caller (sqlite_ctx).


def insert_set_locales(conn: sqlite3.Connection, locales_rows: list) -> None:
    conn.executemany("""
        INSERT OR IGNORE INTO set_locales
            (set_uuid, language, prefix, release_date, booster_image_url)
        VALUES (?,?,?,?,?)
    """, locales_rows)
    # Pas de commit : délégué au caller (sqlite_ctx).


def build_locale_id_index(conn: sqlite3.Connection) -> dict:
    """Construit { (set_uuid, language): locale_id } depuis la table set_locales."""
    cursor = conn.execute("SELECT id, set_uuid, language FROM set_locales")
    return {(row[1], row[2]): row[0] for row in cursor.fetchall()}


def insert_prints(conn: sqlite3.Connection, prints_rows_raw: list,
                  locale_id_index: dict) -> int:
    """
    Insère les set_prints en résolvant set_locale_id depuis l'index.
    Retourne le nombre de lignes insérées.
    """
    rows    = []
    for p in prints_rows_raw:
        locale_id = locale_id_index.get((p["set_uuid"], p["locale_key"]))
        if locale_id is None:
            continue
        rows.append((
            p["set_uuid"], locale_id, p["card_uuid"], p["card_image_uuid"],
            p["set_code"], p["rarity"], p["edition"], p["qty"],
            p.get("print_image_url"),
        ))

    conn.executemany("""
        INSERT INTO set_prints
            (set_uuid, set_locale_id, card_uuid, card_image_uuid,
             set_code, rarity, edition, qty, print_image_url)
        VALUES (?,?,?,?,?,?,?,?,?)
    """, rows)
    # Pas de commit : délégué au caller (sqlite_ctx).
    return len(rows)


def insert_cards(conn: sqlite3.Connection, cards_rows: list) -> None:
    conn.executemany("""
        INSERT OR IGNORE INTO cards
            (uuid, ygoprodeck_id, card_type, subcategory, frame_type,
             atk, def, level, attribute, race,
             banlist_tcg, banlist_ocg, name_fr_confirmed)
        VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)
    """, cards_rows)
    # Pas de commit : délégué au caller (sqlite_ctx).


def insert_card_texts(conn: sqlite3.Connection, texts_rows: list) -> None:
    conn.executemany("""
        INSERT OR IGNORE INTO card_texts (card_uuid, language, name, effect)
        VALUES (?,?,?,?)
    """, texts_rows)
    # Pas de commit : délégué au caller (sqlite_ctx).


def insert_card_images(conn: sqlite3.Connection, images_rows: list) -> None:
    conn.executemany("""
        INSERT OR IGNORE INTO card_images
            (uuid, card_uuid, ygoprodeck_image_id, art_url, card_url)
        VALUES (?,?,?,?,?)
    """, images_rows)
    # Pas de commit : délégué au caller (sqlite_ctx).


def populate_missing_fr(conn: sqlite3.Connection, cards_rows: list,
                        images_rows: list) -> int:
    """
    Insère dans cards_missing_fr les cartes sans traduction FR confirmée.
    """
    img_index = {}
    for (img_uuid, card_uuid, _pw, art_url, _card_url) in images_rows:
        if card_uuid not in img_index and art_url:
            img_index[card_uuid] = art_url

    cursor    = conn.execute("SELECT card_uuid, name FROM card_texts WHERE language = 'en'")
    name_index = {row[0]: row[1] for row in cursor.fetchall()}

    cursor  = conn.execute("SELECT uuid, card_type FROM cards WHERE name_fr_confirmed = 0")
    missing = cursor.fetchall()

    rows = []
    for (card_uuid, card_type) in missing:
        name = name_index.get(card_uuid, "")
        if not name:
            continue
        rows.append((card_uuid, card_uuid, name, card_type, img_index.get(card_uuid, "")))

    conn.executemany("""
        INSERT OR IGNORE INTO cards_missing_fr
            (id, card_uuid, name, card_type, image_url)
        VALUES (
            (SELECT ygoprodeck_id FROM cards WHERE uuid = ?),
            ?, ?, ?, ?
        )
    """, rows)
    # Pas de commit : délégué au caller (sqlite_ctx).
    return len(rows)

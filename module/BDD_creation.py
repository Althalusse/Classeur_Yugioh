"""
BDD_creation.py — Orchestrateur d'initialisation de cardinfo.db.

Ce module est le seul point d'entrée connu des modules externes :
  - init_window.py                    : importe run_init
  - creation_suppression_classeur.py  : importe DB_FILE
  - controle_version_database_api.py  : appelle BDD_creation.run_init(...)

La logique métier est répartie dans :
  - module.ygojson_parser       : téléchargement + parsing YGOJSON
  - module.ygoprodeck_enricher  : téléchargement + enrichissement YGOPRODeck
  - module.db_schema            : schéma SQL + toutes les insertions
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

import json
import os
import sqlite3
from datetime import datetime

import requests

from module.centralisation_dossier import get_paths, sqlite_ctx
from module.fr_names.fr_names_service import (
    get_ygojson_remote_updated_at,
    save_fr_names_cache,
)
from module.ygojson_parser import fetch_ygojson, parse_ygojson_cards, parse_ygojson_sets
from module.ygoprodeck_enricher import (
    fetch_ygoprodeck,
    build_ygoprodeck_index,
    enrich_cards_rows,
    expand_prints_for_multi_art,
)
from module.db_schema import (
    create_schema,
    create_indexes,
    insert_sets,
    insert_set_locales,
    insert_cards,
    insert_card_texts,
    insert_card_images,
    build_locale_id_index,
    insert_prints,
    populate_missing_fr,
)
from module.logger_app import log

# ─────────────────────────────────────────────────────────────────────────────
# Chemins (DB_FILE est importé par d'autres modules)
# ─────────────────────────────────────────────────────────────────────────────
paths            = get_paths()
BDD_FOLDER       = paths["bdd"]
DB_FILE          = os.path.join(BDD_FOLDER, "cardinfo.db")
LAST_UPDATE_FILE = os.path.join(BDD_FOLDER, "last_update.txt")


# ─────────────────────────────────────────────────────────────────────────────
# Vérification de la base
# ─────────────────────────────────────────────────────────────────────────────

class DatabaseInitializationError(Exception):
    pass


def verify_database() -> bool:
    """Vérifie que la BDD existe et contient les tables essentielles peuplées."""
    if not os.path.exists(DB_FILE):
        raise DatabaseInitializationError("La base de données n'existe pas")
    try:
        with sqlite_ctx(DB_FILE) as conn:
            cursor = conn.cursor()
            for table in ("cards", "sets", "set_prints", "card_texts"):
                cursor.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                    (table,),
                )
                if not cursor.fetchone():
                    raise DatabaseInitializationError(f"Table '{table}' introuvable")
            cursor.execute("SELECT COUNT(*) FROM cards")
            if cursor.fetchone()[0] == 0:
                raise DatabaseInitializationError("La table cards est vide")
            cursor.execute("SELECT COUNT(*) FROM set_prints")
            if cursor.fetchone()[0] == 0:
                raise DatabaseInitializationError("La table set_prints est vide")
        return True
    except sqlite3.Error as e:
        raise DatabaseInitializationError(f"Erreur SQLite : {e}")



# ─────────────────────────────────────────────────────────────────────────────
# Fichier de version
# ─────────────────────────────────────────────────────────────────────────────

def save_last_update() -> bool:
    try:
        try:
            response = requests.get(
                "https://db.ygoprodeck.com/api/v7/checkDBVer.php", timeout=10
            )
            version_info = response.json()
        except Exception:
            version_info = [{
                "database_version": "unknown",
                "last_update": datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
            }]
        if isinstance(version_info, dict):
            version_info = [version_info]
        with open(LAST_UPDATE_FILE, "w", encoding="utf-8") as f:
            json.dump(version_info, f, indent=2, ensure_ascii=False)
        return True
    except Exception as e:
        log.info(f"Erreur sauvegarde version : {e}")
        return False


# ─────────────────────────────────────────────────────────────────────────────
# Orchestration principale
# ─────────────────────────────────────────────────────────────────────────────

def run_init(log) -> bool:
    """
    Orchestre l'initialisation complète de cardinfo.db.

    Flux :
      1. Téléchargement YGOJSON
      2. Téléchargement YGOPRODeck
      3. Parsing YGOJSON cards
      4. Parsing YGOJSON sets
      4.5 Résolution artworks + expansion multi-art
      5. Enrichissement stats YGOPRODeck
      6. Création schéma + insertions
      7. Index
      8. Missing FR
      9. Cache FR + fichier de version
    """
    try:
        # Y7 : le save_last_update() prématuré (avant même le DL) a été retiré.
        # Un seul appel final après verify_database() suffit, et garantit que
        # la version n'est écrite que si l'init a vraiment réussi.

        log("Téléchargement YGOJSON...", "blue")
        ygojson_cards_raw, ygojson_sets_raw = fetch_ygojson(log=log)
        if not ygojson_cards_raw:
            log("❌ YGOJSON indisponible — abandon.", "red")
            return False

        log("Téléchargement YGOPRODeck API...", "blue")
        ygoprodeck_cards = fetch_ygoprodeck(log=log)
        if not ygoprodeck_cards:
            log("⚠️ YGOPRODeck indisponible — stats (ATK/DEF/...) non disponibles.", "orange")

        log("Parsing cartes YGOJSON...", "blue")
        cards_rows, texts_rows, images_rows, confirmed_uuids = parse_ygojson_cards(ygojson_cards_raw)
        log(f"✅ {len(cards_rows)} cartes, {len(texts_rows)} textes, {len(images_rows)} images", "green")

        log("Parsing sets YGOJSON...", "blue")
        sets_rows, locales_rows, prints_rows_raw = parse_ygojson_sets(ygojson_sets_raw)
        log(f"✅ {len(sets_rows)} sets, {len(locales_rows)} locales, {len(prints_rows_raw)} prints (brut)", "green")

        if ygoprodeck_cards:
            before = len(prints_rows_raw)
            prints_rows_raw = expand_prints_for_multi_art(
                prints_rows_raw, ygoprodeck_cards, images_rows
            )
            added = len(prints_rows_raw) - before
            if added:
                log(f"✅ {added} prints supplémentaires générés (artworks alternatifs confirmés)", "green")
            else:
                log("✅ Expansion multi-artworks : aucun slot multi-art détecté", "green")

        if ygoprodeck_cards:
            log("Enrichissement stats YGOPRODeck (ATK, DEF, level, ...)...", "blue")
            ygoprodeck_index = build_ygoprodeck_index(ygoprodeck_cards)
            cards_rows       = enrich_cards_rows(cards_rows, ygoprodeck_index)
            enriched = sum(1 for r in cards_rows if r[4] is not None)
            log(f"✅ {enriched} cartes enrichies avec stats YGOPRODeck", "green")

        log("Création du schéma SQL...", "blue")
        with sqlite_ctx(DB_FILE) as conn:
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
            # Transaction explicite obligatoire :
            # sans BEGIN IMMEDIATE, Python sqlite3 fait un commit implicite
            # avant chaque DDL (DROP TABLE, CREATE TABLE), ce qui casserait
            # l'atomicité du rollback en cas d'erreur. Cf. doc Python :
            # https://docs.python.org/3/library/sqlite3.html#controlling-transactions
            conn.execute("BEGIN IMMEDIATE")
            create_schema(conn)
            log("Insertion sets...", "blue")
            insert_sets(conn, sets_rows)
            log("Insertion locales...", "blue")
            insert_set_locales(conn, locales_rows)
            log("Insertion cartes...", "blue")
            insert_cards(conn, cards_rows)
            log("Insertion textes cartes...", "blue")
            insert_card_texts(conn, texts_rows)
            log("Insertion images cartes...", "blue")
            insert_card_images(conn, images_rows)
            log("Résolution locale_id et insertion set_prints...", "blue")
            locale_id_index = build_locale_id_index(conn)
            nb_prints = insert_prints(conn, prints_rows_raw, locale_id_index)
            log(f"✅ {nb_prints} set_prints insérés", "green")
            log("Création des index...", "blue")
            create_indexes(conn)
            log("Détection cartes sans traduction FR...", "blue")
            nb_missing = populate_missing_fr(conn, cards_rows, images_rows)
            log(f"✅ {nb_missing} cartes sans traduction FR isolées", "green")

        log("Mise à jour cache FR...", "blue")
        uuid_to_password = {r[0]: r[1] for r in cards_rows if r[1] is not None}
        fr_names_cache   = {}
        confirmed_ids    = set()
        for (card_uuid, lang, name, _effect) in texts_rows:
            if lang == "fr" and name:
                pw = uuid_to_password.get(card_uuid)
                if pw:
                    fr_names_cache[str(pw)] = name
                    if card_uuid in confirmed_uuids:
                        confirmed_ids.add(pw)

        remote_updated_at = get_ygojson_remote_updated_at()
        save_fr_names_cache(fr_names_cache, confirmed_ids, remote_updated_at)
        log(f"✅ Cache FR mis à jour ({len(fr_names_cache)} noms)", "green")

        verify_database()
        save_last_update()
        log("✅ Base de données créée et vérifiée avec succès.", "green")
        return True

    except Exception as e:
        log(f"❌ Erreur inattendue : {e}", "red")
        import traceback
        log(traceback.format_exc(), "red")
        return False

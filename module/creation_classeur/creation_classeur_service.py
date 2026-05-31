"""
creation_classeur_service.py — Création de classeurs via YGOPRODeck API.

Implémente le flux exact du §4 de la spécification technique :
  1. GET /cardsets.php           → liste de tous les sets disponibles
  2. GET /cardinfo.php?cardset=  → cartes EN  ┐ en parallèle
     GET /cardinfo.php?cardset=&language=fr   ┘
  3. Map FR : { card_id → nom_français }
  4. Pour chaque carte EN × chaque rareté du set → 1 ligne cards
  5. Tri (sort_order, rarity), renumérotation séquentielle, insertion SQLite

Avantage vs YGOJSON : YGOPRODeck est mis à jour en continu et contient
TOUTES les raretés de TOUS les sets (ex: RA05 en 6 raretés par numéro).
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
import shutil
import stat
import requests
from concurrent.futures import ThreadPoolExecutor

from module.centralisation_dossier import CLASSEUR_FOLDER, sqlite_ctx
from module.gestion_rarete.correction_rarete import corriger_rows
from module.logger_app import log

_BASE     = "https://db.ygoprodeck.com/api/v7"
_SETS_URL = f"{_BASE}/cardsets.php"
_INFO_URL = f"{_BASE}/cardinfo.php"
_HEADERS  = {"User-Agent": "YugiohCollectionManager/1.0"}


# ─────────────────────────────────────────────────────────────────────────────
# Helper table meta (Y8)
# ─────────────────────────────────────────────────────────────────────────────

def _ensure_meta_table(conn) -> None:
    """
    Crée la table `meta` du classeur si elle n'existe pas.

    Centralisé ici pour éviter la duplication du DDL à 4 endroits (create_classeur,
    get_classeur_meta, get_classeur_config, save_classeur_config).

    La table stocke les paramètres du classeur sous forme clé/valeur textuelle
    (actuellement : colonnes, lignes ; potentiellement plus tard : date de
    dernière ouverture, etc.).
    """
    conn.execute("""
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        )
    """)


# ─────────────────────────────────────────────────────────────────────────────
# Helpers API
# ─────────────────────────────────────────────────────────────────────────────

def _fetch_cardsets() -> list:
    """GET /cardsets.php — liste complète des sets YGOPRODeck."""
    resp = requests.get(_SETS_URL, headers=_HEADERS, timeout=30)
    resp.raise_for_status()
    return resp.json()


def _fetch_set_cards(set_name: str, language: str | None = None) -> list:
    """GET /cardinfo.php?cardset=... (langue optionnelle)."""
    params: dict = {"cardset": set_name}
    if language:
        params["language"] = language
    try:
        resp = requests.get(_INFO_URL, params=params, headers=_HEADERS, timeout=60)
        if resp.status_code == 200:
            return resp.json().get("data", [])
    except Exception as e:
        log.warning(f"_fetch_set_cards({set_name!r}, lang={language!r}): {e}")
    return []


def _find_set_name(code_set: str, all_sets: list) -> str | None:
    """Retourne le set_name complet correspondant au code_set."""
    code_upper = code_set.upper()
    for s in all_sets:
        if s.get("set_code", "").upper() == code_upper:
            return s["set_name"]
    return None


def _sort_order_from_code(full_set_code: str) -> int:
    """
    Extrait le numéro ordinal depuis la fin du code complet.
    "SS01-ENA01" -> 1   "RA05-EN035" -> 35   "LOB-EN001" -> 1

    Conservé pour la valeur initiale de la colonne `sort_order` (qui
    sera de toute façon écrasée par le rank après le sort). Le tri
    réel se fait via `_sort_key_from_code` qui prend en compte le
    groupe de lettres pour les sets multi-decks.
    """
    try:
        suffix = full_set_code.rsplit("-", 1)[-1]
        m = re.search(r"\d+$", suffix)
        return int(m.group()) if m else 0
    except Exception:
        return 0


def _sort_key_from_code(full_set_code: str) -> tuple:
    """
    Clé de tri (letter_group, number) pour grouper les cartes par
    sous-deck dans les sets multi-decks (ex L26D : ENM01-99 / ENS01-99
    / ENX01-99 → 3 decks dans un classeur unique).

    Doit rester ALIGNÉ avec `tri_carte._extract_numero` (même algorithme),
    car `tri_carte.sort_cartes()` re-trie les cartes au moment de
    l'affichage selon les préférences utilisateur. Le tri à la création
    sert d'état initial cohérent en DB (visible via SQL direct).

    Exemples :
      "L26D-ENM01" → ("M",  1)
      "L26D-ENS03" → ("S",  3)
      "RA05-EN035" → ("",  35)
      "LOB-EN001"  → ("",   1)
    """
    try:
        suffix = full_set_code.rsplit("-", 1)[-1]
        # Codes langue Konami sur 2 lettres : EN, FR, DE, IT, JP, KR, etc.
        m_lang = re.match(r"^[A-Z]{2}", suffix)
        after_lang = suffix[m_lang.end():] if m_lang else suffix
        m = re.match(r"^([A-Z]*)(\d+)$", after_lang)
        if m:
            return (m.group(1), int(m.group(2)))
        return ("", 0)
    except Exception:
        return ("", 0)


# ─────────────────────────────────────────────────────────────────────────────
# Création du classeur
# ─────────────────────────────────────────────────────────────────────────────

def _fetch_rows_from_api(code_set: str) -> list[dict]:
    """
    Récupère les lignes de cartes depuis YGOPRODeck pour un set donné,
    SANS écrire sur disque.

    Extraction de la logique de construction de rows depuis
    `_create_classeur_from_api`, pour permettre à `create_classeur()`
    de comparer le nombre de cartes API vs local avant de choisir
    quelle source utiliser (fix LDK2 : API retourne 130 cartes, local
    en a 132 → on préfère le local plus complet).

    Les rows retournés ont `card_uuid = ""` et `card_image_uuid = ""`
    (non disponibles via l'API YGOPRODeck) pour être compatibles avec
    `_save_classeur_from_rows_local`.

    Lève ValueError si le set est introuvable ou ne contient aucune carte.
    """
    # Étape 1 : trouver le nom complet du set
    try:
        all_sets = _fetch_cardsets()
    except Exception as e:
        raise ValueError(f"Impossible de récupérer la liste des sets : {e}")

    set_name = _find_set_name(code_set, all_sets)
    if not set_name:
        raise ValueError(
            f"Code set '{code_set}' introuvable dans YGOPRODeck.\n"
            f"Vérifiez que le code est correct (ex: LOB, RA05, SS01...)."
        )

    # Étape 2 : requêtes EN + FR en parallèle
    with ThreadPoolExecutor(max_workers=2) as pool:
        fut_en = pool.submit(_fetch_set_cards, set_name)
        fut_fr = pool.submit(_fetch_set_cards, set_name, "fr")
        cards_en = fut_en.result()
        cards_fr = fut_fr.result()

    if not cards_en:
        raise ValueError(
            f"Aucune carte trouvée pour le set '{code_set}' ({set_name})."
        )

    # Étape 3 : map FR { card_id -> nom_fr }
    fr_map: dict[int, str] = {}
    for card in cards_fr:
        cid = card.get("id")
        nfr = card.get("name", "")
        if cid and nfr:
            fr_map[cid] = nfr

    # Étape 4 : 1 ligne par carte × rareté du set
    rows: list[dict] = []
    for card in cards_en:
        card_id   = card.get("id")
        card_name = card.get("name", "")
        images          = card.get("card_images", [])
        image_url       = images[0].get("image_url",       "") if images else ""
        image_url_small = images[0].get("image_url_small", "") if images else ""

        for cs in card.get("card_sets", []):
            if cs.get("set_name") != set_name:
                continue
            full_code   = cs.get("set_code", "")
            rarity      = cs.get("set_rarity", "")
            rarity_code = cs.get("set_rarity_code", "").strip("()")
            rows.append({
                # card_uuid / card_image_uuid non disponibles via l'API ;
                # chaînes vides pour compatibilité avec _save_classeur_from_rows_local.
                "card_uuid":        "",
                "card_image_uuid":  "",
                "card_image_id":    card_id,
                "name":             card_name,
                "name_fr":          fr_map.get(card_id, ""),
                "set_code":         full_code,
                "rarity":           rarity,
                "rarity_code":      rarity_code,
                "set_name":         set_name,
                "card_image_url":   image_url,
                "card_image_small": image_url_small,
                "sort_order":       _sort_order_from_code(full_code),
                "card_type":  card.get("type",      ""),
                "atk":        card.get("atk"),
                "def_val":    card.get("def"),
                "level":      card.get("level"),
                "attribute":  card.get("attribute", ""),
                "race":       card.get("race",      ""),
                "extended_art": 0,
            })

    if not rows:
        raise ValueError(
            f"Aucune carte trouvée dans le set '{code_set}' — "
            f"vérifiez que le code correspond à un set EN."
        )

    # Étape 5 : trier (letter_group, number, rarity) puis renuméroter
    # Le tri (letter_group, number) groupe par sous-deck dans les sets
    # multi-decks (L26D, etc.) ; identique à l'ancien tri par numéro seul
    # pour les sets classiques (letter_group = "" partout).
    rows.sort(key=lambda r: (
        _sort_key_from_code(r["set_code"]),
        1 if r.get("extended_art") else 0,   # Overframe = bloc distinct
        r.get("rarity", ""),
    ))
    for i, row in enumerate(rows):
        row["sort_order"] = i

    return rows


def _create_classeur_from_api(code_set: str) -> bool:
    """
    Crée un classeur SQLite via les API YGOPRODeck (fallback réseau).

    Cette fonction est désormais réservée au fallback : la voie principale
    est `_create_classeur_from_local()` (cardinfo.db). On bascule ici
    uniquement si cardinfo.db ne connaît pas le set demandé (ex : set
    sorti après la dernière MAJ BDD locale).

    Le caller normal est `create_classeur()` (l'orchestrateur public)
    qui essaie d'abord la voie locale puis tombe ici en cas de
    SetNotInLocalDB.

    Retourne True si créé, False si déjà existant.
    Lève ValueError si set introuvable côté API ou sans cartes.
    """
    code_set = str(code_set).strip().upper()
    classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)
    if os.path.exists(classeur_path):
        return False

    rows = _fetch_rows_from_api(code_set)
    return _save_classeur_from_rows_local(code_set, rows)


# ─────────────────────────────────────────────────────────────────────────────
# Utilitaires
# ─────────────────────────────────────────────────────────────────────────────

def remove_readonly(func, path, _):
    os.chmod(path, stat.S_IWRITE)
    func(path)


def get_set_title(code_set: str, use_fr: bool = False) -> str:
    """
    Retourne le nom complet du set depuis le classeur (colonne set_name).
    Fallback sur cardinfo.db (classeurs créés avant migration).
    """
    code_set = code_set.strip().upper()
    db_path  = os.path.join(CLASSEUR_FOLDER, code_set, f"{code_set}.db")
    if os.path.exists(db_path):
        try:
            with sqlite_ctx(db_path) as conn:
                cursor = conn.cursor()
                cursor.execute(
                    "SELECT set_name FROM cards "
                    "WHERE set_name IS NOT NULL AND set_name != '' LIMIT 1"
                )
                row = cursor.fetchone()
                if row:
                    return row[0]
        except Exception:
            pass

    try:
        from module.centralisation_dossier import CARDINFO_DB
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            cursor.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='sets'"
            )
            if cursor.fetchone():
                # Deux cas selon la nature du classeur :
                #   1. TCG (`CROS`)        → set_locales.prefix = `CROS-EN`,
                #      `CROS-EU`, etc.   → match via LIKE 'CROS-%'.
                #   2. OCG-JP (`CROS-JP`) → set_locales.prefix = `CROS-JP`
                #      (préfixe complet, sans suffixe supplémentaire) →
                #      match via égalité exacte `prefix = 'CROS-JP'`.
                # Le OR couvre les deux cas en une seule requête.
                cursor.execute("""
                    SELECT s.name_fr, s.name_en
                    FROM sets s
                    JOIN set_locales sl ON sl.set_uuid = s.uuid
                    WHERE sl.prefix LIKE ?
                       OR UPPER(sl.prefix) = ?
                    LIMIT 1
                """, (f"{code_set}-%", code_set))
                row = cursor.fetchone()
                if row:
                    return (row[0] or row[1]) if use_fr else (row[1] or row[0] or "")
    except Exception:
        pass
    return ""


def get_classeur_config(db_path: str) -> tuple[int, int]:
    """Lit colonnes/lignes depuis meta. Retourne la grille par défaut des
    préférences utilisateur si la meta est absente (au lieu du (3,3) en dur).
    """
    from module.config.preferences import get_grille_defaut
    default_cols, default_rows = get_grille_defaut()

    try:
        with sqlite_ctx(db_path) as conn:
            _ensure_meta_table(conn)  # Y8 : helper centralisé
            cursor = conn.cursor()
            cursor.execute("SELECT value FROM meta WHERE key='colonnes'")
            row_cols = cursor.fetchone()
            cursor.execute("SELECT value FROM meta WHERE key='lignes'")
            row_ligs = cursor.fetchone()
        return (
            int(row_cols[0]) if row_cols else default_cols,
            int(row_ligs[0]) if row_ligs else default_rows,
        )
    except Exception as e:
        log.warning(f"get_classeur_config : {e}")
        return default_cols, default_rows


def save_classeur_config(db_path: str, colonnes: int, lignes: int) -> None:
    """Sauvegarde colonnes/lignes dans meta."""
    try:
        with sqlite_ctx(db_path) as conn:
            _ensure_meta_table(conn)  # Y8 : helper centralisé
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('colonnes', ?)",
                (str(colonnes),)
            )
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('lignes', ?)",
                (str(lignes),)
            )
    except Exception as e:
        log.warning(f"save_classeur_config : {e}")


# ─────────────────────────────────────────────────────────────────────────────
# Override par classeur du nombre de raretés affichées (filtre visuel)
# ─────────────────────────────────────────────────────────────────────────────
#
# Permet à l'utilisateur de définir un N spécifique pour un classeur donné,
# qui prime sur la valeur globale `affichage_n_raretes_par_artwork` de
# preferences.py. Stocké dans la table meta sous la clé `n_raretes_par_artwork`.
#
# Sémantique :
#   - get retourne None  → pas d'override → utiliser la valeur globale
#   - get retourne 0     → override explicite "toutes les raretés"
#   - get retourne N≥1   → override explicite "N plus rares"
#   - save(code, None)   → supprime l'override → re-tombe sur le global
#   - save(code, N)      → enregistre N comme override
#
# Le filtre lui-même est appliqué dans ecran_classeur lors du rendu.
#

_META_KEY_N_RARETES = "n_raretes_par_artwork"


def get_n_raretes_override(code: str) -> int | None:
    """Retourne l'override N pour ce classeur, ou None s'il n'y en a pas.

    None signifie "utiliser la valeur globale". 0 et plus sont des overrides
    explicites (0 = afficher toutes les raretés, ignorant le global).
    """
    db_path = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    if not os.path.exists(db_path):
        return None
    try:
        with sqlite_ctx(db_path) as conn:
            _ensure_meta_table(conn)
            cursor = conn.cursor()
            cursor.execute(
                "SELECT value FROM meta WHERE key=?", (_META_KEY_N_RARETES,)
            )
            row = cursor.fetchone()
            if row is None:
                return None
            try:
                return int(row[0])
            except (TypeError, ValueError):
                return None
    except Exception as e:
        log.warning(f"get_n_raretes_override({code}): {e}")
        return None


def save_n_raretes_override(code: str, n: int | None) -> None:
    """Enregistre ou supprime l'override N pour ce classeur.

    n=None → supprime l'override (le classeur retombe sur la valeur globale).
    n=0+   → enregistre l'override (clamp côté UI, pas ici).
    """
    db_path = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    if not os.path.exists(db_path):
        log.warning(f"save_n_raretes_override : classeur {code} introuvable")
        return
    try:
        with sqlite_ctx(db_path) as conn:
            _ensure_meta_table(conn)
            if n is None:
                conn.execute(
                    "DELETE FROM meta WHERE key=?", (_META_KEY_N_RARETES,)
                )
            else:
                conn.execute(
                    "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
                    (_META_KEY_N_RARETES, str(int(n))),
                )
    except Exception as e:
        log.warning(f"save_n_raretes_override({code}): {e}")


def _fetch_sets_from_cardinfo_db(use_fr: bool = False) -> list[dict] | None:
    """Retourne la liste des sets depuis cardinfo.db locale.

    Format compatible avec _fetch_cardsets() :
        [{"set_code": "RA05", "set_name": "...", "num_of_cards": 692}, ...]

    Une seule requête agrégée — pas de N+1 — qui joint sets + set_locales
    + set_prints pour calculer le nombre de cartes distinctes par préfixe.

    Filtres appliqués :
      - Locales : déterminées par préférences.get_langues_locales_actives() :
        toujours ('en', 'eu') ; ('en', 'eu', 'ja') si l'utilisateur a coché
        "Inclure les sets japonais (OCG)".
      - Préfixes non nuls uniquement.

    Format des set_codes retournés :
      - Sets TCG (locales 'en'/'eu')  : préfixe NU agrégé ("CROS", "RA05").
        Les locales 'en' et 'eu' d'un même set sont fusionnées dans une
        seule entrée (évite les doublons RA02-EN / RA02-EU côté UI).
      - Sets OCG-JP (locale 'ja') si activé : préfixe COMPLET avec suffixe
        régional ("CROS-JP", "LOCH-JP"). Cela permet :
          * de coexister avec un classeur TCG du même set ("CROS" et
            "CROS-JP" deviennent deux entrées indépendantes)
          * d'identifier visuellement la version japonaise dans la liste
          * de servir directement de nom de dossier classeur sur disque

    Retourne None si :
      - cardinfo.db absent ou corrompu
      - Tables set_prints / set_locales absentes (BDD non initialisée)
      - Résultat vide

    Le caller doit alors basculer sur _fetch_cardsets() (API).
    """
    try:
        from module.centralisation_dossier import CARDINFO_DB
        from module.config import preferences as _prefs

        if not os.path.exists(CARDINFO_DB):
            return None

        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()

            # Vérifier que les tables requises existent pour éviter une
            # erreur SQL cryptique ("no such table").
            for tbl in ("sets", "set_locales", "set_prints"):
                cursor.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                    (tbl,),
                )
                if not cursor.fetchone():
                    return None

            # ── Construction dynamique du filtre langue ────────────────
            # ('en','eu') minimum, +('ja',) si OCG-JP activé.
            langues = _prefs.get_langues_locales_actives()
            ph_lang = ",".join("?" * len(langues))

            # ── Requête TCG : agrégat sur préfixe nu ('en'/'eu' uniquement)
            # On garde le strip SUBSTR + INSTR pour fusionner les locales
            # multiples d'un même set TCG (RA02-EN + RA02-EU → RA02).
            cursor.execute(f"""
                SELECT
                    SUBSTR(sl.prefix, 1,
                           INSTR(sl.prefix || '-', '-') - 1)  AS set_code,
                    s.name_en                                  AS name_en,
                    s.name_fr                                  AS name_fr,
                    COUNT(DISTINCT sp.card_uuid)               AS nb_cartes
                FROM sets s
                JOIN set_locales sl ON sl.set_uuid = s.uuid
                LEFT JOIN set_prints sp ON sp.set_locale_id = sl.id
                WHERE sl.prefix IS NOT NULL
                  AND sl.prefix != ''
                  AND sl.language IN ('en', 'eu')
                GROUP BY SUBSTR(sl.prefix, 1,
                                INSTR(sl.prefix || '-', '-') - 1),
                         s.name_en, s.name_fr
                HAVING set_code != ''
                ORDER BY set_code
            """)
            rows_tcg = cursor.fetchall()

            # ── Requête OCG-JP : préfixe complet (sans strip) ───────────
            # Ne tourne que si 'ja' est dans le filtre actif. On NE strip
            # PAS le suffixe pour pouvoir distinguer LOCH-JP de LOCH (TCG)
            # dans le filesystem et dans la liste.
            rows_ocg = []
            if "jp" in langues:
                cursor.execute("""
                    SELECT
                        UPPER(sl.prefix)                       AS set_code,
                        s.name_en                              AS name_en,
                        s.name_fr                              AS name_fr,
                        COUNT(DISTINCT sp.card_uuid)           AS nb_cartes
                    FROM sets s
                    JOIN set_locales sl ON sl.set_uuid = s.uuid
                    LEFT JOIN set_prints sp ON sp.set_locale_id = sl.id
                    WHERE sl.prefix IS NOT NULL
                      AND sl.prefix != ''
                      AND sl.language = 'jp'
                    GROUP BY UPPER(sl.prefix), s.name_en, s.name_fr
                    HAVING set_code != ''
                    ORDER BY set_code
                """)
                rows_ocg = cursor.fetchall()

        all_rows = list(rows_tcg) + list(rows_ocg)
        if not all_rows:
            return None

        result = []
        for set_code, name_en, name_fr, nb in all_rows:
            set_name = (name_fr or name_en) if use_fr else (name_en or name_fr or "")
            result.append({
                "set_code":     set_code,
                "set_name":     set_name,
                "num_of_cards": nb or 0,
            })
        # Tri final : la concaténation TCG+OCG peut produire un ordre
        # non monotone (CROS, ..., LOCH, CROS-JP, LOCH-JP) — on retrie.
        result.sort(key=lambda r: r["set_code"])
        return result

    except Exception as e:
        log.warning(f"_fetch_sets_from_cardinfo_db : {e}")
        return None


def get_available_set_codes(use_fr: bool = False,
                            force_api: bool = False) -> list[tuple[str, str]]:
    """
    Retourne les sets disponibles qui n'ont pas encore de classeur créé sur disque.
    Format : [(code, "CODE (Nom complet) [nb_cartes]"), ...]

    Source de données (par ordre de priorité) :
      1. cardinfo.db locale — instantané, marche offline
      2. API YGOPRODeck /cardsets.php — fallback si BDD absente/incomplète,
         OU si force_api=True (bouton "↻ Sync" de l'UI)

    force_api=True force un appel réseau même si la BDD locale est disponible.
    Utile quand l'utilisateur veut refresh après un nouveau set paru récemment.
    """
    deja_crees = {
        d.upper()
        for d in os.listdir(CLASSEUR_FOLDER)
        if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))
    }

    # 1) Tentative lecture BDD locale (sauf si force_api)
    all_sets: list[dict] | None = None
    if not force_api:
        all_sets = _fetch_sets_from_cardinfo_db(use_fr=use_fr)

    # 2) Fallback API si BDD indisponible ou force_api
    if all_sets is None:
        try:
            all_sets = _fetch_cardsets()
        except Exception as e:
            log.warning(f"get_available_set_codes : {e}")
            return []

    result = []
    for s in all_sets:
        code  = (s.get("set_code") or "").strip().upper()
        name  = s.get("set_name", "")
        count = s.get("num_of_cards", 0)
        if not code or code in deja_crees:
            continue
        label = f"{code} ({name}) [{count}]"
        result.append((code, label))

    return sorted(result, key=lambda x: x[0])


# ─────────────────────────────────────────────────────────────────────────────
# Création du classeur
# ─────────────────────────────────────────────────────────────────────────────


class SetNotInLocalDB(Exception):
    """
    Levée lorsque le set demandé n'existe pas dans cardinfo.db.

    Permet au caller de distinguer "set inconnu localement" (peut être
    un set récent dont MAJ BDD n'a pas encore tiré les données) d'une
    erreur générique. L'UI peut alors orienter l'utilisateur vers MAJ BDD.
    """
    pass


def _build_rows_from_local_db(code_set: str) -> list[dict]:
    """
    Construit la liste de lignes à insérer dans le classeur, depuis
    cardinfo.db uniquement (zéro appel API).

    Algorithme (révisé après diagnostic RA05 de mai 2026) :
      1. Trouve le `set_uuid` correspondant au préfixe `code_set`
      2. Choisit la locale d'énumération : FR uniquement si elle est AU
         MOINS AUSSI COMPLÈTE qu'EN (count FR ≥ count EN), sinon EN.
         Voir le commentaire détaillé dans le code pour la justification.
      3. Énumère les `set_prints` de la locale choisie
      4. Si on énumère depuis FR, joint la locale EN (par card_uuid +
         image_uuid + rarity, qui est unique 1-pour-1) pour récupérer le
         set_code EN
      5. Joint cards + card_images + card_texts EN/FR pour les métadonnées

    Garantie : le nombre de lignes correspond à la réalité physique du set.
    Vérifié sur LDK2 (132 vs 172 avant fix avril 2026), EGO1, EGS1, SDLI,
    SDWD, VASM, RA02 (7 raretés × N), RA05 (7 raretés × N après fix mai 2026).

    Limite connue (sets EN-only) : si un set n'a pas de locale FR,
    on retombe sur la locale EN avec ses doublons d'artworks historiques.
    Concerne uniquement les sets exclusifs au TCG anglophone (rare).

    Lève SetNotInLocalDB si le set n'existe pas dans cardinfo.db.
    """
    from module.centralisation_dossier import CARDINFO_DB
    if not os.path.exists(CARDINFO_DB):
        raise SetNotInLocalDB(
            f"cardinfo.db introuvable. Lancez 'MAJ BDD' depuis l'écran principal."
        )

    rows: list[dict] = []
    with sqlite_ctx(CARDINFO_DB) as conn:
        cursor = conn.cursor()

        # Vérification rapide des tables nécessaires
        for tbl in ("sets", "set_locales", "set_prints", "cards",
                    "card_images", "card_texts"):
            cursor.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                (tbl,),
            )
            if not cursor.fetchone():
                raise SetNotInLocalDB(
                    f"Table '{tbl}' absente dans cardinfo.db. "
                    f"Lancez 'MAJ BDD' pour reconstruire la base."
                )

        # Hook Overframe (B) : complète les prints "art étendu" via Yugipedia
        # AVANT l'énumération de set_prints. No-op instantané (zéro requête)
        # pour un set non-Overframe ; idempotent (revid) pour les autres.
        # Jamais bloquant : toute erreur est capturée et loggée par l'enricher.
        try:
            from module.donnees.overframe_enrichment import enrich_for_classeur
            enrich_for_classeur(conn, code_set)
        except Exception as _e_ofe:
            log.warning(f"enrich_for_classeur({code_set}) ignoré : {_e_ofe}")

        # 1) Identifier le set via le préfixe — toutes locales acceptées
        #    pour la résolution du set_uuid (un préfixe quelconque suffit).
        #
        # Pour les classeurs OCG-JP (`LOCH-JP`, `CROS-JP`, etc.), le préfixe
        # passé est COMPLET (avec suffixe régional) : la condition
        # `sl.prefix = ?` matchera directement sur la locale 'ja'.
        # Pour les classeurs TCG (`CROS`), le `LIKE 'CROS-%'` matchera
        # `CROS-EN`, `CROS-EU`, etc.
        cursor.execute("""
            SELECT DISTINCT s.uuid, s.name_en
            FROM sets s
            JOIN set_locales sl ON sl.set_uuid = s.uuid
            WHERE sl.prefix = ? OR sl.prefix LIKE ? || '-%'
            LIMIT 1
        """, (code_set, code_set))
        row = cursor.fetchone()
        if not row:
            raise SetNotInLocalDB(
                f"Set '{code_set}' absent de cardinfo.db. "
                f"Lancez 'MAJ BDD' si c'est un set récent, "
                f"ou vérifiez l'orthographe du code."
            )
        set_uuid, set_name_en = row

        # 2) Choisir la locale d'énumération.
        #
        # ── Cas OCG-JP (préfixe avec suffixe régional `-JP`/`-JA`/etc.) ──
        # Pour un classeur OCG-JP, on force la locale 'ja' SANS fallback
        # vers FR/EN : un set OCG-JP doit être énuméré avec ses set_codes
        # japonais (`LOCH-JP001`, `CROS-JP001`). Mélanger des prints FR
        # ou EN dans un classeur "japonais" produirait des set_codes
        # incohérents avec le contenu attendu par le collectionneur.
        from module.config.preferences import a_suffixe_ocg as _is_ocg

        if _is_ocg(code_set):
            cursor.execute("""
                SELECT id FROM set_locales
                WHERE set_uuid = ? AND language = 'jp'
                LIMIT 1
            """, (set_uuid,))
            r_ja = cursor.fetchone()
            if not r_ja:
                raise SetNotInLocalDB(
                    f"Set OCG '{code_set}' présent mais aucune locale 'ja' "
                    f"avec des données. Lancez 'MAJ BDD' pour mettre à jour la base."
                )
            locale_enum_id = r_ja[0]
            locale_en_id   = None  # pas de cross-jointure EN en mode OCG
            mode_enum      = 'jp'

        else:
            # ── Cas TCG (préfixe nu) — comportement historique ──────────
            #    On récupère les ids des deux locales en une seule requête.
            cursor.execute("""
                SELECT language, id FROM set_locales
                WHERE set_uuid = ? AND language IN ('fr', 'en')
            """, (set_uuid,))
            locale_ids: dict[str, int] = {lang: lid for lang, lid in cursor.fetchall()}

            # Choix de la locale d'énumération : on compare le NOMBRE de prints
            # entre FR et EN, pas seulement leur existence.
            #
            # Pourquoi ne pas se contenter de "FR non vide" :
            # ─────────────────────────────────────────────
            # Cas réel rencontré (RA05, avril 2026) : YGOJSON peut avoir importé
            # PARTIELLEMENT la locale FR d'un set récent (ex: 1 rareté sur 7
            # disponibles, parce que Yugipedia n'a pas encore publié toutes les
            # variantes FR). Une condition "FR non vide" prendrait alors la FR
            # comme source d'énumération et le classeur perdrait toutes les
            # raretés non encore traduites — exactement le bug observé sur RA05
            # (1 rareté créée au lieu de 7) tandis que RA02 (set ancien à FR
            # complète) fonctionnait correctement.
            #
            # Règle robuste : FR n'est utilisée que si elle est AU MOINS AUSSI
            # complète qu'EN. Sinon on bascule sur EN.
            #
            # Cas couverts :
            #   - Sets bien établis (FR == EN ou FR > EN) → énumération FR
            #     (préserve la propriété "FR évite les doublons d'artworks
            #     historiques sur les cartes-icônes")
            #   - Sets récents type RA05 (0 < FR < EN)    → énumération EN ✅ FIX
            #   - Sets EN-only (FR == 0)                  → énumération EN
            #   - Sets FR-only théoriques (EN == 0)       → énumération FR
            #   - Aucune des deux                         → SetNotInLocalDB
            #
            # Note : passer en mode EN ne fait PAS perdre les noms FR ; la
            # requête EN ci-dessous joint déjà card_texts (lang='fr') pour les
            # récupérer carte par carte indépendamment de la locale d'énumération.
            def _count_prints(loc_id: int) -> int:
                cursor.execute(
                    "SELECT COUNT(*) FROM set_prints WHERE set_locale_id = ?",
                    (loc_id,),
                )
                row = cursor.fetchone()
                return row[0] if row else 0

            fr_count = _count_prints(locale_ids['fr']) if 'fr' in locale_ids else 0
            en_count = _count_prints(locale_ids['en']) if 'en' in locale_ids else 0

            locale_enum_id = None
            locale_en_id   = locale_ids.get('en')  # peut être None pour sets FR-only
            mode_enum      = None

            if fr_count > 0 and fr_count >= en_count:
                locale_enum_id = locale_ids['fr']
                mode_enum      = 'fr'
            elif en_count > 0:
                locale_enum_id = locale_ids['en']
                locale_en_id   = locale_ids['en']
                mode_enum      = 'en'
            else:
                raise SetNotInLocalDB(
                    f"Set '{code_set}' présent mais aucune locale 'fr' ni 'en' "
                    f"avec des données. Lancez 'MAJ BDD' pour mettre à jour la base."
                )

        # 3) Requête principale — énumérer depuis la locale choisie
        #    + jointure 1-pour-1 sur EN pour le set_code EN si on énumère FR.
        if mode_enum == 'fr' and locale_en_id is not None:
            # Énumération FR avec récupération du set_code EN.
            #
            # Bug fix mai 2026 — SDWD :
            # ─────────────────────────
            # Le LEFT JOIN sp_en exigeait initialement
            # `sp_en.rarity = sp_fr.rarity`. Cette condition était trop
            # stricte : si une carte avait en FR la rareté "Common" mais
            # en EN "Short Print" (ou variante) — situation rencontrée sur
            # plusieurs reprints de Structure Deck dans cardinfo.db — le
            # JOIN échouait, le COALESCE retombait sur `sp_fr.set_code`,
            # et la BDD du classeur se retrouvait avec des set_codes "FR"
            # (ex SDWD-FR013) au lieu d'EN.
            #
            # Conséquence : l'import CSV — qui convertit toujours
            # FR→EN — ne retrouvait pas ces cartes. Bug observé sur SDWD
            # (6 cartes Common manquées).
            #
            # Pour une carte physique donnée (card_uuid + card_image_uuid)
            # dans un set, le set_code est invariant par rareté
            # — l'identifiant numérique de la carte ne change pas selon
            # qu'elle est imprimée en Common ou en Secret Rare. La
            # contrainte de rareté n'apporte donc aucune information
            # supplémentaire mais empêche le rattrapage en cas de
            # divergences mineures de nommage de rareté entre langues.
            #
            # Solution : on retire `AND sp_en.rarity = sp_fr.rarity`,
            # puis on déduplique avec MIN(set_code) pour garantir une
            # seule ligne EN par couple (card_uuid, card_image_uuid)
            # — au cas tordu où plusieurs set_codes EN coexisteraient,
            # on prend le plus petit lexicographiquement (déterministe).
            cursor.execute("""
                SELECT
                    COALESCE(sp_en.set_code_min, sp_fr.set_code) AS set_code_en,
                    sp_fr.rarity                                 AS rarity,
                    ci.ygoprodeck_image_id                       AS card_image_id,
                    ci.card_url                                  AS card_image_url,
                    ci.art_url                                   AS card_image_small,
                    c.uuid                                       AS card_uuid,
                    ci.uuid                                      AS card_image_uuid,
                    c.card_type                                  AS card_type,
                    c.atk                                        AS atk,
                    c.def                                        AS def_val,
                    c.level                                      AS level,
                    c.attribute                                  AS attribute,
                    c.race                                       AS race,
                    ct_en.name                                   AS name_en,
                    ct_fr.name                                   AS name_fr,
                    sp_fr.extended_art                           AS extended_art
                FROM set_prints sp_fr
                LEFT JOIN (
                    -- Sous-requête : un seul set_code EN par couple
                    -- (card_uuid, card_image_uuid). MIN() pour
                    -- déterminisme si plusieurs set_codes coexistaient
                    -- en EN — situation extrêmement rare en pratique.
                    SELECT
                        card_uuid,
                        card_image_uuid,
                        MIN(set_code) AS set_code_min
                    FROM set_prints
                    WHERE set_locale_id = ?
                    GROUP BY card_uuid, card_image_uuid
                ) sp_en
                    ON sp_en.card_uuid       = sp_fr.card_uuid
                   AND sp_en.card_image_uuid = sp_fr.card_image_uuid
                JOIN cards c ON c.uuid = sp_fr.card_uuid
                LEFT JOIN card_images ci ON ci.uuid = sp_fr.card_image_uuid
                LEFT JOIN card_texts ct_en ON ct_en.card_uuid = c.uuid
                                           AND ct_en.language = 'en'
                LEFT JOIN card_texts ct_fr ON ct_fr.card_uuid = c.uuid
                                           AND ct_fr.language = 'fr'
                WHERE sp_fr.set_locale_id = ?
                ORDER BY COALESCE(sp_en.set_code_min, sp_fr.set_code), sp_fr.rarity
            """, (locale_en_id, locale_enum_id))
        else:
            # Énumération directe depuis la locale choisie (set_locale_id).
            # Cette branche couvre :
            #   - mode_enum == 'en' (TCG sans FR utilisable)
            #   - mode_enum == 'ja' (OCG-JP, classeurs `LOCH-JP`, `CROS-JP`…)
            # Les set_codes énumérés (`sp.set_code`) sont natifs de la locale
            # choisie : `LOCH-JP001` pour 'ja', `RA05-EN001` pour 'en', etc.
            # Note : sets EN-only conservent les doublons d'artworks historiques
            # YGOJSON. Limite documentée — peu fréquent en pratique.
            cursor.execute("""
                SELECT
                    sp.set_code             AS set_code_en,
                    sp.rarity               AS rarity,
                    ci.ygoprodeck_image_id  AS card_image_id,
                    ci.card_url             AS card_image_url,
                    ci.art_url              AS card_image_small,
                    c.uuid                  AS card_uuid,
                    ci.uuid                 AS card_image_uuid,
                    c.card_type             AS card_type,
                    c.atk                   AS atk,
                    c.def                   AS def_val,
                    c.level                 AS level,
                    c.attribute             AS attribute,
                    c.race                  AS race,
                    ct_en.name              AS name_en,
                    ct_fr.name              AS name_fr,
                    sp.extended_art         AS extended_art
                FROM set_prints sp
                JOIN cards c ON c.uuid = sp.card_uuid
                LEFT JOIN card_images ci ON ci.uuid = sp.card_image_uuid
                LEFT JOIN card_texts ct_en ON ct_en.card_uuid = c.uuid
                                           AND ct_en.language = 'en'
                LEFT JOIN card_texts ct_fr ON ct_fr.card_uuid = c.uuid
                                           AND ct_fr.language = 'fr'
                WHERE sp.set_locale_id = ?
                ORDER BY sp.set_code, sp.rarity
            """, (locale_enum_id,))

        records = cursor.fetchall()

    if not records:
        raise SetNotInLocalDB(
            f"Set '{code_set}' connu mais aucune carte trouvée dans cardinfo.db. "
            f"Lancez 'MAJ BDD' pour reconstruire la base."
        )

    # 4) Conversion vers le format attendu (mêmes clés que la version API)
    for rec in records:
        (set_code, rarity, card_image_id, card_image_url,
         card_image_small, card_uuid, card_image_uuid,
         card_type, atk, def_val, level, attribute, race,
         name_en, name_fr, extended_art) = rec

        rows.append({
            "card_uuid":        card_uuid or "",
            "card_image_uuid":  card_image_uuid or "",
            "card_image_id":    card_image_id,
            "name":             name_en or "",
            "name_fr":          name_fr or "",
            "set_code":         set_code or "",
            "rarity":           rarity or "",
            "rarity_code":      "",  # YGOPRODeck-only, pas dans cardinfo.db
            "set_name":         set_name_en or "",
            "card_image_url":   card_image_url or "",
            "card_image_small": card_image_small or "",
            "sort_order":       0,   # placeholder, écrasé par le rank ci-dessous
            "card_type":        card_type or "",
            "atk":              atk,
            "def_val":          def_val,
            "level":            level,
            "attribute":        attribute or "",
            "race":             race or "",
            "extended_art":     int(extended_art or 0),
        })

    # Tri Python explicite (letter_group, number, rarity), puis renumérotation
    # en rangs séquentiels. Aligné avec `_fetch_rows_from_api` étape 5 et
    # `tri_carte._extract_numero` pour produire le même ordre dans tous les
    # chemins de création (local et API) et l'affichage.
    rows.sort(key=lambda r: (
        _sort_key_from_code(r["set_code"]),
        1 if r.get("extended_art") else 0,   # Overframe = bloc distinct
        r.get("rarity", ""),
    ))
    for i, row in enumerate(rows):
        row["sort_order"] = i

    return rows


def _save_classeur_from_rows_local(code_set: str, rows: list[dict]) -> bool:
    """
    Sauvegarde sur disque un classeur depuis des `rows` déjà construites.

    Crée le dossier `classeurs/<code_set>/`, la BDD `<code_set>.db` avec son
    schéma, insère les lignes et applique le hook anomalies.

    Cette fonction est extraite de `_create_classeur_from_local` (mai 2026)
    pour permettre à `create_classeur()` d'inspecter les rows avant de
    décider local vs API, sans avoir à créer puis supprimer un classeur.

    Lève une exception en cas d'erreur d'I/O (ne masque pas).
    Retourne True (création réussie). Le caller a déjà vérifié que le
    classeur n'existe pas (court-circuit `os.path.exists`).
    """
    classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)
    db_file       = os.path.join(classeur_path, f"{code_set}.db")

    os.makedirs(classeur_path)

    # Correction rareté : YGOPRODeck insère parfois une annotation dans
    # set_rarity qui REMPLACE une vraie rareté (ex CH02-EN001 : 'New artwork'
    # à la place de 'Ultra Rare'). On complète/corrige depuis Yugipedia plutôt
    # que de supprimer (sinon la vraie rareté disparaîtrait). Fallback interne
    # si Yugipedia est injoignable : les lignes invalides sont écartées.
    try:
        rows = corriger_rows(code_set, rows)
    except Exception as e:
        log.warning(f"{code_set}: correction rareté Yugipedia ignorée ({e})")

    # Filet de sécurité : l'INSERT utilise le paramètre nommé :extended_art,
    # qui doit exister sur CHAQUE ligne quel que soit le chemin de construction
    # (local, API, reprise…). On garantit la clé sans écraser une valeur posée.
    for r in rows:
        r.setdefault("extended_art", 0)

    with sqlite_ctx(db_file) as conn:
        conn.execute("""
            CREATE TABLE cards (
                card_uuid        TEXT,
                card_image_uuid  TEXT,
                card_image_id    INTEGER,
                set_code         TEXT,
                rarity           TEXT,
                rarity_code      TEXT    DEFAULT '',
                set_name         TEXT,
                name             TEXT,
                name_fr          TEXT    DEFAULT '',
                card_image_url   TEXT,
                card_image_small TEXT    DEFAULT '',
                sort_order       INTEGER DEFAULT 0,
                card_type        TEXT    DEFAULT '',
                atk              INTEGER,
                def_val          INTEGER,
                level            INTEGER,
                attribute        TEXT    DEFAULT '',
                race             TEXT    DEFAULT '',
                possessed        INTEGER DEFAULT 0,
                quantite         INTEGER DEFAULT 0,
                qualite          TEXT    DEFAULT NULL,
                edition          TEXT    DEFAULT NULL,
                extended_art     INTEGER DEFAULT 0,
                is_custom        INTEGER DEFAULT 0
            )
        """)
        conn.execute("CREATE INDEX idx_sort    ON cards(sort_order)")
        conn.execute("CREATE INDEX idx_card_id ON cards(card_image_id)")
        conn.executemany("""
            INSERT INTO cards
                (card_uuid, card_image_uuid, card_image_id,
                 set_code, rarity, rarity_code, set_name,
                 name, name_fr, card_image_url, card_image_small,
                 sort_order, card_type, atk, def_val, level, attribute, race,
                 extended_art)
            VALUES
                (:card_uuid, :card_image_uuid, :card_image_id,
                 :set_code, :rarity, :rarity_code, :set_name,
                 :name, :name_fr, :card_image_url, :card_image_small,
                 :sort_order, :card_type, :atk, :def_val, :level, :attribute, :race,
                 :extended_art)
        """, rows)
        _ensure_meta_table(conn)

    # Hook auto-migration : rectifie les set_codes non-EN et les raretés
    # numériques. Pour un classeur fraîchement créé via le pipeline actuel,
    # c'est un no-op (set_codes déjà EN, raretés déjà correctes). Sert de
    # safety net défensif si une régression future réintroduisait ces bugs.
    # Doit s'exécuter AVANT le hook anomalies pour que ce dernier matche
    # bien sur les set_codes et raretés canoniques.
    try:
        from module.utilitaire.migration_set_codes import reparer_classeur
        reparer_classeur(code_set)
    except Exception as e:
        log.warning(f"_save_classeur_from_rows_local : hook migration pour {code_set}: {e}")

    # Hook anomalies (identique à _create_classeur_from_api)
    try:
        from module.anomalie.anomalie_service import appliquer_overrides_sur_classeur_neuf
        appliquer_overrides_sur_classeur_neuf(db_file)
    except Exception as e:
        log.warning(f"_save_classeur_from_rows_local : hook overrides pour {code_set}: {e}")

    return True


def _create_classeur_from_local(code_set: str) -> bool:
    """
    Crée un classeur en lisant exclusivement cardinfo.db (zéro appel API).

    Comportement identique à _create_classeur_from_api côté disque :
      - Crée le dossier classeurs/<code_set>/
      - Crée la BDD <code_set>.db avec la même structure (table cards + index)
      - Insère les cartes
      - Applique le hook anomalies (overrides connus)

    Retourne True si créé, False si déjà existant.
    Lève SetNotInLocalDB si cardinfo.db ne connaît pas le set.

    Cette fonction est la voie principale appelée par l'orchestrateur
    public `create_classeur()`. Elle évite tout appel réseau et garantit
    une création instantanée tant que cardinfo.db est à jour. En cas
    d'absence du set (set très récent par exemple), `create_classeur()`
    bascule sur `_create_classeur_from_api()` en fallback.

    Note historique (mai 2026) : l'import CSV en lot appelait jadis cette
    fonction en direct pour ne pas dépendre du réseau. Cette décision a
    été révoquée car elle créait des classeurs avec des artworks alts
    "hallucinés" sur certains sets (Legendary Decks, Battle Pack, etc.)
    dont les `set_prints` YGOJSON contiennent des artefacts. L'import CSV
    passe désormais par `create_classeur()` comme la création manuelle, ce
    qui restitue la cohérence des deux chemins. Le mode hors-ligne reste
    garanti via le fallback final sur le local au sein de `create_classeur`.

    Architecture interne (mai 2026) : depuis l'extraction de
    `_save_classeur_from_rows_local`, cette fonction n'est plus qu'une
    façade qui combine `_build_rows_from_local_db` + save. La séparation
    permet à `create_classeur()` d'inspecter les rows avant de décider
    local vs API (détection d'incomplétude type RA05).
    """
    code_set = str(code_set).strip().upper()
    classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)
    if os.path.exists(classeur_path):
        return False

    # Construction des lignes depuis cardinfo.db (peut lever SetNotInLocalDB)
    rows = _build_rows_from_local_db(code_set)
    if not rows:
        raise SetNotInLocalDB(
            f"Set '{code_set}' présent mais aucune carte exploitable. "
            f"Lancez 'MAJ BDD' pour reconstruire la base."
        )

    return _save_classeur_from_rows_local(code_set, rows)


# ─────────────────────────────────────────────────────────────────────────────
# Helpers de détection d'incomplétude (correctif RA05 — mai 2026)
# ─────────────────────────────────────────────────────────────────────────────

# Seuil heuristique : moyenne raretés/carte en dessous de laquelle on
# suspecte une incomplétude des données YGOJSON (cas type RA05 où 80/150
# cartes sont taguées "Common" alors que le set sort en 7 raretés).
#
# Pourquoi 2.0 :
#   - Les Booster Packs et Rarity Collection sets ont typiquement >= 2 raretés
#     par carte (souvent 3-7).
#   - Les Structure Decks ont 1 rareté par carte (Common). Pour eux, le
#     fallback API serait déclenché à tort, mais sans dommage : YGOPRODeck
#     retournera également 1 rareté/carte, donc le résultat final est
#     identique. Coût : 1 appel API "inutile". Acceptable car ces sets sont
#     rarement créés (le user les a déjà tous).
#   - Pour RA05 (avg=1.52), le seuil 2.0 déclenche bien le fallback.
SEUIL_AVG_RARETES_SUSPECT = 2.0


def _rows_locales_semblent_incompletes(rows: list[dict]) -> tuple[bool, float, int]:
    """
    Heuristique de détection d'incomplétude des données YGOJSON locales.

    Calcule la moyenne raretés/carte et compare au seuil. Si moyenne basse,
    on considère le set suspect → caller doit basculer sur YGOPRODeck.

    Retourne (suspect: bool, avg: float, nb_cartes_uniques: int).

    Diagnostic confirmé sur RA05 (mai 2026) :
      - YGOJSON : 228 prints, 150 cartes uniques → avg = 1.52 (suspect)
      - YGOPRODeck : ~7 raretés par carte du main pool → avg ≈ 5+
    """
    nb_cartes_uniques = len({r.get("card_uuid") for r in rows if r.get("card_uuid")})
    if nb_cartes_uniques == 0:
        return False, 0.0, 0
    avg = len(rows) / nb_cartes_uniques
    suspect = avg < SEUIL_AVG_RARETES_SUSPECT
    return suspect, avg, nb_cartes_uniques


# ─────────────────────────────────────────────────────────────────────────────
# Orchestrateur public : cardinfo.db prioritaire, API en fallback
# ─────────────────────────────────────────────────────────────────────────────

def create_classeur(code_set: str) -> bool:
    """
    Crée un classeur SQLite pour le set donné.

    Stratégie (révisée mai 2026 pour gérer l'incomplétude YGOJSON) :
      1. Tente d'abord la construction des lignes depuis cardinfo.db locale
         (instantanée, offline, zéro appel réseau).
      2. Si le set est ABSENT localement → fallback API YGOPRODeck.
      3. Si le set est PRÉSENT mais semble INCOMPLET (heuristique : moyenne
         raretés/carte < SEUIL_AVG_RARETES_SUSPECT) → bascule sur l'API
         YGOPRODeck, qui contient toutes les raretés à jour.
         Cas typique : sets récents (RA05) où YGOJSON n'a importé qu'une
         partie des raretés (ex. tout en "Common" alors que le set est
         all-foil 7-raretés). Voir _rows_locales_semblent_incompletes.
      4. Si le set est complet localement → création locale immédiate.

    Retourne True si créé, False si déjà existant.
    Lève ValueError si toutes les sources échouent.

    Important : la signature publique est inchangée. Les callers existants
    (file_attente_classeur.py, etc.) n'ont aucune modification à faire.

    Comportement de fallback en cas d'échec API
    ───────────────────────────────────────────
    Si le local est suspect mais que l'API est inaccessible (réseau down),
    on retombe sur le local malgré l'incomplétude — mieux d'avoir un
    classeur partiel que pas de classeur du tout. L'utilisateur peut
    refaire MAJ BDD plus tard quand YGOJSON aura corrigé ses données.
    """
    code_set = str(code_set).strip().upper()
    classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)

    # Court-circuit : un classeur DÉJÀ PEUPLÉ n'est jamais recréé.
    # En revanche, un dossier RÉSIDUEL (suppression partielle, .db absent ou
    # vide, fichiers WAL orphelins) ne doit PAS bloquer la re-création :
    # on le purge puis on recrée. C'est ce qui permet de re-télécharger un
    # classeur immédiatement après l'avoir supprimé.
    if os.path.exists(classeur_path):
        if classeur_db_est_peuple(code_set):
            return False
        try:
            shutil.rmtree(classeur_path, onerror=remove_readonly)
        except Exception as e:
            log.warning(
                f"create_classeur({code_set}) : purge dossier résiduel impossible : {e}")

    # Détection OCG-JP : si le code finit par un suffixe régional OCG
    # (`-JP`, `-JA`, `-KR`, `-AE`, `-SC`…), le fallback API YGOPRODeck
    # n'est PAS compatible — l'API indexe les sets par préfixe nu
    # (`set_code='LOCH'`), pas par préfixe + langue. On désactive donc
    # la cascade vers l'API : si le set OCG n'est pas dans cardinfo.db,
    # on remonte une erreur claire invitant à faire MAJ BDD.
    from module.config.preferences import a_suffixe_ocg as _is_ocg_classeur
    is_ocg = _is_ocg_classeur(code_set)

    # ── Étape 1 : tenter la construction locale ─────────────────────────────
    rows_local: list[dict] | None = None
    raison_locale: str | None = None
    try:
        rows_local = _build_rows_from_local_db(code_set)
        if not rows_local:
            raison_locale = (
                f"Set '{code_set}' présent mais aucune carte exploitable."
            )
            rows_local = None
    except SetNotInLocalDB as e_local:
        raison_locale = str(e_local)
        rows_local = None
    # NB : les autres exceptions (I/O, corruption SQLite) se propagent
    # naturellement — on ne les masque pas par un fallback API silencieux.

    # ── Étape 2 : si local absent → API directement (sauf OCG-JP) ───────────
    if rows_local is None:
        if is_ocg:
            # OCG-JP : pas de fallback API possible (cf. note plus haut).
            raise ValueError(
                f"Impossible de créer le classeur OCG '{code_set}' :\n"
                f"  - cardinfo.db locale : {raison_locale}\n"
                f"  - L'API YGOPRODeck ne prend pas en charge le format OCG-JP.\n"
                f"  Lancez 'MAJ BDD' depuis l'écran principal pour mettre à jour."
            )
        try:
            return _create_classeur_from_api(code_set)
        except ValueError as e_api:
            raise ValueError(
                f"Impossible de créer le classeur '{code_set}' :\n"
                f"  - cardinfo.db locale : {raison_locale}\n"
                f"  - API YGOPRODeck    : {e_api}"
            ) from e_api

    # ── Étape 3 : local présent — vérifier la complétude ────────────────────
    suspect, avg, nb_cartes = _rows_locales_semblent_incompletes(rows_local)

    if suspect and not is_ocg:
        # Données YGOJSON probablement incomplètes (cas RA05). On bascule
        # sur l'API qui contient les raretés à jour, et on n'écrit RIEN sur
        # disque tant que l'API n'a pas répondu — pas de classeur partiel
        # à supprimer en cas d'échec.
        #
        # IMPORTANT — ne pas tenter de "préférer le local quand il a
        # plus de cartes que l'API"
        # ─────────────────────────────────────────────────────────────
        # Tentation : pour LDK2 où le local a 132 (3 Blue-Eyes alts) et
        # l'API n'en retourne que 130, comparer les deux et garder le
        # plus complet. ÇA RÉINTRODUIT UN BUG MAJEUR :
        #
        # YGOJSON contient des `set_prints` "hallucinés" pour beaucoup
        # de sets (Legendary Decks, Battle Pack, certains sets RA…). Il
        # associe TOUS les artworks alt globalement connus de chaque
        # carte au set, même ceux qui ne sont PAS physiquement dans le
        # set. Conséquence d'utiliser le local : Obelisk/Slifer × 3
        # artworks au lieu d'1 dans les sets RA, LDK2 à ~189 cartes au
        # lieu de 132, etc.
        #
        # Le compromis acceptable est : perdre quelques cartes (130 vs
        # 132 pour LDK2) plutôt que d'introduire des artworks fantômes
        # dans des dizaines d'autres classeurs. Les artworks alt
        # manquants peuvent être ajoutés à la main via le scan
        # d'anomalies ou le clic droit "Modifier l'artwork".
        log.info(
            f"{code_set} : moyenne raretés/carte = {avg:.2f} "
            f"sur {nb_cartes} cartes (< {SEUIL_AVG_RARETES_SUSPECT}). "
            f"Tentative API YGOPRODeck pour données complètes..."
        )
        try:
            return _create_classeur_from_api(code_set)
        except Exception as e_api:
            # API down ou erreur : on retombe sur le local malgré
            # l'incomplétude. Mieux qu'aucun classeur.
            log.warning(
                f"{code_set} : API échouée ({e_api}). "
                f"Création depuis local malgré données partielles. "
                f"Refaites 'MAJ BDD' plus tard quand YGOJSON sera à jour."
            )

    # ── Étape 4 : sauvegarder depuis le local ───────────────────────────────
    return _save_classeur_from_rows_local(code_set, rows_local)


# ─────────────────────────────────────────────────────────────────────────────
# Helpers de lecture (lecture seule, indépendants du chemin de création)
# ─────────────────────────────────────────────────────────────────────────────


def get_classeur_meta(db_path: str) -> tuple[int, int, int]:
    """
    Lit en UNE SEULE ouverture SQLite : (card_count, cols, rows).

    Optimisation du chargement de l'écran d'accueil : remplace les appels
    séquentiels get_classeur_card_count() + get_classeur_config() qui
    ouvraient 2 connexions distinctes par classeur.

    Retourne (0, default_cols, default_rows) en cas d'erreur (ex. fichier
    BDD corrompu ou table cards absente), cohérent avec les helpers legacy.
    """
    from module.config.preferences import get_grille_defaut
    default_cols, default_rows = get_grille_defaut()

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            # Table meta peut être absente sur classeurs G1 historiques :
            # on la crée à la volée via le helper centralisé (Y8).
            _ensure_meta_table(conn)
            # Card count
            cursor.execute("SELECT COUNT(*) FROM cards")
            row = cursor.fetchone()
            card_count = row[0] if row else 0

            # Grille
            cursor.execute("SELECT value FROM meta WHERE key='colonnes'")
            row_cols = cursor.fetchone()
            cursor.execute("SELECT value FROM meta WHERE key='lignes'")
            row_ligs = cursor.fetchone()
            cols = int(row_cols[0]) if row_cols else default_cols
            rows = int(row_ligs[0]) if row_ligs else default_rows

        return card_count, cols, rows
    except Exception as e:
        log.warning(f"get_classeur_meta : {e}")
        return 0, default_cols, default_rows


def get_premiere_image_id(db_path: str) -> str | None:
    """
    Retourne le card_image_id d'une carte représentative du classeur
    (la première dont l'image_id n'est pas NULL), ou None.

    Utilisé par l'écran d'accueil pour choisir une image de couverture
    de classeur quand aucun booster n'est disponible.
    """
    try:
        with sqlite_ctx(db_path) as conn:
            row = conn.execute(
                "SELECT card_image_id FROM cards "
                "WHERE card_image_id IS NOT NULL LIMIT 1"
            ).fetchone()
            return row[0] if row else None
    except Exception as e:
        log.warning(f"get_premiere_image_id : {e}")
        return None


def get_classeur_meta_full(db_path: str) -> tuple[int, int, int, str | None]:
    """Comme get_classeur_meta mais lit AUSSI le card_image_id de couverture
    dans la MÊME ouverture SQLite.

    Optimisation accueil : remplace get_classeur_meta() + get_premiere_image_id()
    (2 ouvertures par classeur) par une seule connexion → (card_count, cols,
    rows, image_id). image_id peut être None.

    Retourne (0, default_cols, default_rows, None) en cas d'erreur, cohérent
    avec get_classeur_meta.
    """
    from module.config.preferences import get_grille_defaut
    default_cols, default_rows = get_grille_defaut()

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            _ensure_meta_table(conn)

            cursor.execute("SELECT COUNT(*) FROM cards")
            row = cursor.fetchone()
            card_count = row[0] if row else 0

            cursor.execute("SELECT value FROM meta WHERE key='colonnes'")
            row_cols = cursor.fetchone()
            cursor.execute("SELECT value FROM meta WHERE key='lignes'")
            row_ligs = cursor.fetchone()
            cols = int(row_cols[0]) if row_cols else default_cols
            rows = int(row_ligs[0]) if row_ligs else default_rows

            row_img = cursor.execute(
                "SELECT card_image_id FROM cards "
                "WHERE card_image_id IS NOT NULL LIMIT 1"
            ).fetchone()
            image_id = row_img[0] if row_img else None

        return card_count, cols, rows, image_id
    except Exception as e:
        log.warning(f"get_classeur_meta_full : {e}")
        return 0, default_cols, default_rows, None


def classeur_db_est_peuple(code: str) -> bool:
    """True si le `.db` du classeur existe ET contient au moins une carte.

    Sert à distinguer un classeur réellement créé (court-circuit de
    create_classeur, saut de création par le worker) d'un dossier résiduel
    vide laissé par une suppression partielle (Windows + WAL). Robuste :
    toute erreur d'accès (DB absente, corrompue, verrouillée) -> False.
    """
    dbp = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    if not os.path.exists(dbp):
        return False
    try:
        with sqlite_ctx(dbp) as conn:
            cur = conn.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='cards'")
            if not cur.fetchone():
                return False
            return conn.execute("SELECT COUNT(*) FROM cards").fetchone()[0] > 0
    except Exception:
        return False


def supprimer_classeur(code: str) -> bool:
    """
    Supprime définitivement le dossier d'un classeur (BDD + images) du disque.

    Robustesse Windows (WAL activé sur les bases classeur) :
      - purge explicite des fichiers SQLite -wal/-shm (un mapping résiduel
        peut faire échouer rmtree) ;
      - plusieurs tentatives entrecoupées de gc.collect()+pause (laisse le
        temps de libérer un handle transitoire : worker d'images, GC…) ;
      - en dernier recours, suppression du seul `.db` afin qu'un dossier
        résiduel ne bloque PAS la re-création (cf. create_classeur, qui
        court-circuite sur la présence d'une DB peuplée et nettoie sinon).

    Gère les fichiers en lecture seule via remove_readonly (handler onerror).
    Retourne True si le dossier a été entièrement supprimé (ou n'existait
    déjà plus), False sinon.
    """
    import gc
    import glob
    import time

    path = os.path.join(CLASSEUR_FOLDER, code)
    if not os.path.exists(path):
        return True

    gc.collect()  # libère d'éventuelles connexions SQLite non référencées

    # Purge des fichiers WAL/SHM d'abord (sinon rmtree peut échouer sous Windows).
    for extra in glob.glob(os.path.join(path, "*.db-wal")) + \
                 glob.glob(os.path.join(path, "*.db-shm")):
        try:
            os.chmod(extra, stat.S_IWRITE)
            os.remove(extra)
        except Exception:
            pass

    last_err = None
    for _ in range(4):
        try:
            shutil.rmtree(path, onerror=remove_readonly)
            return True
        except Exception as e:
            last_err = e
            gc.collect()
            time.sleep(0.25)

    # Échec rmtree : on supprime au moins le .db pour débloquer la re-création.
    try:
        dbp = os.path.join(path, f"{code}.db")
        if os.path.exists(dbp):
            os.chmod(dbp, stat.S_IWRITE)
            os.remove(dbp)
    except Exception:
        pass

    log.warning(f"supprimer_classeur({code}) : {last_err}")
    return False

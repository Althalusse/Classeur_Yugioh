"""
anomalie_service.py
===================
Gestion des anomalies d'artwork Yu-Gi-Oh.

Principe :
  Une anomalie = une carte avec plusieurs artworks (card_image_uuid différents)
  dont l'un est absent de certains sets où l'autre artwork est présent.
  Stockage dans la table `anomalies` de cardinfo.db.
  Correction = INSERT direct dans le classeur SQLite concerné.
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
from collections import defaultdict

from module.centralisation_dossier import CARDINFO_DB, CLASSEUR_FOLDER, sqlite_ctx
from module.logger_app import log


# ─────────────────────────────────────────────────────────────────────────────
# 1. Table anomalies
# ─────────────────────────────────────────────────────────────────────────────

def _ensure_anomalies_table():
    with sqlite_ctx(CARDINFO_DB) as conn:
        conn.execute("""
            CREATE TABLE IF NOT EXISTS anomalies (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                name                TEXT    NOT NULL,
                art_a_image_uuid    TEXT    NOT NULL,
                art_b_image_uuid    TEXT    NOT NULL,
                art_index           INTEGER NOT NULL,
                set_code_prefix     TEXT    NOT NULL,
                missing_set_code    TEXT    NOT NULL,
                missing_set_rarity  TEXT    NOT NULL,
                image_url           TEXT,
                image_url_small     TEXT,
                image_id            INTEGER,
                corrige             INTEGER DEFAULT 0,
                UNIQUE(art_b_image_uuid, missing_set_code, missing_set_rarity)
            )
        """)


class CardinfoIncompleteError(Exception):
    """Soulevée quand cardinfo.db n'a pas les tables requises pour le scan.

    C'est typiquement le cas au premier démarrage si l'initialisation BDD
    (show_init_window → run_init) n'a pas été exécutée. L'application peut
    créer des classeurs sans cardinfo.db (via l'API YGOPRODeck directement),
    mais le scan d'anomalies requiert les tables `set_prints`, `set_locales`,
    `card_texts` et `card_images` peuplées par YGOJSON.
    """


_REQUIRED_TABLES = ("set_prints", "set_locales", "card_texts", "card_images")


def _prefix_classeur_depuis_set_code(set_code: str) -> str:
    """Calcule le préfixe identifiant le classeur à partir d'un set_code complet.

    Règle :
      - Pour un set_code TCG (`CROS-EN001`, `RA02-EU006`) → préfixe NU
        (`CROS`, `RA02`). C'est le nom du dossier classeur sur disque.
      - Pour un set_code OCG (`LOCH-JP001`, `CROS-JP002`) → préfixe COMPLET
        avec suffixe régional (`LOCH-JP`, `CROS-JP`). Cela correspond
        également au nom de dossier classeur sur disque.

    Le retour de cette fonction sert deux finalités :
      1. Filtrer les anomalies par classeur existant sur disque (via
         get_prefixes_classeurs_existants).
      2. Stocker `set_code_prefix` dans la table anomalies de manière
         cohérente avec l'identifiant du classeur (le `prefix_filtre`
         passé à lire_anomalies vient de `self._code` côté UI = nom de
         dossier).

    Exemples :
      'CROS-EN001'  → 'CROS'
      'RA02-EU006'  → 'RA02'
      'LOB-EN1'     → 'LOB'
      'LOCH-JP001'  → 'LOCH-JP'
      'CROS-JP002'  → 'CROS-JP'
      'L26D-ENM01'  → 'L26D'  (sets multi-decks : suffixe ENM/ENS/… non OCG)
      ''            → ''
    """
    if not set_code:
        return ""
    s = set_code.strip().upper()
    if "-" not in s:
        return s
    # Découpe sur le PREMIER tiret pour isoler [préfixe nu, reste]
    head, _, tail = s.partition("-")
    # `tail` commence par le code-langue (1 à 3 lettres) suivi du numéro.
    # On extrait la partie alphabétique de tête de tail.
    i = 0
    while i < len(tail) and tail[i].isalpha():
        i += 1
    code_lang = tail[:i]
    # Si le code-langue est un suffixe OCG, on conserve `head-code_lang`
    # comme préfixe (= nom de dossier classeur OCG).
    # Import différé pour éviter cycle avec preferences.
    from module.config.preferences import get_ocg_suffixes
    if code_lang in get_ocg_suffixes():
        return f"{head}-{code_lang}"
    return head


def _verifier_cardinfo_complete() -> tuple[bool, list[str]]:
    """Retourne (ok, tables_manquantes).

    Ne soulève pas d'exception si cardinfo.db est absent : retourne juste la
    liste des tables attendues.
    """
    if not os.path.exists(CARDINFO_DB):
        return False, list(_REQUIRED_TABLES)
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            manquantes = []
            for tbl in _REQUIRED_TABLES:
                cursor.execute(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
                    (tbl,),
                )
                if not cursor.fetchone():
                    manquantes.append(tbl)
            return (len(manquantes) == 0), manquantes
    except Exception:
        return False, list(_REQUIRED_TABLES)


# ─────────────────────────────────────────────────────────────────────────────
# 2. Classeurs existants
# ─────────────────────────────────────────────────────────────────────────────

def get_prefixes_classeurs_existants() -> set:
    if not os.path.exists(CLASSEUR_FOLDER):
        return set()
    return {
        d for d in os.listdir(CLASSEUR_FOLDER)
        if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))
    }


# ─────────────────────────────────────────────────────────────────────────────
# 3. Scan et stockage
# ─────────────────────────────────────────────────────────────────────────────

def scanner_et_stocker_anomalies() -> int:
    """
    Scanne cardinfo.db (set_prints × card_images), détecte les artworks manquants
    pour les sets dont le prefix correspond à un classeur existant.

    Filtre langue : déterminé par préférences.get_langues_locales_actives() :
    toujours ('en', 'eu') pour les sets TCG, +('ja',) si l'utilisateur a
    activé "Inclure les sets japonais (OCG)".

    Retourne le nombre total d'anomalies en base.

    Lève CardinfoIncompleteError si cardinfo.db n'est pas initialisée — dans
    ce cas, l'utilisateur doit lancer l'initialisation de la base depuis
    l'interface (setup YGOJSON + YGOPRODeck via module.BDD_creation.run_init).
    """
    _ensure_anomalies_table()

    ok, manquantes = _verifier_cardinfo_complete()
    if not ok:
        raise CardinfoIncompleteError(
            "La base de données interne (cardinfo.db) n'est pas initialisée.\n"
            "Le scan d'anomalies nécessite les tables : "
            + ", ".join(manquantes) + ".\n\n"
            "Lancez l'initialisation depuis l'onglet « Version » ou redémarrez "
            "l'application après avoir supprimé « first_run.flag »."
        )

    prefixes = get_prefixes_classeurs_existants()
    if not prefixes:
        return 0

    # Filtre langue dynamique : aligné sur la préférence "Inclure OCG-JP".
    from module.config import preferences as _prefs
    langues_actives = _prefs.get_langues_locales_actives()
    ph_lang = ",".join("?" * len(langues_actives))

    with sqlite_ctx(CARDINFO_DB) as conn:
        cursor = conn.cursor()
        # Une ligne par (artwork × set_print) avec nom EN de la carte.
        # Filtre langue dynamique cohérent avec create_classeur.
        cursor.execute(f"""
            SELECT
                ci.ygoprodeck_image_id AS image_id,
                ct.name,
                sp.set_code,
                sp.rarity,
                ci.card_url   AS image_url,
                ci.art_url    AS image_url_small,
                ci.uuid       AS image_uuid,
                sp.card_uuid,
                sp.card_image_uuid
            FROM set_prints sp
            JOIN set_locales sl ON sl.id = sp.set_locale_id
                               AND sl.language IN ({ph_lang})
            JOIN card_texts ct ON ct.card_uuid = sp.card_uuid AND ct.language = 'en'
            JOIN card_images ci ON ci.uuid = sp.card_image_uuid
            WHERE sp.set_code IS NOT NULL
            ORDER BY ct.name, ci.ygoprodeck_image_id
        """, langues_actives)
        rows = cursor.fetchall()

    # Regroupe par (card_uuid, image_uuid) → set de (set_code, rarity)
    # Structure : by_name[name][image_uuid] = liste de prints
    by_name = defaultdict(lambda: defaultdict(list))
    image_meta = {}  # image_uuid → {image_id, image_url, image_url_small}

    for image_id, name, set_code, rarity, image_url, image_url_small, image_uuid, card_uuid, _ in rows:
        by_name[name][image_uuid].append({"set_code": set_code, "rarity": rarity})
        if image_uuid not in image_meta:
            image_meta[image_uuid] = {
                "image_id": image_id,
                "image_url": image_url,
                "image_url_small": image_url_small,
            }

    anomalies_a_inserer = []
    for name, artworks in by_name.items():
        if len(artworks) < 2:
            continue
        sorted_uuids = sorted(artworks.keys())

        # Compare chaque paire d'artworks dans LES DEUX SENS.
        # Sans bidirectionnel, une anomalie n'est détectée que si l'artwork
        # alphabétiquement premier (art_a) est présent mais le second (art_b)
        # est absent — le sens inverse (art_b présent, art_a absent) était ignoré.
        # Exemple : Droll & Lock Bird RA02-EN006 (Art1 présent, Art2 absent)
        # n'était pas détecté car Art2 (uuid 486a…) < Art1 (uuid 56b5…),
        # ce qui faisait de Art2 le "art_a" et de Art1 le "art_b" —
        # la direction RA02 (manque Art2) n'était donc jamais calculée.
        #
        # Complexité : O(N²) en nombre d'artworks par carte (Y9).
        # Borne pratique observée : Blue-Eyes White Dragon ≈ 15 artworks
        # (225 comparaisons), négligeable. Pour ~13 000 cartes uniques et
        # une moyenne de 2-3 artworks par carte, le scan complet s'exécute
        # en < 1 s. Si un jour une carte dépasse 30 artworks (~900
        # comparaisons × ratio), envisager un algorithme par différence
        # symétrique de sets (O(N) amorti).
        for i, art_a_uuid in enumerate(sorted_uuids):
            art_a_prints = {(p["set_code"], p["rarity"]) for p in artworks[art_a_uuid]}
            for j, art_b_uuid in enumerate(sorted_uuids):
                if i == j:
                    continue
                art_b_prints = {(p["set_code"], p["rarity"]) for p in artworks[art_b_uuid]}
                # Sets où art_a est présent mais art_b est absent → art_b manquant
                missing = art_a_prints - art_b_prints
                if not missing:
                    continue
                meta = image_meta.get(art_b_uuid, {})
                for missing_code, missing_rarity in sorted(missing):
                    prefix = _prefix_classeur_depuis_set_code(missing_code)
                    if prefix not in prefixes:
                        continue
                    anomalies_a_inserer.append((
                        name,
                        art_a_uuid,
                        art_b_uuid,
                        j + 1,
                        prefix,
                        missing_code,
                        missing_rarity,
                        meta.get("image_url", ""),
                        meta.get("image_url_small", ""),
                        meta.get("image_id"),
                    ))

    with sqlite_ctx(CARDINFO_DB) as conn:
        conn.executemany("""
            INSERT OR IGNORE INTO anomalies
                (name, art_a_image_uuid, art_b_image_uuid, art_index,
                 set_code_prefix, missing_set_code, missing_set_rarity,
                 image_url, image_url_small, image_id)
            VALUES (?,?,?,?,?,?,?,?,?,?)
        """, anomalies_a_inserer)

        conn.execute(
            "DELETE FROM anomalies WHERE set_code_prefix NOT IN ({})".format(
                ",".join("?" for _ in prefixes)
            ),
            list(prefixes)
        )

        cursor = conn.cursor()
        cursor.execute("SELECT COUNT(*) FROM anomalies")
        return cursor.fetchone()[0]


# ─────────────────────────────────────────────────────────────────────────────
# 4. Lecture
# ─────────────────────────────────────────────────────────────────────────────

def _anomalie_sort_key(anomalie: dict) -> tuple:
    """Clé de tri alignée sur le classeur : (prefix, letter_group, numéro, rareté, nom).

    Doit rester ALIGNÉ avec `tri_carte._extract_numero` et
    `creation_classeur_service._sort_key_from_code` pour que le dialog
    d'anomalies présente les cartes dans le même ordre que le visualiseur.

    Le `letter_group` permet de grouper les anomalies par sous-deck dans
    les sets multi-decks (L26D : ENM01-99 puis ENS01-99 puis ENX01-99).
    Pour un set classique (LOB, RA05…), letter_group vaut "" partout donc
    le tri se fait sur le numéro seul → comportement strictement identique
    à l'ancien.
    """
    import re
    code = anomalie.get("missing_set_code") or ""
    suffix = code.rsplit("-", 1)[-1] if code else ""
    # Codes langue Konami sur 2 lettres : EN, FR, DE, IT, JP, KR, etc.
    m_lang = re.match(r"^[A-Z]{2}", suffix)
    after_lang = suffix[m_lang.end():] if m_lang else suffix
    m = re.match(r"^([A-Z]*)(\d+)$", after_lang)
    if m:
        letter_group, numero = m.group(1), int(m.group(2))
    else:
        letter_group, numero = "", 0
    return (
        anomalie.get("set_code_prefix") or "",
        letter_group,
        numero,
        anomalie.get("missing_set_rarity") or "",
        anomalie.get("name") or "",
    )


def lire_anomalies(prefix_filtre: str = None) -> list:
    """Retourne la liste des anomalies, triées dans le même ordre que le
    classeur visuel (par numéro de carte extrait, puis rareté, puis nom).

    Ce tri est effectué en Python après la lecture SQL pour utiliser le même
    algorithme que creation_classeur_service._sort_order_from_code() —
    robuste aux set_codes de largeur variable (RA02-EN006, LOB-EN1, etc.).
    """
    _ensure_anomalies_table()
    with sqlite_ctx(CARDINFO_DB) as conn:
        cursor = conn.cursor()
        if prefix_filtre:
            cursor.execute("""
                SELECT id, name, art_a_image_uuid, art_b_image_uuid, art_index,
                       set_code_prefix, missing_set_code, missing_set_rarity,
                       image_url, image_url_small, image_id, corrige
                FROM anomalies
                WHERE set_code_prefix=?
            """, (prefix_filtre,))
        else:
            cursor.execute("""
                SELECT id, name, art_a_image_uuid, art_b_image_uuid, art_index,
                       set_code_prefix, missing_set_code, missing_set_rarity,
                       image_url, image_url_small, image_id, corrige
                FROM anomalies
            """)
        cols = [d[0] for d in cursor.description]
        anomalies = [dict(zip(cols, row)) for row in cursor.fetchall()]

    # Tri Python aligné sur l'ordre visuel du classeur
    anomalies.sort(key=_anomalie_sort_key)
    return anomalies


def lire_prefixes_avec_anomalies() -> list:
    _ensure_anomalies_table()
    with sqlite_ctx(CARDINFO_DB) as conn:
        cursor = conn.cursor()
        cursor.execute("""
            SELECT DISTINCT set_code_prefix FROM anomalies ORDER BY set_code_prefix
        """)
        return [row[0] for row in cursor.fetchall()]


def lister_artworks_alternatifs_pour_carte(
    classeur_code: str, set_code: str
) -> list[dict]:
    """
    Pour une carte identifiée par son set_code dans un classeur, retourne
    tous les artworks connus dans cardinfo.db qui NE SONT PAS encore présents
    dans le classeur pour ce set_code.

    Complémentaire au scan cross-set de scanner_et_stocker_anomalies :
    celui-ci ne détecte que les artworks absents d'un set mais présents dans
    un autre. Cette fonction détecte tous les artworks connus pour la carte
    (via card_images), quelle que soit leur distribution dans les sets.

    Utilisé par dialog_anomalies en mode "Modifier l'artwork" (set_code_filter)
    pour enrichir les résultats du scan cross-set avec les artworks du même slot.

    Retourne des dicts au format "anomalie synthétique" compatibles avec le
    reste du pipeline dialog_anomalies (correction, preview, etc.) :
      - "id" : None  (pas de ligne dans la table anomalies)
      - tous les autres champs attendus par corriger_anomalie

    Retourne [] si cardinfo.db absent, set_code inconnu ou aucun artwork manquant.
    """
    if not os.path.exists(CARDINFO_DB):
        return []

    db_path = os.path.join(CLASSEUR_FOLDER, classeur_code, f"{classeur_code}.db")
    if not os.path.exists(db_path):
        return []

    try:
        # 1) Récupérer card_uuid, name et rarity depuis le classeur pour ce set_code
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("""
                SELECT card_uuid, name, rarity
                FROM cards
                WHERE set_code = ?
                  AND card_uuid IS NOT NULL AND card_uuid != ''
                LIMIT 1
            """, (set_code,))
            row = cursor.fetchone()
            if not row:
                return []
            card_uuid, name, rarity = row

            # IDs déjà présents dans le classeur pour ce set_code
            cursor.execute("""
                SELECT card_image_id FROM cards
                WHERE set_code = ? AND card_image_id IS NOT NULL
            """, (set_code,))
            ids_presents = {r[0] for r in cursor.fetchall()}

        if not card_uuid:
            return []

        # 2) Tous les artworks pour ce card_uuid dans cardinfo.db
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            cursor.execute("""
                SELECT uuid, ygoprodeck_image_id, card_url, art_url
                FROM card_images
                WHERE card_uuid = ?
                ORDER BY ygoprodeck_image_id
            """, (card_uuid,))
            all_artworks = cursor.fetchall()

        # 3) Filtrer ceux déjà présents dans le classeur
        prefix = set_code.split("-")[0] if "-" in set_code else set_code
        propositions: list[dict] = []
        for idx, (img_uuid, img_id, card_url, art_url) in enumerate(all_artworks, start=1):
            if img_id in ids_presents:
                continue
            # img_id requis pour l'affichage et la correction ;
            # on ignore les artworks sans id (cas très rare).
            if img_id is None:
                continue
            # Id synthétique négatif (-img_id) : unique par artwork (img_id
            # est l'identifiant YGOPRODeck, toujours positif), ne collide jamais
            # avec les ids réels de la table anomalies (auto-increment > 0).
            # Permet à la logique de sélection de distinguer chaque entrée.
            # _marquer_corrige(-img_id, True) exécutera UPDATE ... WHERE id=-img_id
            # qui ne matchera aucune ligne → inoffensif.
            propositions.append({
                "id":                 -int(img_id),
                "name":               name or "",
                "art_a_image_uuid":   "",
                "art_b_image_uuid":   img_uuid or "",
                "art_index":          idx,
                "set_code_prefix":    prefix,
                "missing_set_code":   set_code,
                "missing_set_rarity": rarity or "",
                "image_url":          card_url or "",
                "image_url_small":    art_url or "",
                "image_id":           img_id,
                "corrige":            0,
            })

        return propositions

    except Exception as e:
        log.warning(f"lister_artworks_alternatifs_pour_carte"
              f"({classeur_code!r}, {set_code!r}): {e}")
        return []


# ─────────────────────────────────────────────────────────────────────────────
# 5. Correction
# ─────────────────────────────────────────────────────────────────────────────

def corriger_anomalie(anomalie: dict) -> tuple:
    """
    Corrige une anomalie : insère la ligne Art B dans le classeur.
    Retourne (nb_lignes_insérées, classeur_touché_ou_None).
    """
    prefix    = anomalie["set_code_prefix"]
    name      = anomalie["name"]
    set_code  = anomalie["missing_set_code"]
    rarity    = anomalie["missing_set_rarity"]
    new_url   = anomalie.get("image_url", "")
    new_id    = anomalie.get("image_id")

    db_path = os.path.join(CLASSEUR_FOLDER, prefix, f"{prefix}.db")
    if not os.path.exists(db_path):
        return 0, None

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()

            # Art A doit exister dans le classeur
            cursor.execute("""
                SELECT rowid FROM cards
                WHERE name=? AND set_code=? AND rarity=? LIMIT 1
            """, (name, set_code, rarity))
            if not cursor.fetchone():
                # Pour les anomalies réelles (id > 0), marquer corrigé en DB.
                # Pour les synthétiques (id < 0), l'UPDATE est sans effet (no-op).
                _marquer_corrige(anomalie["id"], True)
                return 0, None

            # Art B déjà présent ?
            if new_id:
                cursor.execute("""
                    SELECT rowid FROM cards
                    WHERE name=? AND set_code=? AND rarity=? AND card_image_id=?
                """, (name, set_code, rarity, new_id))
            elif new_url:
                cursor.execute("""
                    SELECT rowid FROM cards
                    WHERE name=? AND set_code=? AND rarity=? AND card_image_url=?
                """, (name, set_code, rarity, new_url))
            else:
                return 0, None

            if cursor.fetchone():
                # Déjà présent : marquer corrigé si anomalie réelle (id > 0).
                _marquer_corrige(anomalie["id"], True)
                return 0, None

            # Copie l'Art A comme base pour l'Art B (récupère TOUTES les
            # colonnes nécessaires pour que la nouvelle ligne soit cohérente
            # avec les autres cartes du classeur — sort_order compris, pour
            # que le tri (sort_order, rarity) place l'Art B juste à côté
            # de l'Art A dans le visualiseur).
            cursor.execute("""
                SELECT card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                       set_name, name, name_fr, card_image_url, card_image_small,
                       card_image_id, sort_order,
                       card_type, atk, def_val, level, attribute, race
                FROM cards
                WHERE name=? AND set_code=? AND rarity=?
                ORDER BY rowid LIMIT 1
            """, (name, set_code, rarity))
            source = cursor.fetchone()
            if not source:
                return 0, None

            (s_card_uuid, s_card_image_uuid, s_set_code, s_rarity, s_rarity_code,
             s_set_name, s_name, s_name_fr, s_image_url, s_image_small,
             s_image_id, s_sort_order,
             s_card_type, s_atk, s_def_val, s_level, s_attribute, s_race) = source

            # Remplace l'image par celle de l'Art B
            final_url = new_url or s_image_url
            final_id  = new_id  if new_id is not None else s_image_id

            # INSERT complet : on réutilise le sort_order de l'Art A pour que
            # l'Art B se place directement à côté dans le tri. Les stats
            # (card_type, atk, def, level, attribute, race) et le set_name
            # viennent de l'Art A (même carte logique, juste artwork
            # alternatif). rarity_code et card_image_small également pour
            # rester cohérent avec les autres lignes du classeur.
            cursor.execute("""
                INSERT INTO cards
                  (card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                   set_name, name, name_fr,
                   card_image_url, card_image_small, card_image_id,
                   sort_order,
                   card_type, atk, def_val, level, attribute, race,
                   possessed, quantite, qualite, is_custom)
                VALUES (?,?,?,?,?, ?,?,?, ?,?,?, ?, ?,?,?,?,?,?, 0,0,NULL,0)
            """, (
                s_card_uuid,
                anomalie.get("art_b_image_uuid", s_card_image_uuid),
                s_set_code, s_rarity, s_rarity_code,
                s_set_name, s_name, s_name_fr,
                final_url, s_image_small, final_id,
                s_sort_order,
                s_card_type, s_atk, s_def_val, s_level, s_attribute, s_race,
            ))
            inserted = cursor.rowcount

        if inserted:
            _marquer_corrige(anomalie["id"], True)
            return inserted, prefix
        return 0, None
    except Exception as e:
        log.warning(f"corriger_anomalie : {e}")
        return 0, None


def corriger_anomalies(anomalies: list) -> tuple:
    total, touches = 0, set()
    for a in anomalies:
        n, cl = corriger_anomalie(a)
        total += n
        if cl:
            touches.add(cl)
    return total, sorted(touches)


def annuler_correction(anomalie: dict) -> bool:
    prefix    = anomalie["set_code_prefix"]
    name      = anomalie["name"]
    set_code  = anomalie["missing_set_code"]
    rarity    = anomalie["missing_set_rarity"]
    new_id    = anomalie.get("image_id")
    new_url   = anomalie.get("image_url", "")

    db_path = os.path.join(CLASSEUR_FOLDER, prefix, f"{prefix}.db")
    if not os.path.exists(db_path):
        return False

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            if new_id:
                cursor.execute("""
                    DELETE FROM cards
                    WHERE name=? AND set_code=? AND rarity=? AND card_image_id=?
                """, (name, set_code, rarity, new_id))
            elif new_url:
                cursor.execute("""
                    DELETE FROM cards
                    WHERE name=? AND set_code=? AND rarity=? AND card_image_url=?
                """, (name, set_code, rarity, new_url))
            else:
                return False
            deleted = cursor.rowcount

        if deleted:
            _marquer_corrige(anomalie["id"], False)
        return bool(deleted)
    except Exception as e:
        log.warning(f"annuler_correction : {e}")
        return False


# ─────────────────────────────────────────────────────────────────────────────
# 6. Hook création classeur
# ─────────────────────────────────────────────────────────────────────────────

def appliquer_overrides_sur_classeur_neuf(db_file: str) -> int:
    prefix    = os.path.splitext(os.path.basename(db_file))[0]
    anomalies = lire_anomalies(prefix_filtre=prefix)
    if not anomalies:
        return 0
    total = 0
    for anomalie in anomalies:
        n, _ = corriger_anomalie(anomalie)
        total += n
    return total


# ─────────────────────────────────────────────────────────────────────────────
# 7. Interne
# ─────────────────────────────────────────────────────────────────────────────

def _marquer_corrige(anomalie_id: int, corrige: bool):
    with sqlite_ctx(CARDINFO_DB) as conn:
        conn.execute(
            "UPDATE anomalies SET corrige=? WHERE id=?",
            (1 if corrige else 0, anomalie_id)
        )

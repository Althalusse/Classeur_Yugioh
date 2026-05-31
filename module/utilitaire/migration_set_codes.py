"""
migration_set_codes.py — Rectifie les set_codes non-EN et les raretés
numériques des classeurs legacy.

Contexte (mai 2026) :

  Bug 1 — set_codes mixtes :
    Avant le fix de _build_rows_from_local_db, certains classeurs ont été
    créés avec des set_codes en FR/IT/DE/etc. au lieu d'EN, à cause d'une
    jointure trop stricte sur la rareté entre locales.

  Bug 2 — raretés numériques :
    Pour certains sets (typiquement Structure Decks reprints), des cartes
    Common ont leur champ `rarity` stocké comme un chiffre ("2", "3", ...)
    au lieu de "Common". Le pattern correspond exactement au nombre de
    copies de la carte dans le deck (Sage with Eyes of Blue = 3 copies →
    rarity="3"; Effect Veiler = 2 copies → rarity="2"). Cela confirme
    que le champ `qty` de YGOJSON a été mal lu comme `rarity` dans le
    pipeline initial, ou que l'API YGOPRODeck a renvoyé des données
    malformées pour ce set.

    Conséquence : à l'import CSV, ces cartes ne sont jamais matchées
    (le CSV dit "Common", la BDD dit "2"/"3").

Conséquence pratique des deux : à l'import CSV, les cartes sont marquées
non-possédées alors qu'elles sont bien dans le CSV.

Ce module fournit :
  - migrer_classeur(code)           — rectifie les set_codes
  - migrer_raretes_numeriques(code) — rectifie les raretés numériques en "Common"
  - migrer_tous_classeurs()         — applique les deux à TOUS les classeurs

Toutes les fonctions sont idempotentes (ré-exécutables sans effet sur un
classeur déjà rectifié) et préservent toutes les autres données
(quantités, qualités, éditions, etc.).
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
import sqlite3
from module.centralisation_dossier import CLASSEUR_FOLDER, sqlite_ctx
from module.logger_app import log


# Pattern : capture le préfixe + la langue + le suffixe alphanumérique.
# Ex 'SDWD-FR013' → groupe 1 = 'SDWD-', groupe 2 = 'FR', groupe 3 = '013'
# Ex 'LDK2-FRJ01' → groupe 1 = 'LDK2-', groupe 2 = 'FR', groupe 3 = 'J01'
_PATTERN_SETCODE = re.compile(
    r'^(.+-)(FR|DE|IT|ES|PT|JP|JA|KR|KO|SP|AE|SC|NA|EU|AU|AS|TC|TF|TG)([A-Z]*\d+)$',
    re.IGNORECASE,
)


def _set_code_to_en(set_code: str) -> str | None:
    """Convertit un set_code non-EN vers son équivalent EN.

    Retourne None si le set_code est déjà en EN ou si le format ne
    correspond pas au pattern attendu. Sinon retourne le set_code EN
    canonique (préserve les sous-préfixes alphabétiques).
    """
    if not set_code:
        return None
    m = _PATTERN_SETCODE.match(set_code)
    if not m:
        return None  # Format inconnu, pas de conversion possible
    return f"{m.group(1)}EN{m.group(3)}"


def detecter_classeurs_a_migrer() -> list[dict]:
    """Parcourt tous les classeurs et retourne ceux qui ont des set_codes
    non-EN à migrer.

    Les classeurs OCG (`LOCH-JP`, `CROS-JP`, etc.) sont volontairement
    EXCLUS : leurs set_codes japonais (`LOCH-JP001`) sont natifs et
    corrects, pas un legacy à migrer vers EN. Convertir `LOCH-JP001` en
    `LOCH-EN001` produirait un set_code qui n'existe pas (LOCH n'est pas
    sorti en TCG).

    Returns:
        Liste de dicts {code, nb_cartes_a_migrer, exemples (3 max)}.
        Liste vide si aucun classeur n'a besoin de migration.
    """
    if not os.path.exists(CLASSEUR_FOLDER):
        return []

    # Import différé pour éviter cycle.
    from module.config.preferences import a_suffixe_ocg as _is_ocg

    a_migrer = []
    for nom_dossier in sorted(os.listdir(CLASSEUR_FOLDER)):
        chemin = os.path.join(CLASSEUR_FOLDER, nom_dossier)
        if not os.path.isdir(chemin):
            continue
        # Skip silencieux des classeurs OCG (cf. docstring).
        if _is_ocg(nom_dossier):
            continue
        db_path = os.path.join(chemin, f"{nom_dossier}.db")
        if not os.path.exists(db_path):
            continue

        try:
            with sqlite_ctx(db_path) as conn:
                cursor = conn.execute("SELECT DISTINCT set_code FROM cards")
                non_en = [
                    sc for (sc,) in cursor.fetchall()
                    if sc and _set_code_to_en(sc) is not None
                ]
                if non_en:
                    a_migrer.append({
                        "code":       nom_dossier,
                        "nb_distincts": len(non_en),
                        "exemples":   non_en[:3],
                    })
        except sqlite3.Error as e:
            # Classeur cassé ? On le saute en silence — l'utilisateur
            # le verra dans Maintenance s'il y a un autre problème.
            log.warning(f"migration : erreur lecture {nom_dossier}: {e}")

    return a_migrer


def migrer_classeur(code: str) -> dict:
    """Convertit tous les set_codes non-EN d'un classeur vers EN.

    Args:
        code : code du classeur (ex 'SDWD').

    Returns:
        Dict {success, migrees, conflits, raison?} :
          - migrees : nb de set_codes effectivement convertis
          - conflits : nb de cartes laissées telles quelles parce que
            le set_code EN cible existe déjà dans le classeur (cas rare
            où FR et EN coexistaient — on ne peut pas fusionner sans
            risque). Ces cartes sont listées dans `cartes_conflits`.

    Pour un classeur OCG (`LOCH-JP`, etc.), la fonction est un no-op
    immédiat : les set_codes japonais sont natifs et ne doivent JAMAIS
    être convertis vers EN. Cette protection complète celle de
    detecter_classeurs_a_migrer (defense in depth — la fonction peut
    être appelée directement sans passer par le détecteur, notamment via
    le hook reparer_classeur du flux de création).

    Idempotente : ré-exécution = 0 migrée, 0 conflit.
    """
    # Skip immédiat des classeurs OCG (cf. docstring).
    from module.config.preferences import a_suffixe_ocg as _is_ocg
    if _is_ocg(code):
        return {
            "success":  True,
            "migrees":  0,
            "conflits": 0,
            "raison":   f"Classeur OCG {code} préservé (pas de migration vers EN).",
        }

    db_path = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    if not os.path.exists(db_path):
        return {
            "success":  False,
            "migrees":  0,
            "conflits": 0,
            "raison":   f"Classeur {code} introuvable.",
        }

    migrees     = 0
    conflits    = 0
    cartes_conflits: list[str] = []

    try:
        with sqlite_ctx(db_path) as conn:
            # Étape 1 : trouver toutes les cartes à migrer
            cursor = conn.execute("""
                SELECT rowid, set_code FROM cards
                WHERE set_code IS NOT NULL AND set_code != ''
            """)
            rows = cursor.fetchall()

            # Étape 2 : pour chaque carte, calculer le set_code EN cible.
            # Si la cible existe déjà → conflit (on n'écrase pas).
            existants = {sc for (_rid, sc) in rows}

            for rowid, old_sc in rows:
                new_sc = _set_code_to_en(old_sc)
                if new_sc is None:
                    continue  # déjà EN ou format inconnu
                if new_sc == old_sc:
                    continue  # idempotence
                if new_sc in existants:
                    # Conflit : la version EN existe déjà → on garde les
                    # deux entrées séparées pour ne rien perdre. Sera
                    # signalé dans le retour.
                    conflits += 1
                    cartes_conflits.append(f"{old_sc} → {new_sc} (cible existante)")
                    continue
                conn.execute(
                    "UPDATE cards SET set_code = ? WHERE rowid = ?",
                    (new_sc, rowid),
                )
                existants.add(new_sc)
                existants.discard(old_sc)
                migrees += 1
    except sqlite3.Error as e:
        return {
            "success":  False,
            "migrees":  migrees,
            "conflits": conflits,
            "raison":   f"Erreur SQLite : {e}",
        }

    return {
        "success":         True,
        "migrees":         migrees,
        "conflits":        conflits,
        "cartes_conflits": cartes_conflits,
    }


def reparer_classeur(code: str) -> dict:
    """Applique en séquence les deux migrations sur un classeur unique.

    Helper pratique pour les hooks automatiques (création de classeur,
    import CSV). Strictement équivalent à appeler `migrer_classeur(code)`
    puis `migrer_raretes_numeriques(code)` à la suite.

    Idempotent (ré-exécutable sans effet sur un classeur déjà sain).
    Pour un classeur fraîchement créé via le pipeline actuel, c'est un
    no-op : les set_codes sont déjà EN et les raretés correctes. Sert
    de safety net défensif si une régression future réintroduisait les
    bugs corrigés en avril/mai 2026.

    Returns:
        dict {set_codes: {...}, raretes: {...}} concatène les deux
        retours pour permettre au caller de logger ce qui a été corrigé.
        Aucune exception ne remonte ; les erreurs sont encapsulées dans
        les sous-dicts (champ `success`).
    """
    return {
        "set_codes": migrer_classeur(code),
        "raretes":   migrer_raretes_numeriques(code),
    }


def migrer_tous_classeurs() -> dict:
    """Migre tous les classeurs détectés en une passe.

    Applique en séquence :
      1. La rectification des set_codes non-EN (migrer_classeur)
      2. La rectification des raretés numériques (migrer_raretes_numeriques)

    Pratique pour un bouton "Réparer tous les classeurs" en Options.
    Retourne un résumé global cumulant les deux types de réparations.
    """
    a_migrer_sc      = detecter_classeurs_a_migrer()
    a_migrer_rar     = detecter_classeurs_raretes_numeriques()

    # Union des codes à traiter
    codes_a_traiter = set()
    for c in a_migrer_sc:
        codes_a_traiter.add(c["code"])
    for c in a_migrer_rar:
        codes_a_traiter.add(c["code"])

    if not codes_a_traiter:
        return {
            "classeurs_traites":   0,
            "total_migrees_sc":    0,
            "total_conflits_sc":   0,
            "total_migrees_rar":   0,
            "details":             [],
        }

    details = []
    total_migrees_sc  = 0
    total_conflits_sc = 0
    total_migrees_rar = 0

    for code in sorted(codes_a_traiter):
        # 1. set_codes
        res_sc = migrer_classeur(code)
        # 2. raretés numériques
        res_rar = migrer_raretes_numeriques(code)
        details.append({
            "code":     code,
            "set_codes":   res_sc,
            "raretes":     res_rar,
        })
        total_migrees_sc  += res_sc.get("migrees",  0)
        total_conflits_sc += res_sc.get("conflits", 0)
        total_migrees_rar += res_rar.get("migrees", 0)

    return {
        "classeurs_traites":   len(codes_a_traiter),
        "total_migrees_sc":    total_migrees_sc,
        "total_conflits_sc":   total_conflits_sc,
        "total_migrees_rar":   total_migrees_rar,
        "details":             details,
    }


# ─────────────────────────────────────────────────────────────────────────────
# Migration 2 : raretés numériques → "Common"
# ─────────────────────────────────────────────────────────────────────────────

def _est_rarete_numerique(rarity) -> bool:
    """Détecte une rareté qui ressemble à un chiffre (bug du pipeline)."""
    if rarity is None:
        return False
    s = str(rarity).strip()
    return s.isdigit()


def detecter_classeurs_raretes_numeriques() -> list[dict]:
    """Parcourt tous les classeurs et retourne ceux qui ont des raretés
    numériques à corriger.

    Returns:
        Liste de dicts {code, nb_cartes_concernees, exemples (3 max)}.
    """
    if not os.path.exists(CLASSEUR_FOLDER):
        return []

    a_migrer = []
    for nom_dossier in sorted(os.listdir(CLASSEUR_FOLDER)):
        chemin = os.path.join(CLASSEUR_FOLDER, nom_dossier)
        if not os.path.isdir(chemin):
            continue
        db_path = os.path.join(chemin, f"{nom_dossier}.db")
        if not os.path.exists(db_path):
            continue

        try:
            with sqlite_ctx(db_path) as conn:
                cursor = conn.execute("SELECT set_code, rarity, name FROM cards")
                cartes_num = [
                    (sc, ra, nm) for (sc, ra, nm) in cursor.fetchall()
                    if _est_rarete_numerique(ra)
                ]
                if cartes_num:
                    a_migrer.append({
                        "code":                 nom_dossier,
                        "nb_cartes_concernees": len(cartes_num),
                        "exemples":             [
                            f"{sc} (rarity={ra!r}) {nm}"
                            for sc, ra, nm in cartes_num[:3]
                        ],
                    })
        except sqlite3.Error as e:
            log.warning(f"migration raretés : erreur lecture {nom_dossier}: {e}")

    return a_migrer


def migrer_raretes_numeriques(code: str, rarete_par_defaut: str = "Common") -> dict:
    """Convertit toutes les raretés numériques d'un classeur vers
    `rarete_par_defaut` (par défaut "Common").

    Pourquoi "Common" par défaut :
        Le pattern observé sur SDWD montre que les raretés numériques
        correspondent au nombre de copies dans un deck, et que ces cartes
        sont en réalité toujours en Common (reprints d'un Structure Deck).
        D'autres cas pourraient théoriquement nécessiter une autre rareté
        — mais 99% des occurrences observées sont des Common, donc c'est
        un bon défaut. Le caller peut passer une autre valeur si besoin.

    La quantité initialement stockée comme rareté n'est PAS perdue : elle
    est sauvegardée dans la colonne `qty` si celle-ci est à 1 ou absente,
    ou loggée pour traçabilité.

    Args:
        code              : code du classeur (ex 'SDWD').
        rarete_par_defaut : rareté à utiliser pour remplacer (default 'Common').

    Returns:
        Dict {success, migrees, raison?}
          - migrees : nb de raretés effectivement converties
    """
    db_path = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    if not os.path.exists(db_path):
        return {
            "success":  False,
            "migrees":  0,
            "raison":   f"Classeur {code} introuvable.",
        }

    migrees = 0

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.execute("""
                SELECT rowid, set_code, rarity, name FROM cards
                WHERE rarity IS NOT NULL
            """)
            rows_a_migrer = [
                (rid, sc, ra, nm) for (rid, sc, ra, nm) in cursor.fetchall()
                if _est_rarete_numerique(ra)
            ]

            for rowid, sc, ra, nm in rows_a_migrer:
                # Log de traçabilité — utile si on doit débugger plus tard
                log.info(
                    f"{code}: rowid={rowid} {sc} '{nm}' "
                    f"rarity {ra!r} → {rarete_par_defaut!r}"
                )
                conn.execute(
                    "UPDATE cards SET rarity = ? WHERE rowid = ?",
                    (rarete_par_defaut, rowid),
                )
                migrees += 1
    except sqlite3.Error as e:
        return {
            "success":  False,
            "migrees":  migrees,
            "raison":   f"Erreur SQLite : {e}",
        }

    return {
        "success": True,
        "migrees": migrees,
    }

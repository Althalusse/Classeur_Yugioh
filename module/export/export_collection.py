"""
export_collection.py — Export CSV de la collection au format Scanflip.

Format de référence : https://scanflip.fr — round-trip exact garanti.

Caractéristiques du format Scanflip :
  - Encodage : UTF-8 avec BOM (utf-8-sig)
  - Séparateur : virgule
  - Sauts de ligne : \\r\\n (CRLF)
  - 11 colonnes :
      Langue, Extension, Code, Nom de la carte, Rareté,
      1st Edition, Unlimited, Limited / Autre, Quantité, N° Artwork, Reprint
  - Édition : encodée par la colonne (1st / Unlimited / Limited) qui contient
    la qualité ; les 2 autres sont vides. Mutuellement exclusives.
  - Artwork : vide pour artwork principal, "1" pour 1er alternatif, "2" pour 2e, etc.
  - Quantité : entier ≥ 1. Une carte non possédée n'apparaît pas dans le CSV.

Différences notables avec l'ancienne version :
  - Convention artwork corrigée : ancien produisait vide/2/3, Scanflip
    attend vide/1/2 (le rang 1 = 1er alternatif, pas le principal).
  - Édition explicite : l'ancien mettait toujours la qualité dans
    "1st Edition", maintenant on respecte la colonne `edition` de la DB.
  - Raretés via la table de référence centralisée (raretes_reference.py)
    au lieu d'une table locale qui collisionnait sur certains codes.
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
import csv
from module.centralisation_dossier import CLASSEUR_FOLDER, sqlite_ctx
from module.gestion_rarete.raretes_reference import name_to_code
from module.logger_app import log

# ─────────────────────────────────────────────────────────────────────────────
# Constantes
# ─────────────────────────────────────────────────────────────────────────────

QUALITE_TO_CODE = {
    "M — Mint": "M", "NM — Near Mint": "NM", "EX — Excellent": "EX",
    "GD — Good": "GD", "PL — Lightly Played": "PL", "PO — Poor": "PO",
    "DM — Damaged": "DM",
    "M": "M", "NM": "NM", "NM+": "NM", "EX": "EX", "GD": "GD",
    "PL": "PL", "PO": "PO", "DM": "DM",
    "Mint": "M", "Near Mint": "NM", "Excellent": "EX", "Good": "GD",
    "Bon": "GD", "Lightly Played": "PL", "Moyen": "PL",
    "Poor": "PO", "joué": "PO", "Damaged": "DM", "Abîmé": "DM",
}

CSV_HEADERS = [
    "Langue", "Extension", "Code", "Nom de la carte", "Rareté",
    "1st Edition", "Unlimited", "Limited / Autre", "Quantité",
    "N° Artwork", "Reprint",
]

LANGUE_LABEL = {"EN": "English", "FR": "Français (France)"}

# Mapping édition DB → colonne CSV qui doit contenir la qualité
EDITION_TO_COLUMN = {
    "1st":       "1st Edition",
    "unlimited": "Unlimited",
    "limited":   "Limited / Autre",
}

# Édition par défaut si la colonne `edition` est NULL (anciennes données ou
# imports legacy qui n'ont pas l'info). On suppose 1st Edition par cohérence
# avec l'ancien comportement de l'export.
DEFAULT_EDITION_COLUMN = "1st Edition"


# ─────────────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────────────

def _qualite_to_code(qualite: str) -> str:
    """Normalise une qualité vers son code court (M, NM, EX, ...)."""
    if not qualite:
        return ""
    code = QUALITE_TO_CODE.get(qualite)
    if code:
        return code
    if " — " in qualite:
        code = QUALITE_TO_CODE.get(qualite.split(" — ")[0].strip(), "")
        if code:
            return code
    return qualite


def _code_to_fr(code: str) -> str:
    """Convertit RA02-EN001 → RA02-FR001 pour l'export FR.

    NE TOUCHE PAS les set_codes OCG (`LOCH-JP001`, `CROS-KR001`, etc.) :
    transformer un code OCG en `-FR` produirait un identifiant qui n'existe
    pas (le set n'est jamais sorti en FR). Pour un classeur OCG-JP, le
    set_code est natif et déjà correct, peu importe la langue d'export.
    """
    if not code:
        return code
    # Détection OCG : import différé pour éviter cycle.
    from module.config.preferences import a_suffixe_ocg as _is_ocg
    if _is_ocg(code):
        return code
    return re.sub(r'-([A-Z]{2,3})(\d)', r'-FR\2', code)


def _rarity_to_scanflip(rarity_full: str) -> str:
    """
    Convertit une rareté nominale (en DB locale, ex 'Secret Rare')
    vers son code Scanflip (ex 'SCR').

    Si la rareté est inconnue de la table de référence, on la conserve
    telle quelle — l'utilisateur saura interpréter dans Scanflip ou
    n'importera tout simplement pas cette ligne.
    """
    code = name_to_code(rarity_full)
    return code if code else (rarity_full or "")


def _edition_to_column(edition) -> str:
    """
    Détermine dans quelle colonne CSV la qualité doit être écrite,
    selon la valeur d'édition stockée en DB.
    """
    if not edition:
        return DEFAULT_EDITION_COLUMN
    return EDITION_TO_COLUMN.get(edition.lower(), DEFAULT_EDITION_COLUMN)


def _get_art_rank(cartes: list) -> dict:
    """
    Calcule le rang d'artwork pour chaque carte au format Scanflip.

    Convention Scanflip :
      - Artwork principal (le plus petit card_image_id) → rang 0 (= cellule vide)
      - 1er alternatif (2e card_image_id par ordre croissant) → rang 1
      - 2e alternatif → rang 2, etc.

    Note : différence avec l'ancien comportement qui produisait rang 1, 2, 3
    (et écrivait vide pour rang 1). La nouvelle convention écrit vide pour
    rang 0, "1" pour rang 1, etc. — ce qui correspond exactement au format
    Scanflip.

    Retourne un dict (name, set_code, card_image_id) → rang (int).
    """
    groups = {}
    for c in cartes:
        key    = (c["name"], c["set_code"])
        img_id = c.get("card_image_id") or 0
        groups.setdefault(key, set()).add(img_id)
    rank_map = {}
    for key, ids in groups.items():
        # Tri croissant, premier = principal (rang 0), suivants = alternatifs
        for rank, img_id in enumerate(sorted(ids)):
            rank_map[(key[0], key[1], img_id)] = rank
    return rank_map


# ─────────────────────────────────────────────────────────────────────────────
# Lecture des cartes possédées
# ─────────────────────────────────────────────────────────────────────────────

def _get_cartes_possedees(classeur: str) -> list:
    """
    Récupère toutes les cartes possédées d'un classeur, avec leur édition.

    Note : pas de filtre `card_image_url IS NOT NULL` — toutes les cartes
    possédées doivent être exportées, même celles sans image (cf. ticket M4
    de l'audit). Une carte sans image reste une carte de la collection.
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.exists(db_path):
        return []

    cartes = []
    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("PRAGMA table_info(cards)")
            cols = {c[1] for c in cursor.fetchall()}

            quantite_col = "quantite" if "quantite" in cols else "1"
            qualite_col  = "qualite"  if "qualite"  in cols else "NULL"
            name_fr_col  = "name_fr"  if "name_fr"  in cols else "name"
            edition_col  = "edition"  if "edition"  in cols else "NULL"

            cursor.execute(f"""
                SELECT name, set_code, rarity, set_name,
                       card_image_id,
                       {quantite_col} AS quantite,
                       {qualite_col}  AS qualite,
                       {name_fr_col}  AS name_fr,
                       {edition_col}  AS edition
                FROM cards
                WHERE possessed = 1
                ORDER BY set_code, rarity
            """)
            for row in cursor.fetchall():
                name, code, rarity, set_name, img_id, qty, qualite, name_fr, edition = row
                cartes.append({
                    "name":          name or "",
                    "name_fr":       name_fr or name or "",
                    "set_code":      code or "",
                    "rarity":        rarity or "",
                    "set_name":      set_name or "",
                    "card_image_id": img_id,
                    "quantite":      max(1, qty if qty else 1),
                    "qualite":       qualite or "",
                    "edition":       edition,  # peut être None
                    "classeur":      classeur,
                })
    except Exception as e:
        log.warning(f"export get_cartes_possedees : {e}")
    return cartes


def get_classeurs_disponibles() -> list:
    """Liste les classeurs disponibles (dossiers dans CLASSEUR_FOLDER)."""
    if not os.path.exists(CLASSEUR_FOLDER):
        return []
    return sorted([
        d for d in os.listdir(CLASSEUR_FOLDER)
        if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))
    ])


# ─────────────────────────────────────────────────────────────────────────────
# Construction des lignes CSV
# ─────────────────────────────────────────────────────────────────────────────

def _build_rows(cartes: list, langue: str) -> list:
    """
    Transforme la liste de cartes possédées en lignes prêtes pour le DictWriter.
    Format strictement aligné sur Scanflip.
    """
    art_rank = _get_art_rank(cartes)
    rows = []

    for c in cartes:
        # Code carte adapté à la langue (RA02-EN001 → RA02-FR001 pour FR)
        code_export = _code_to_fr(c["set_code"]) if langue == "FR" else c["set_code"]

        # Rareté → code Scanflip
        rarity_code = _rarity_to_scanflip(c["rarity"])

        # Artwork : rang 0 = "", rang 1 = "1", rang 2 = "2", etc.
        img_id  = c.get("card_image_id") or 0
        rank    = art_rank.get((c["name"], c["set_code"], img_id), 0)
        art_str = str(rank) if rank > 0 else ""

        # Qualité normalisée (M, NM, EX...)
        qualite_code = _qualite_to_code(c["qualite"])

        # Édition → quelle colonne reçoit la qualité
        edition_col = _edition_to_column(c.get("edition"))

        # Nom dans la langue d'export
        if langue == "EN":
            nom = c["name"]
        else:
            nom = c.get("name_fr") or ""

        # Construction de la ligne avec UNE seule colonne d'édition remplie
        row = {
            "Langue":           LANGUE_LABEL[langue],
            "Extension":        c["classeur"],
            "Code":             code_export,
            "Nom de la carte":  nom,
            "Rareté":           rarity_code,
            "1st Edition":      "",
            "Unlimited":        "",
            "Limited / Autre":  "",
            "Quantité":         str(c["quantite"]),
            "N° Artwork":       art_str,
            "Reprint":          "",
        }
        # Place la qualité dans la colonne correspondant à l'édition
        row[edition_col] = qualite_code
        rows.append(row)
    return rows


# ─────────────────────────────────────────────────────────────────────────────
# API publique
# ─────────────────────────────────────────────────────────────────────────────

def exporter_csv(chemin_fichier: str, langue: str = "EN", classeur: str = None) -> dict:
    """
    Exporte la collection (un classeur ou tous) vers un fichier CSV au format Scanflip.

    Args:
        chemin_fichier : chemin de sortie du CSV.
        langue : "EN" ou "FR" — détermine le code carte exporté et le nom.
        classeur : code d'un classeur précis (ex "RA02"), ou None pour tous.

    Returns:
        dict avec : total_cartes, classeurs_exportes, chemin.

    Format de sortie strictement compatible Scanflip pour round-trip parfait
    via la fonction d'import correspondante (à venir dans le Lot 2).
    """
    if langue not in ("EN", "FR"):
        raise ValueError(f"Langue invalide : {langue}")

    classeurs  = [classeur] if classeur else get_classeurs_disponibles()
    all_rows   = []
    exportes   = []

    for cl in classeurs:
        cartes = _get_cartes_possedees(cl)
        if not cartes:
            continue
        all_rows.extend(_build_rows(cartes, langue))
        exportes.append(cl)

    # CRLF + UTF-8 BOM = exigences Scanflip
    with open(chemin_fichier, "w", newline="", encoding="utf-8-sig") as f:
        writer = csv.DictWriter(f, fieldnames=CSV_HEADERS, lineterminator="\r\n")
        writer.writeheader()
        writer.writerows(all_rows)

    return {
        "total_cartes":       len(all_rows),
        "classeurs_exportes": exportes,
        "chemin":             chemin_fichier,
    }

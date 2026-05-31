"""
raretes_reference.py — Table de référence centralisée des raretés Yu-Gi-Oh!.

Source : convention Scanflip (https://scanflip.fr) — référence pour le
round-trip CSV import/export.

Structure du dictionnaire principal RARETES :
    code (str) → {
        "fr"    : nom complet français,
        "en"    : nom complet anglais (tel que YGOPRODeck le renvoie),
        "alias" : liste d'autres noms vus dans la nature (synonymes),
    }

Helpers exposés :
    - code_to_name_fr(code)       → nom FR
    - code_to_name_en(code)       → nom EN
    - name_to_code(nom)           → code Scanflip (matching tolérant)
    - all_codes()                 → tous les codes connus
    - is_known_code(code)         → True/False

Note de design — collisions résolues :
    Avant ce module, le code utilisait `RARITY_TO_ABBR` (dans export_collection.py)
    où certaines abréviations collisionnaient (ex: "Secret Rare" → SCR ET
    "Ultra Secret Rare" → SCR). Cette table de référence supprime les
    collisions en respectant strictement les codes Scanflip uniques.
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

# ─────────────────────────────────────────────────────────────────────────────
# Table principale — codes Scanflip
# ─────────────────────────────────────────────────────────────────────────────
#
# Ordre d'apparition basé sur la liste fournie par l'utilisateur (capture
# Scanflip), enrichie avec quelques alias YGOPRODeck pour tolérance d'import.
#
RARETES: dict[str, dict] = {
    # ── Communes & Parallèles ──
    "C":    {"fr": "Commune",                       "en": "Common",
             "alias": ["Short Print", "Super Short Print"]},
    "CPA":  {"fr": "Commune Parallèle",             "en": "Normal Parallel Rare",
             "alias": ["Common Parallel Rare"]},
    "CDT":  {"fr": "Commune Duel Terminal",         "en": "Duel Terminal Normal Parallel Rare",
             "alias": ["Duel Terminal Normal Rare Parallel Rare"]},

    # ── Rares (et déclinaisons couleur) ──
    "R":    {"fr": "Rare",                          "en": "Rare",            "alias": []},
    "RDT":  {"fr": "Rare Duel Terminal",            "en": "Duel Terminal Rare Parallel Rare", "alias": []},
    "RVI":  {"fr": "Rare Violet",                   "en": "Purple Rare",     "alias": []},
    "RVE":  {"fr": "Rare Vert",                     "en": "Green Rare",      "alias": []},
    "RBR":  {"fr": "Rare Bronze",                   "en": "Bronze Rare",     "alias": []},
    "RBL":  {"fr": "Rare Bleu",                     "en": "Blue Rare",       "alias": []},
    "RRO":  {"fr": "Rare Rouge",                    "en": "Red Rare",        "alias": []},
    "RAR":  {"fr": "Rare Argent",                   "en": "Silver Rare",     "alias": []},

    # ── Foils spéciaux ──
    "SFR":  {"fr": "Starfoil Rare",                 "en": "Starfoil Rare",
             "alias": ["Starfoil"]},
    "SDT":  {"fr": "Super Rare Duel Terminal",      "en": "Duel Terminal Super Parallel Rare",
             "alias": ["Super Parallel Rare"]},
    "MO":   {"fr": "Mosaic Rare",                   "en": "Mosaic Rare",     "alias": []},
    "SHA":  {"fr": "Shatterfoil Rare",              "en": "Shatterfoil Rare", "alias": []},

    # ── Super Rare ──
    "SR":   {"fr": "Super Rare",                    "en": "Super Rare",      "alias": []},

    # ── Ultra Rare et déclinaisons ──
    "U":    {"fr": "Ultra Rare",                    "en": "Ultra Rare",      "alias": []},
    "UDT":  {"fr": "Ultra Rare Duel Terminal",      "en": "Duel Terminal Ultra Parallel Rare",
             "alias": ["Ultra Parallel Rare"]},
    "UBL":  {"fr": "Ultra Rare Bleu",               "en": "Blue Ultra Rare", "alias": []},
    "UB":   {"fr": "Ultra Blasonnée",               "en": "Crested Ultra Rare", "alias": []},
    "URO":  {"fr": "Ultra Rare Rouge",              "en": "Red Ultra Rare",  "alias": []},
    "UVI":  {"fr": "Ultra Rare Violet",             "en": "Purple Ultra Rare", "alias": []},
    "UAR":  {"fr": "Ultra Argent",                  "en": "Silver Ultra Rare", "alias": []},
    "UVE":  {"fr": "Ultra Rare Vert",               "en": "Green Ultra Rare", "alias": []},

    # ── Secrètes ──
    "SCR":  {"fr": "Secrète Rare",                  "en": "Secret Rare",     "alias": []},
    "SCRB": {"fr": "Secret Rare Blasonnée",         "en": "Crested Secret Rare", "alias": []},

    # ── Parallèles ──
    "PAR":  {"fr": "Parallèle Rare",                "en": "Parallel Rare",   "alias": []},

    # ── Métaux & exotiques ──
    "PLA":  {"fr": "Platinum Rare",                 "en": "Platinum Rare",   "alias": []},
    "GLD":  {"fr": "Gold Rare",                     "en": "Gold Rare",       "alias": []},
    "GS":   {"fr": "Gold Secrète Rare",             "en": "Gold Secret Rare", "alias": []},
    "COL":  {"fr": "Collector's Rare",              "en": "Collector's Rare", "alias": []},
    "PRG":  {"fr": "Premium Gold Rare",             "en": "Premium Gold Rare", "alias": []},
    "UTR":  {"fr": "Ultimate Rare",                 "en": "Ultimate Rare",   "alias": []},
    "SPL":  {"fr": "Secrète Platinum",              "en": "Platinum Secret Rare", "alias": []},

    # ── Variantes Pharaoh / Prismatic / Starlight ──
    "PHR":  {"fr": "Pharaonique Rare",              "en": "Ultra Rare (Pharaoh's Rare)",
             "alias": ["Pharaoh's Rare"]},
    "PRI":  {"fr": "Prismatique",                   "en": "Prismatic Secret Rare", "alias": []},
    "STR":  {"fr": "Starlight Rare",                "en": "Starlight Rare",  "alias": []},

    # ── 10000 / Quart de Siècle / Ghost ──
    "S10K": {"fr": "Secrète 10000",                 "en": "10000 Secret Rare", "alias": []},
    "GG":   {"fr": "Ghost Gold Rare",               "en": "Ghost/Gold Rare", "alias": []},
    "EXS":  {"fr": "Extra Secrète",                 "en": "Extra Secret Rare",
             "alias": ["Extra Secret"]},
    "RSC":  {"fr": "Remote Secrète Rare",           "en": "Remote Secret Rare", "alias": []},
    "QCR":  {"fr": "Secrète Rare Quart de Siècle",  "en": "Quarter Century Secret Rare", "alias": []},
    "G":    {"fr": "Ghost Rare",                    "en": "Ghost Rare",      "alias": []},
}


# ─────────────────────────────────────────────────────────────────────────────
# Index inverses (construits à l'import)
# ─────────────────────────────────────────────────────────────────────────────
_FR_TO_CODE: dict[str, str] = {}
_EN_TO_CODE: dict[str, str] = {}
_NORMALIZED_TO_CODE: dict[str, str] = {}


def _normalize(s: str) -> str:
    """Normalise pour comparaison tolérante : minuscules, sans espaces, sans accents simples."""
    if not s:
        return ""
    # Suppression accents les plus courants
    table = str.maketrans({
        "é": "e", "è": "e", "ê": "e", "ë": "e",
        "à": "a", "â": "a", "ä": "a",
        "î": "i", "ï": "i",
        "ô": "o", "ö": "o",
        "ù": "u", "û": "u", "ü": "u",
        "ç": "c",
        "É": "E", "È": "E", "Ê": "E",
    })
    return s.translate(table).lower().replace(" ", "").replace("'", "").replace("-", "")


def _build_indexes() -> None:
    """Construit les index inverses au chargement du module."""
    global _FR_TO_CODE, _EN_TO_CODE, _NORMALIZED_TO_CODE
    for code, info in RARETES.items():
        _FR_TO_CODE[info["fr"]] = code
        _EN_TO_CODE[info["en"]] = code
        # Index normalisé pour matching tolérant (FR + EN + alias)
        _NORMALIZED_TO_CODE[_normalize(info["fr"])] = code
        _NORMALIZED_TO_CODE[_normalize(info["en"])] = code
        for alias in info.get("alias", []):
            _NORMALIZED_TO_CODE[_normalize(alias)] = code


_build_indexes()


# ─────────────────────────────────────────────────────────────────────────────
# Raretés « À venir » — registre persistant des raretés inconnues du
# référentiel (saisies manuellement lors d'un ajout de carte custom).
#
# Objectif : garantir UNE forme stockée canonique pour une rareté inconnue
# (sinon deux saisies divergentes cassent le matching set_code+rareté+artwork).
# Ces raretés sont marquées `"a_venir": True` et persistées dans
# `BDD_FOLDER/raretes_a_venir.json` pour survivre au redémarrage. Elles sont
# fusionnées dans RARETES puis ré-indexées → `name_to_code()` les reconnaît.
# ─────────────────────────────────────────────────────────────────────────────
import json as _json
import os as _os

try:
    from module.centralisation_dossier import BDD_FOLDER as _BDD_FOLDER
    _A_VENIR_FILE = _os.path.join(_BDD_FOLDER, "raretes_a_venir.json")
except Exception:
    _A_VENIR_FILE = None


def _code_a_venir(nom: str) -> str:
    """Code synthétique stable pour une rareté à-venir (préfixe AV:)."""
    return "AV:" + _normalize(nom)


def _charger_raretes_a_venir() -> None:
    """Charge le JSON des raretés à-venir et les fusionne dans RARETES."""
    if not _A_VENIR_FILE or not _os.path.isfile(_A_VENIR_FILE):
        return
    try:
        with open(_A_VENIR_FILE, "r", encoding="utf-8") as f:
            noms = _json.load(f)
        if isinstance(noms, list):
            for nom in noms:
                nom = (nom or "").strip()
                if not nom:
                    continue
                code = _code_a_venir(nom)
                RARETES.setdefault(code, {"fr": nom, "en": nom,
                                          "alias": [], "a_venir": True})
    except Exception:
        # Fichier corrompu : on n'empêche pas le démarrage.
        pass
    _build_indexes()


def _sauver_raretes_a_venir() -> None:
    """Persiste la liste des noms de raretés marquées à-venir."""
    if not _A_VENIR_FILE:
        return
    noms = sorted({info["en"] for info in RARETES.values()
                   if info.get("a_venir")})
    try:
        _os.makedirs(_os.path.dirname(_A_VENIR_FILE), exist_ok=True)
        with open(_A_VENIR_FILE, "w", encoding="utf-8") as f:
            _json.dump(noms, f, indent=2, ensure_ascii=False)
    except Exception:
        pass


_charger_raretes_a_venir()


def enregistrer_rarete_a_venir(nom: str) -> str:
    """Enregistre une rareté inconnue comme « à venir » (persistée + indexée).

    Retourne la forme canonique stockée (le nom nettoyé). Idempotent.
    """
    nom = (nom or "").strip()
    if not nom:
        return ""
    code = _code_a_venir(nom)
    if code not in RARETES:
        RARETES[code] = {"fr": nom, "en": nom, "alias": [], "a_venir": True}
        _build_indexes()
        _sauver_raretes_a_venir()
    return nom


def est_a_venir(name_ou_code: str) -> bool:
    """True si la rareté (nom OU code) est marquée « à venir »."""
    if not name_ou_code:
        return False
    info = RARETES.get(name_ou_code)
    if info is None:
        code = name_to_code(name_ou_code)
        info = RARETES.get(code) if code else None
    return bool(info and info.get("a_venir"))


def lister_raretes_a_venir() -> list[str]:
    """Liste des noms (EN) des raretés actuellement marquées « à venir »."""
    return sorted({info["en"] for info in RARETES.values()
                   if info.get("a_venir")})


def lister_raretes_reference() -> list[str]:
    """Liste de tous les noms EN du référentiel (pour autocomplétion UI)."""
    return sorted({info["en"] for info in RARETES.values()})


def normaliser_rarete(saisie: str) -> tuple[str, bool]:
    """Normalise une rareté saisie vers sa forme canonique EN.

    - Reconnue (référentiel ou à-venir déjà enregistrée) → (nom_EN, False)
    - Inconnue → enregistrée comme « à venir » (persistée) → (nom_nettoyé, True)
    - Vide → ("", False)

    Le booléen indique qu'une nouvelle rareté « à venir » vient d'être créée.
    """
    saisie = (saisie or "").strip()
    if not saisie:
        return ("", False)
    code = name_to_code(saisie)
    if code:
        return (code_to_name_en(code), False)
    return (enregistrer_rarete_a_venir(saisie), True)

def code_to_name_fr(code: str) -> str:
    """Retourne le nom FR de la rareté, ou le code lui-même si inconnu."""
    info = RARETES.get(code)
    return info["fr"] if info else code


def code_to_name_en(code: str) -> str:
    """Retourne le nom EN de la rareté, ou le code lui-même si inconnu."""
    info = RARETES.get(code)
    return info["en"] if info else code


def name_to_code(name: str) -> str | None:
    """
    Convertit un nom de rareté (FR, EN, ou alias) vers le code Scanflip.

    Matching tolérant :
      1. Correspondance exacte FR
      2. Correspondance exacte EN
      3. Correspondance normalisée (sans accents, espaces, casse)

    Retourne None si aucun match — l'appelant décide de la stratégie de
    fallback (typiquement : utiliser le nom brut comme rareté).
    """
    if not name:
        return None
    if name in _FR_TO_CODE:
        return _FR_TO_CODE[name]
    if name in _EN_TO_CODE:
        return _EN_TO_CODE[name]
    return _NORMALIZED_TO_CODE.get(_normalize(name))



def is_known_code(code: str) -> bool:
    """True si le code est dans la table de référence."""
    return code in RARETES


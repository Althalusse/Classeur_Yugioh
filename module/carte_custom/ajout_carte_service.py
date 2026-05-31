"""
ajout_carte_service.py — Ajout MANUEL d'une carte à un classeur.

Couvre deux cas où cardinfo.db est incomplet :
  - Cas 1 : artwork manquant d'une carte connue (la carte existe déjà dans
    le classeur, on ajoute une variante d'artwork — éventuellement sans
    image si l'artwork n'est pas connu).
  - Cas 2 : carte absente du set dans l'API (trop récente) — on résout la
    carte par NOM dans cardinfo.db (donc ses stats + artworks, hérités d'un
    autre set), puis on l'« accommode » au set cible (set_code/rareté).

Principe clé : les métadonnées qui comptent (nom, stats, artworks) sont
stockées PAR CARTE (`card_uuid`) dans cardinfo.db, indépendamment du set.
Seuls les champs « print » (set_code, set_name, rareté, sort_order) sont
propres au set cible et fournis par l'utilisateur.

Toute carte ajoutée par ce service est marquée `is_custom = 1`.
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

from module.centralisation_dossier import CARDINFO_DB, CLASSEUR_FOLDER, sqlite_ctx
from module.logger_app import log
from module.gestion_rarete.raretes_reference import (
    normaliser_rarete, name_to_code,
)


def _norm_recherche(s) -> str:
    """Normalise un texte pour recherche tolérante.

    Minuscules + suppression des accents courants + suppression des
    séparateurs (espaces, tirets, apostrophes, points, etc.). Utilisée
    des DEUX côtés (terme saisi ET noms en base) pour que « blue eyes »
    matche « Blue-Eyes ». Robuste si `s` est NULL (None → "").
    """
    if not s:
        return ""
    table = str.maketrans({
        "é": "e", "è": "e", "ê": "e", "ë": "e",
        "à": "a", "â": "a", "ä": "a",
        "î": "i", "ï": "i", "ô": "o", "ö": "o",
        "ù": "u", "û": "u", "ü": "u", "ç": "c",
    })
    out = s.translate(table).lower()
    for ch in " -’'.,:/&!?()":
        out = out.replace(ch, "")
    return out


# ─────────────────────────────────────────────────────────────────────────────
# Recherche dans cardinfo.db (lecture seule)
# ─────────────────────────────────────────────────────────────────────────────

def rechercher_cartes_par_nom(terme: str, limit: int = 25) -> list[dict]:
    """Cherche des cartes par nom (EN ou FR) dans cardinfo.db.

    Retourne une liste de dicts {card_uuid, name_en, name_fr, card_type},
    triée par nom EN.

    Recherche tolérante : le terme est découpé en mots, et chaque mot doit
    apparaître dans le nom (EN ou FR), indépendamment de l'ordre, de la casse
    et des séparateurs (tiret, espace, apostrophe, point). Ainsi « blue eyes »
    matche « Blue-Eyes White Dragon », et « dragon blanc » matche
    « Dragon Blanc aux Yeux Bleus ».
    """
    terme = (terme or "").strip()
    if not terme or not os.path.isfile(CARDINFO_DB):
        return []

    # Découpe en mots significatifs (sépare sur espaces/tirets/ponctuation).
    import re as _re
    mots = [m for m in _re.split(r"[\s\-’'.,:/]+", terme.lower()) if m]
    if not mots:
        return []

    out: dict[str, dict] = {}
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            conn.create_function("normtxt", 1, _norm_recherche)
            cur = conn.cursor()
            # Chaque mot → une condition LIKE sur le nom normalisé (EN ou FR).
            # normtxt() retire accents/séparateurs et met en minuscules des
            # deux côtés → matching réellement tolérant.
            clauses = []
            params: list = []
            for mot in mots:
                clauses.append(
                    "(normtxt(ct_en.name) LIKE ? OR normtxt(ct_fr.name) LIKE ?)"
                )
                motif = f"%{_norm_recherche(mot)}%"
                params.extend([motif, motif])
            where = " AND ".join(clauses)
            params.append(limit)
            cur.execute(f"""
                SELECT c.uuid,
                       ct_en.name AS name_en,
                       ct_fr.name AS name_fr,
                       c.card_type
                FROM cards c
                LEFT JOIN card_texts ct_en
                       ON ct_en.card_uuid = c.uuid AND ct_en.language = 'en'
                LEFT JOIN card_texts ct_fr
                       ON ct_fr.card_uuid = c.uuid AND ct_fr.language = 'fr'
                WHERE {where}
                LIMIT ?
            """, params)
            for uuid, name_en, name_fr, card_type in cur.fetchall():
                if uuid in out:
                    continue
                out[uuid] = {
                    "card_uuid": uuid,
                    "name_en":   name_en or "",
                    "name_fr":   name_fr or "",
                    "card_type": card_type or "",
                }

            # Fallback FLOU : si le LIKE par mots ne donne rien (faute de
            # frappe, terme approximatif), on classe les noms par similarité
            # et on suggère les plus proches. difflib = stdlib, zéro dépendance.
            if not out:
                cibles = _suggestions_floues(conn, terme, limit)
                for d in cibles:
                    out[d["card_uuid"]] = d
    except Exception as e:
        log.warning(f"rechercher_cartes_par_nom({terme!r}) : {e}")
        return []
    return sorted(out.values(), key=lambda d: (d["name_en"] or d["name_fr"]).lower())


def _suggestions_floues(conn, terme: str, limit: int) -> list[dict]:
    """Suggère les cartes dont le nom RESSEMBLE le plus au terme saisi.

    Activé en dernier recours (le LIKE par mots n'a rien trouvé). Utilise
    difflib.SequenceMatcher sur les noms normalisés (EN et FR). On garde le
    meilleur ratio des deux langues par carte, puis on trie décroissant.
    Seuil minimal pour éviter les suggestions absurdes.
    """
    import difflib
    norm_terme = _norm_recherche(terme)
    if len(norm_terme) < 3:
        return []
    cur = conn.cursor()
    cur.execute("""
        SELECT c.uuid, ct_en.name, ct_fr.name, c.card_type
        FROM cards c
        LEFT JOIN card_texts ct_en
               ON ct_en.card_uuid = c.uuid AND ct_en.language = 'en'
        LEFT JOIN card_texts ct_fr
               ON ct_fr.card_uuid = c.uuid AND ct_fr.language = 'fr'
    """)
    scored: list[tuple[float, dict]] = []
    sm = difflib.SequenceMatcher()
    sm.set_seq2(norm_terme)
    for uuid, name_en, name_fr, card_type in cur.fetchall():
        best = 0.0
        for nm in (name_en, name_fr):
            if not nm:
                continue
            n = _norm_recherche(nm)
            if not n:
                continue
            # Bonus fort si le terme est contenu (sous-chaîne) ; sinon ratio.
            if norm_terme in n:
                r = 0.9 + 0.1 * (len(norm_terme) / len(n))
            else:
                sm.set_seq1(n)
                r = sm.ratio()
            if r > best:
                best = r
        if best >= 0.45:
            scored.append((best, {
                "card_uuid": uuid,
                "name_en":   name_en or "",
                "name_fr":   name_fr or "",
                "card_type": card_type or "",
            }))
    scored.sort(key=lambda t: t[0], reverse=True)
    return [d for _, d in scored[:limit]]


def get_artworks_carte(card_uuid: str) -> list[dict]:
    """Liste les artworks connus d'une carte.

    Retourne [{card_image_uuid, card_image_id, card_image_url, card_image_small}]
    (mapping aligné sur les colonnes du classeur : card_image_url = card_url,
    card_image_small = art_url).
    """
    if not card_uuid or not os.path.isfile(CARDINFO_DB):
        return []
    arts: list[dict] = []
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cur = conn.cursor()
            cur.execute("""
                SELECT uuid, ygoprodeck_image_id, card_url, art_url
                FROM card_images
                WHERE card_uuid = ?
                ORDER BY ygoprodeck_image_id
            """, (card_uuid,))
            for img_uuid, img_id, card_url, art_url in cur.fetchall():
                arts.append({
                    "card_image_uuid":  img_uuid or "",
                    "card_image_id":    img_id,
                    "card_image_url":   card_url or "",
                    "card_image_small": art_url or "",
                })
    except Exception as e:
        log.warning(f"get_artworks_carte({card_uuid!r}) : {e}")
    return arts


def get_metadonnees_carte(card_uuid: str) -> dict | None:
    """Métadonnées (stats + noms) d'une carte. None si introuvable."""
    if not card_uuid or not os.path.isfile(CARDINFO_DB):
        return None
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cur = conn.cursor()
            cur.execute("""
                SELECT c.card_type, c.atk, c.def, c.level, c.attribute, c.race,
                       ct_en.name, ct_fr.name
                FROM cards c
                LEFT JOIN card_texts ct_en
                       ON ct_en.card_uuid = c.uuid AND ct_en.language = 'en'
                LEFT JOIN card_texts ct_fr
                       ON ct_fr.card_uuid = c.uuid AND ct_fr.language = 'fr'
                WHERE c.uuid = ?
            """, (card_uuid,))
            r = cur.fetchone()
            if not r:
                return None
            return {
                "card_type": r[0] or "",
                "atk":       r[1],
                "def_val":   r[2],
                "level":     r[3],
                "attribute": r[4] or "",
                "race":      r[5] or "",
                "name":      r[6] or "",
                "name_fr":   r[7] or "",
            }
    except Exception as e:
        log.warning(f"get_metadonnees_carte({card_uuid!r}) : {e}")
        return None


def resoudre_set_name(code_set: str) -> str:
    """Résout le nom du set cible depuis cardinfo.db via le préfixe du set_code.

    Ex: 'RA05-EN015' → préfixe 'RA05' → sets.name_en. Retourne '' si inconnu.
    """
    code_set = (code_set or "").strip().upper()
    if not code_set or not os.path.isfile(CARDINFO_DB):
        return ""
    # Préfixe = partie avant le 1er '-' (ou tout si pas de tiret)
    prefixe = code_set.split("-", 1)[0]
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cur = conn.cursor()
            cur.execute("""
                SELECT s.name_en
                FROM sets s
                JOIN set_locales sl ON sl.set_uuid = s.uuid
                WHERE sl.prefix = ? OR sl.prefix LIKE ? || '-%'
                LIMIT 1
            """, (prefixe, prefixe))
            r = cur.fetchone()
            return (r[0] or "") if r else ""
    except Exception as e:
        log.warning(f"resoudre_set_name({code_set!r}) : {e}")
        return ""


# ─────────────────────────────────────────────────────────────────────────────
# Insertion dans le classeur (écriture)
# ─────────────────────────────────────────────────────────────────────────────

def ajouter_carte_au_classeur(
    classeur: str,
    set_code: str,
    rarete_saisie: str,
    *,
    card_uuid: str | None = None,
    artwork: dict | None = None,
    nom_manuel: str = "",
    possessed: int = 0,
    quantite: int = 0,
) -> tuple[bool, str]:
    """Insère une carte `is_custom = 1` dans le classeur.

    Args:
        classeur       : code du classeur (= dossier, ex 'RA05', 'LOCR-JP')
        set_code       : code complet de la carte dans le set cible
                         (ex 'RA05-EN015', 'LOCR-JP001')
        rarete_saisie  : rareté tapée par l'utilisateur (normalisée ici)
        card_uuid      : si fourni, métadonnées + nom tirés de cardinfo.db.
                         Sinon, carte 100% manuelle (nom_manuel requis).
        artwork        : dict {card_image_uuid, card_image_id, card_image_url,
                         card_image_small} choisi par l'utilisateur. None →
                         pas d'image (placeholder, ex artwork inconnu).
        possessed/quantite : possession initiale (défaut 0/0).

    Returns:
        (ok: bool, message: str)
    """
    classeur = (classeur or "").strip()
    set_code = (set_code or "").strip()
    if not classeur or not set_code:
        return False, "Classeur ou set_code manquant."

    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return False, f"Base du classeur {classeur} introuvable."

    # Rareté : normalisation obligatoire (forme canonique cohérente).
    rarity, est_nouvelle = normaliser_rarete(rarete_saisie)
    if not rarity:
        return False, "Rareté manquante."
    rarity_code = name_to_code(rarity) or ""

    # Métadonnées : depuis cardinfo via card_uuid, sinon saisie manuelle.
    meta = get_metadonnees_carte(card_uuid) if card_uuid else None
    if meta:
        name    = meta["name"]
        name_fr = meta["name_fr"]
        card_type = meta["card_type"]
        atk, def_val = meta["atk"], meta["def_val"]
        level, attribute, race = meta["level"], meta["attribute"], meta["race"]
    else:
        if not nom_manuel.strip():
            return False, "Nom de carte requis (aucune carte cardinfo sélectionnée)."
        name    = nom_manuel.strip()
        name_fr = ""
        card_type = ""
        atk = def_val = level = None
        attribute = race = ""

    art = artwork or {}
    card_image_uuid  = art.get("card_image_uuid") or ""
    card_image_id    = art.get("card_image_id")
    card_image_url   = art.get("card_image_url") or ""
    card_image_small = art.get("card_image_small") or ""

    set_name = resoudre_set_name(set_code) or set_code

    try:
        with sqlite_ctx(db_path) as conn:
            cur = conn.cursor()

            # Anti-doublon : même (set_code, rareté, card_image_id) déjà présent ?
            if card_image_id is not None:
                cur.execute("""
                    SELECT rowid FROM cards
                    WHERE set_code=? AND rarity=? AND card_image_id=? LIMIT 1
                """, (set_code, rarity, card_image_id))
                if cur.fetchone():
                    return False, "Cette carte (même artwork) est déjà dans le classeur."

            # sort_order : on place à la fin (renumérotation fine laissée à
            # un éventuel re-tri ultérieur).
            cur.execute("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM cards")
            sort_order = cur.fetchone()[0]

            cur.execute("""
                INSERT INTO cards
                  (card_uuid, card_image_uuid, card_image_id,
                   set_code, rarity, rarity_code, set_name,
                   name, name_fr,
                   card_image_url, card_image_small,
                   sort_order,
                   card_type, atk, def_val, level, attribute, race,
                   possessed, quantite, qualite, edition, is_custom)
                VALUES (?,?,?, ?,?,?,?, ?,?, ?,?, ?, ?,?,?,?,?,?, ?,?,NULL,NULL,1)
            """, (
                card_uuid or "", card_image_uuid, card_image_id,
                set_code, rarity, rarity_code, set_name,
                name, name_fr,
                card_image_url, card_image_small,
                sort_order,
                card_type, atk, def_val, level, attribute, race,
                1 if (possessed or quantite) else 0,
                max(0, int(quantite or 0)),
            ))
    except Exception as e:
        log.warning(f"ajouter_carte_au_classeur({classeur}, {set_code}) : {e}")
        return False, f"Erreur d'insertion : {e}"

    msg = f"Carte ajoutée ({set_code} · {rarity})."
    if est_nouvelle:
        msg += f" Rareté « {rarity} » enregistrée comme « À venir »."
    log.info(f"Ajout carte custom : {classeur} / {set_code} / {rarity}"
             f"{' [rareté à venir]' if est_nouvelle else ''}")
    return True, msg


def supprimer_carte_custom(classeur: str, rowid: int) -> tuple[bool, str]:
    """Supprime une carte AJOUTÉE MANUELLEMENT (is_custom=1) du classeur.

    Sécurité : refuse de supprimer une carte du catalogue (is_custom=0 ou
    NULL) pour éviter qu'un utilisateur n'altère le contenu officiel du set
    par erreur. Seuls les ajouts manuels sont supprimables ici.

    Returns: (ok, message)
    """
    classeur = (classeur or "").strip()
    if not classeur or rowid is None:
        return False, "Paramètres manquants."

    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return False, f"Base du classeur {classeur} introuvable."

    try:
        with sqlite_ctx(db_path) as conn:
            cur = conn.cursor()
            cur.execute("PRAGMA table_info(cards)")
            cols = {r[1] for r in cur.fetchall()}
            if "is_custom" not in cols:
                return False, "Ce classeur ne gère pas les cartes manuelles."

            cur.execute(
                "SELECT is_custom, name FROM cards WHERE rowid = ?", (rowid,)
            )
            r = cur.fetchone()
            if not r:
                return False, "Carte introuvable (déjà supprimée ?)."
            if not r[0]:
                return False, ("Cette carte provient du catalogue et ne peut "
                               "pas être supprimée ici (seuls les ajouts "
                               "manuels le peuvent).")
            nom = r[1] or "carte"
            cur.execute("DELETE FROM cards WHERE rowid = ? AND is_custom = 1",
                        (rowid,))
    except Exception as e:
        log.warning(f"supprimer_carte_custom({classeur}, {rowid}) : {e}")
        return False, f"Erreur de suppression : {e}"

    log.info(f"Suppression carte custom : {classeur} / rowid {rowid}")
    return True, f"Carte « {nom} » supprimée."

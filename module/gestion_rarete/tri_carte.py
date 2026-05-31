"""
tri_carte.py — Tri des cartes d'un classeur selon préférences utilisateur.

L'utilisateur peut réordonner par drag & drop 3 critères dans l'onglet
Options : "numero" (extrait du set_code), "rarete" (via priorités),
"artwork" (A avant B via card_image_id). Ce module applique l'ordre choisi.

Deux stratégies selon la génération du classeur :
  G2 (YGOPRODeck API) : sort_order présent (encode déjà numéro + rareté)
  G1 (YGOJSON)        : sort_order absent — on calcule tout

Dans les deux cas, l'ordre des critères préféré par l'utilisateur est
appliqué au-dessus.
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

import re

from module.gestion_rarete.gestion_rarete_service import load_rarity_priorities
from module.config.preferences import get_ordre_tri


def _extract_numero(set_code: str) -> tuple:
    """Extrait une clé de tri (letter_group, number) du set_code complet.

    Permet de grouper les cartes par sous-deck dans les sets multi-decks
    où plusieurs decks partagent le même préfixe de classeur (ex L26D :
    L26D-ENM01..M99 / L26D-ENS01..S99 / L26D-ENX01..X99). Avec ce tri,
    on obtient l'ordre groupé naturel :
        ENM01, ENM02, …, ENM99,
        ENS01, ENS02, …, ENS99,
        ENX01, ENX02, …, ENX99
    au lieu de l'interleaving (ENM01, ENS01, ENX01, ENM02, …) qui
    résultait du tri par numéro seul.

    Le préfixe langue (EN, FR, DE, IT, JP, KR, SP, PT, AE — toujours
    2 lettres en standard Konami) est retiré avant extraction.

    Exemples :
      "L26D-ENM01" → ("M",  1)
      "L26D-ENS03" → ("S",  3)
      "RA02-EN006" → ("",   6)
      "LOB-EN001"  → ("",   1)
      "SS01-ENA01" → ("A",  1)
      "MGED-ENB17" → ("B", 17)
      ""           → ("",   0)

    Retourne un tuple Python comparable naturellement. Pour un set
    classique (sans lettre intermédiaire), letter_group est "" donc
    tous les codes ont la même première composante et le tri se fait
    sur le numéro seul → comportement strictement identique à l'ancien.
    """
    if not set_code:
        return ("", 0)
    try:
        suffix = set_code.rsplit("-", 1)[-1]
        # Codes langue Konami : EN, FR, DE, IT, JP, KR, SP, PT, AE — 2 lettres.
        m_lang = re.match(r"^[A-Z]{2}", suffix)
        after_lang = suffix[m_lang.end():] if m_lang else suffix
        m = re.match(r"^([A-Z]*)(\d+)$", after_lang)
        if m:
            return (m.group(1), int(m.group(2)))
        return ("", 0)
    except Exception:
        return ("", 0)


def _compute_art_ranks(cartes: list) -> dict:
    """Pour chaque (name, set_code, extended_art), classe les card_image_id
    par ordre croissant : le plus petit = Art A (rank 0), suivant = Art B, etc.

    L'Overframe (extended_art=1) est traité comme un ARTWORK DISTINCT : il
    forme son propre groupe de rangs, séparé du cadre normal — même quand il
    partage le même card_image_id que la version normale (cas OCG courant).
    """
    groups: dict[tuple, set] = {}
    for c in cartes:
        ext = 1 if c.get("extended_art") else 0
        key = (c.get("name", ""), c.get("set_code", ""), ext)
        img_id = c.get("card_image_id") or 0
        groups.setdefault(key, set()).add(img_id)

    ranks = {}
    for (name, code, ext), ids in groups.items():
        for rank, img_id in enumerate(sorted(ids)):
            ranks[(name, code, ext, img_id)] = rank
    return ranks


def _build_sort_key_function(ordre: list[str], rarity_priorities: dict,
                              art_ranks: dict):
    """Construit dynamiquement la fonction de clé de tri selon l'ordre.

    Chaque critère produit une valeur numérique triable croissante :
      - numero  : numéro extrait du set_code
      - rarete  : priorité de rareté (plus petit = plus commun)
      - artwork : rank de l'artwork (0 = Art A, 1 = Art B)

    Le nom de carte et le set_code sont toujours ajoutés en fin de clé
    comme tri stable pour gérer les cas où les 3 critères sont identiques
    (même carte, même print, même artwork, même rareté → ex: doublons en DB).
    """
    def compute_value(c, critere):
        if critere == "numero":
            return _extract_numero(c.get("set_code", ""))
        if critere == "rarete":
            return rarity_priorities.get(c.get("rarity", ""), 9999)
        if critere == "artwork":
            name = c.get("name", "")
            code = c.get("set_code", "")
            img  = c.get("card_image_id") or 0
            ext  = 1 if c.get("extended_art") else 0
            # Overframe = artwork distinct : on trie d'abord par cadre (normal
            # avant Overframe), puis par rang d'artwork (Art A avant Art B).
            return (ext, art_ranks.get((name, code, ext, img), 0))
        return 0

    def key_fn(c):
        # Tuple dynamique selon l'ordre demandé, puis nom + set_code stable
        return tuple(compute_value(c, crit) for crit in ordre) + (
            c.get("set_code", ""),
            c.get("name", ""),
        )

    return key_fn


def sort_cartes(cartes: list) -> list:
    """Trie les cartes d'un classeur selon l'ordre des critères préféré.

    L'utilisateur choisit via Options ⇒ drag & drop l'ordre parmi :
      - numero  : numéro extrait du set_code
      - rarete  : priorité de rareté (depuis rarity_config.json)
      - artwork : Art A (image_id le plus petit) avant Art B

    Si sort_order est présent (classeur G2 YGOPRODeck), on l'utilise comme
    base pour le critère "numero" (via _extract_numero sur set_code — plus
    fiable que sort_order car après correction d'anomalies, le sort_order
    est recopié depuis l'Art A donc deux cartes peuvent partager le même).
    """
    if not cartes:
        return cartes

    ordre = get_ordre_tri()
    rarity_priorities = load_rarity_priorities()
    art_ranks = _compute_art_ranks(cartes)

    key_fn = _build_sort_key_function(ordre, rarity_priorities, art_ranks)
    return sorted(cartes, key=key_fn)


# ─────────────────────────────────────────────────────────────────────────────
# Filtrage : N raretés par numéro+artwork
# ─────────────────────────────────────────────────────────────────────────────

def filtrer_n_raretes_par_artwork(cartes: list, n: int) -> list:
    """Réduit la liste aux N raretés les plus rares par (set_code, rang_artwork).

    Pour chaque groupe (même numéro de carte, même artwork), on garde
    UNIQUEMENT les `n` versions avec les priorités de rareté les plus
    élevées (cf. rarity_config.json — plus le rang est élevé, plus la rareté
    est rare).

    Cas d'usage : RA02-EN001 existe en 7 raretés. Avec n=3, on garde les
    3 plus rares (Quarter Century Secret, Platinum Secret, Prismatic
    Secret par exemple). Pour RA02-EN001 (Art B), même logique mais sur
    le groupe Art B séparément.

    n=0 → aucun filtre (retourne une copie identique).
    n=1 → comportement de l'ancien `filtrer_une_rarete_par_artwork`.

    PURE : la liste d'entrée n'est pas mutée. L'ordre relatif des cartes
    restantes est préservé (stable).

    Tie-break : en cas d'égalité de priorité, on garde la première carte
    rencontrée (premier vu = gagnant). Évite tout comportement aléatoire
    si deux raretés inconnues partagent la priorité 9999.

    Args:
        cartes : liste de dicts avec au minimum 'set_code', 'rarity',
                 'card_image_id', 'name'.
        n : nombre de raretés à conserver par groupe (≥ 0).

    Returns:
        Nouvelle liste avec au plus N cartes par (set_code, art_rank).
    """
    if not cartes or n <= 0:
        return list(cartes)

    rarity_priorities = load_rarity_priorities()
    art_ranks = _compute_art_ranks(cartes)

    # Index : (set_code, extended_art, art_rank) -> liste de (priorite, idx).
    # extended_art dans la clé => l'Overframe est un artwork distinct : on
    # garde N raretés pour le cadre normal ET N pour l'Overframe (au lieu de
    # mélanger les deux dans un seul groupe, ce qui décalerait la grille).
    groupes: dict[tuple, list[tuple[int, int]]] = {}
    for idx, c in enumerate(cartes):
        set_code = c.get("set_code", "")
        name     = c.get("name", "")
        img_id   = c.get("card_image_id") or 0
        ext      = 1 if c.get("extended_art") else 0
        art_rank = art_ranks.get((name, set_code, ext, img_id), 0)
        priorite = rarity_priorities.get(c.get("rarity", ""), 0)

        cle = (set_code, ext, art_rank)
        groupes.setdefault(cle, []).append((priorite, idx))

    # Sélectionne les N gagnants par groupe : tri par (-priorite, idx_origine)
    # pour avoir les plus rares d'abord, puis tie-break stable sur l'ordre
    # d'apparition (premier vu = gagnant en cas d'égalité).
    gagnants_idx: set[int] = set()
    for cle, items in groupes.items():
        items.sort(key=lambda x: (-x[0], x[1]))   # plus rare d'abord, stable
        for _pri, idx_orig in items[:n]:
            gagnants_idx.add(idx_orig)

    # Reconstitue la liste en respectant l'ordre d'entrée d'origine
    return [c for i, c in enumerate(cartes) if i in gagnants_idx]


# ── Alias rétrocompatibilité (ancien API booléen) ────────────────────────────

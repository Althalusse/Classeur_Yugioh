"""
inventaire_service.py — Couche métier de l'inventaire global des cartes possédées.

Responsabilité unique : agréger, depuis toutes les bases de classeur, les cartes
marquées `possessed = 1`, et exposer les opérations d'écriture (qualité, quantité,
retrait) en restant indépendant de toute logique d'interface.

Pourquoi un identifiant `rowid` ?
─────────────────────────────────
Une même carte (même nom + même set_code + même rareté) peut exister en plusieurs
lignes distinctes dans un classeur lorsqu'il y a des artworks alternatifs
(`extended_art`). Identifier une ligne par (nom, set_code, rareté) serait donc
ambigu. On utilise le `rowid` SQLite — déjà l'identifiant retenu par
`dialog_carte` via `update_quantite_by_rowid` / `update_qualite_by_rowid` — ce qui
garantit que l'inventaire et le visualiseur de classeur agissent sur la MÊME ligne.

Les écritures sont déléguées à `module.carte_posseder.gestion_carte_posseder`
(point d'entrée unique des mises à jour de possession dans le projet).
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

from module.centralisation_dossier import CLASSEUR_FOLDER, sqlite_ctx
from module.config_langue import get_name_column
from module.carte_posseder.gestion_carte_posseder import (
    update_quantite_by_rowid, update_qualite_by_rowid,
)
from module.logger_app import log


def db_path_for(classeur: str) -> str:
    """Chemin de la base d'un classeur donné (`bdd/classeur_creer/<C>/<C>.db`)."""
    return os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")


def get_cartes_possedees() -> list:
    """
    Parcourt TOUS les classeurs et récupère les cartes possédées (possessed = 1).

    Retourne une liste de dicts :
        {
          "rowid":    int,      # identifiant ligne dans la base du classeur
          "classeur": str,      # nom du dossier/préfixe du classeur
          "name":     str,      # nom affichable (FR si dispo + langue FR, sinon EN)
          "set_name": str,
          "set_code": str,
          "rarity":   str,
          "quantite": int,      # >= 1 (une carte possédée a au moins 1 exemplaire)
          "qualite":  str,      # valeur canonique stockée ("" si non renseignée)
          "overframe":     bool, # True si art étendu (extended_art = 1)
          "card_image_id": int|None,
          "n_arts":   int,      # nb d'artworks distincts de l'impression (>=1)
          "art_rank": int,      # rang de l'artwork (1..n_arts), 0 si artwork unique
        }

    La langue d'affichage du nom suit `config_langue.get_name_column()` :
    FR → name_fr avec repli sur name ; EN → name.
    """
    result = []

    if not os.path.exists(CLASSEUR_FOLDER):
        return result

    name_col = get_name_column()

    for classeur in sorted(os.listdir(CLASSEUR_FOLDER)):
        db_path = db_path_for(classeur)
        if not os.path.isdir(os.path.join(CLASSEUR_FOLDER, classeur)):
            continue
        if not os.path.exists(db_path):
            continue
        try:
            with sqlite_ctx(db_path) as conn:
                cursor = conn.cursor()

                # Défensif : certaines bases anciennes pourraient ne pas avoir
                # name_fr / extended_art / card_image_id. On replie pour ne
                # jamais planter.
                cursor.execute("PRAGMA table_info(cards)")
                cols = {row[1] for row in cursor.fetchall()}
                if name_col == "name_fr" and "name_fr" in cols:
                    name_expr = "COALESCE(NULLIF(name_fr, ''), name)"
                else:
                    name_expr = "name"
                ext_expr = "extended_art" if "extended_art" in cols else "0"
                img_expr = "card_image_id" if "card_image_id" in cols else "NULL"

                cursor.execute(f"""
                    SELECT rowid,
                           {name_expr} AS display_name,
                           set_name, set_code, rarity, quantite, qualite,
                           {ext_expr}  AS ext_art,
                           {img_expr}  AS img_id
                    FROM cards
                    WHERE possessed = 1
                """)
                for (rowid, name, set_name, set_code,
                     rarity, quantite, qualite, ext_art, img_id) in cursor.fetchall():
                    qte = quantite if (quantite is not None and quantite > 0) else 1
                    result.append({
                        "rowid":    rowid,
                        "classeur": classeur,
                        "name":     name or "",
                        "set_name": set_name or "",
                        "set_code": set_code or "",
                        "rarity":   rarity or "",
                        "quantite": qte,
                        "qualite":  qualite or "",
                        # Nature de l'impression (pour différencier les doublons)
                        "overframe":     bool(ext_art),
                        "card_image_id": img_id,
                    })
        except Exception as e:
            log.warning(f"inventaire_service.get_cartes_possedees [{classeur}]: {e}")

    _calculer_variantes(result)
    return result


def _calculer_variantes(cartes: list) -> None:
    """
    Renseigne, pour chaque carte, deux champs de différenciation des doublons
    d'une même impression (même nom + set_code + rareté dans un classeur) :

      "n_arts"   : nombre d'artworks distincts de l'impression (>= 1)
      "art_rank" : rang (1..n_arts) de l'artwork de cette ligne, ou 0 si
                   l'impression n'a qu'un seul artwork.

    Un artwork est identifié par (overframe, card_image_id). Le tri est
    déterministe : cadre normal avant Overframe, puis par card_image_id.
    La mise en forme du libellé (« Art 2 », « Overframe ») est faite côté UI
    pour garder ce module indépendant de l'affichage et de l'i18n.
    """
    from collections import defaultdict

    groupes = defaultdict(list)
    for c in cartes:
        groupes[(c["classeur"], c["set_code"], c["rarity"], c["name"])].append(c)

    for rows in groupes.values():
        sigs = []
        for c in rows:
            sig = (c["overframe"], c["card_image_id"])
            if sig not in sigs:
                sigs.append(sig)
        sigs.sort(key=lambda s: (s[0], s[1] if s[1] is not None else -1))
        n_arts = len(sigs)
        for c in rows:
            c["n_arts"] = n_arts
            if n_arts > 1:
                c["art_rank"] = sigs.index((c["overframe"], c["card_image_id"])) + 1
            else:
                c["art_rank"] = 0


def set_qualite(classeur: str, rowid: int, qualite: str) -> bool:
    """
    Met à jour la qualité d'une carte possédée (identifiée par classeur + rowid).
    `qualite` est la valeur CANONIQUE à stocker (ex. "Near Mint", "" pour vide).
    Retourne True si l'écriture a abouti.
    """
    db_path = db_path_for(classeur)
    if not os.path.exists(db_path):
        return False
    try:
        update_qualite_by_rowid(db_path, rowid, qualite or None)
        return True
    except Exception as e:
        log.warning(f"inventaire_service.set_qualite [{classeur}#{rowid}]: {e}")
        return False


def set_quantite(classeur: str, rowid: int, quantite: int) -> bool:
    """
    Met à jour la quantité d'une carte possédée (classeur + rowid).

    Cohérent avec `dialog_carte` : `update_quantite_by_rowid` bascule
    automatiquement `possessed = 0` si la quantité tombe à 0 — la carte
    quittera donc l'inventaire au prochain rafraîchissement.
    Retourne True si l'écriture a abouti.
    """
    db_path = db_path_for(classeur)
    if not os.path.exists(db_path):
        return False
    try:
        update_quantite_by_rowid(db_path, rowid, max(0, int(quantite)))
        return True
    except Exception as e:
        log.warning(f"inventaire_service.set_quantite [{classeur}#{rowid}]: {e}")
        return False


def retirer_de_inventaire(classeur: str, rowid: int) -> bool:
    """
    Retire une carte de l'inventaire : possessed = 0 et quantite = 0.
    La ligne reste dans le classeur (la carte existe toujours dans le set),
    mais n'est plus comptée comme possédée.
    Retourne True si l'écriture a abouti.
    """
    # Réutilise set_quantite(0) qui, via update_quantite_by_rowid, met aussi
    # possessed = 0. Un point d'entrée unique pour la cohérence des écritures.
    return set_quantite(classeur, rowid, 0)


def filtrer_cartes(cartes: list, rarete: str = "", code: str = "",
                   set_name: str = "", classeur: str = "",
                   qualite: str = "", nom: str = "",
                   valeur_tous: str = "(Tous)") -> list:
    """
    Filtre une liste de cartes selon les critères fournis. Logique pure,
    séparée de l'affichage.

      rarete   : rareté exacte, ou "" / valeur_tous pour ne pas filtrer
      code     : sous-chaîne (insensible à la casse) du set_code
      set_name : nom de set exact, ou "" / valeur_tous pour ne pas filtrer
      classeur : classeur exact, ou "" / valeur_tous pour ne pas filtrer
      qualite  : qualité canonique exacte, ou "" / valeur_tous pour ne pas filtrer
                 (cas spécial : "__VIDE__" → uniquement les qualités non renseignées)
      nom      : sous-chaîne (insensible à la casse) du nom de carte
    """
    def actif(v: str) -> bool:
        return bool(v) and v != valeur_tous

    code_lower = (code or "").strip().lower()
    nom_lower  = (nom or "").strip().lower()
    result = []
    for c in cartes:
        if actif(rarete) and (c.get("rarity") or "") != rarete:
            continue
        if code_lower and code_lower not in (c.get("set_code") or "").lower():
            continue
        if actif(set_name) and (c.get("set_name") or "") != set_name:
            continue
        if actif(classeur) and (c.get("classeur") or "") != classeur:
            continue
        if actif(qualite):
            cq = c.get("qualite") or ""
            if qualite == "__VIDE__":
                if cq != "":
                    continue
            elif cq != qualite:
                continue
        if nom_lower and nom_lower not in (c.get("name") or "").lower():
            continue
        result.append(c)
    return result


if __name__ == "__main__":
    # Petit test manuel hors interface.
    cartes = get_cartes_possedees()
    print(f"{len(cartes)} carte(s) possédée(s) au total")
    for c in cartes[:20]:
        print(f"  [{c['classeur']}] {c['name']} ; {c['set_code']} ; "
              f"{c['rarity']} ; x{c['quantite']} ; {c['qualite'] or '—'}")

"""
correction_rarete.py — Correction des raretés parasites de YGOPRODeck via
Yugipedia.

YGOPRODeck insère parfois une annotation dans set_rarity (ex 'New artwork'
sur CH02-EN001) qui REMPLACE une vraie rareté (ici 'Ultra Rare'). Plutôt que
de supprimer ces lignes (ce qui ferait disparaître la vraie rareté), on
complète/corrige depuis Yugipedia, qui est la source de vérité.

Deux usages :

1. CRÉATION de classeur — `corriger_rows(code_set, rows)` :
   fonction sur listes de dicts (pas de DB). Pour chaque carte/set ayant une
   rareté non reconnue, récupère les vraies raretés Yugipedia et reconstruit
   les lignes du groupe. Fallback (Yugipedia KO) : retire les lignes
   invalides (comportement sûr, n'invente rien).

2. RÉPARATION d'un classeur existant — `reparer_db(db_file)` :
   opère par UPDATE/DELETE/INSERT pour PRÉSERVER la possession. Quand une
   rareté invalide correspond à une rareté manquante, on fait un UPDATE (la
   possession cochée reste attachée à la carte, juste la rareté est corrigée).

Identité d'un « groupe » : (card_image_id, set_code) — une carte dans un set.
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

from collections import defaultdict, OrderedDict

from module.gestion_rarete.raretes_reference import name_to_code
from module.gestion_rarete.yugipedia_rarete import get_raretes_carte_set
from module.centralisation_dossier import sqlite_ctx
from module.logger_app import log


def _rarity(row: dict) -> str:
    return (row.get("rarity") or "").strip()


def _est_invalide(rarity: str) -> bool:
    """Rareté NON vide et NON reconnue par le référentiel."""
    return bool(rarity) and name_to_code(rarity) is None


# ───────────────────────── CRÉATION (rows en mémoire) ──────────────────────

def corriger_rows(code_set: str, rows: list) -> list:
    """Corrige les raretés invalides d'une liste de rows via Yugipedia.

    - Groupe par (card_image_id, set_code).
    - Groupe sans rareté invalide : conservé tel quel.
    - Groupe avec rareté invalide : reconstruit depuis Yugipedia
      (raretés cibles). Fallback si Yugipedia KO : on retire les lignes
      invalides et on garde les valides.
    """
    groupes: "OrderedDict[tuple, list]" = OrderedDict()
    for r in rows:
        cle = (r.get("card_image_id"), r.get("set_code"))
        groupes.setdefault(cle, []).append(r)

    resultat = []
    nb_corr = nb_fallback = 0

    for (_cid, set_code), grp in groupes.items():
        if not any(_est_invalide(_rarity(r)) for r in grp):
            resultat.extend(grp)
            continue

        card_name = grp[0].get("name", "")
        cibles = get_raretes_carte_set(card_name, set_code) if set_code else None

        if not cibles:
            # Fallback sûr : garder uniquement les raretés reconnues.
            nb_fallback += 1
            resultat.extend(r for r in grp if not _est_invalide(_rarity(r)))
            continue

        nb_corr += 1
        # Index des rows valides existantes par code de rareté
        valides_par_code = {}
        for r in grp:
            c = name_to_code(_rarity(r))
            if c is not None and c not in valides_par_code:
                valides_par_code[c] = r
        # Modèle pour créer une rareté absente : une row valide sinon n'importe
        # laquelle du groupe (garde les métadonnées de la carte).
        modele = next(iter(valides_par_code.values()), grp[0])

        for nom_cible in cibles:
            code = name_to_code(nom_cible)
            if code in valides_par_code:
                resultat.append(valides_par_code[code])
            else:
                neuf = dict(modele)
                neuf["rarity"] = nom_cible
                neuf["rarity_code"] = code or ""
                resultat.append(neuf)

    if nb_corr or nb_fallback:
        log.info(
            f"{code_set}: raretés corrigées via Yugipedia pour {nb_corr} "
            f"carte(s), fallback (suppression) pour {nb_fallback}."
        )
    return resultat


# ───────────────────────── RÉPARATION (DB existante) ───────────────────────

def _lignes_db(conn):
    cur = conn.cursor()
    cur.execute(
        "SELECT rowid, card_image_id, set_code, name, rarity FROM cards"
    )
    return cur.fetchall()


def reparer_db(db_file: str) -> dict:
    """Répare un classeur SQLite existant. PRÉSERVE la possession.

    Retourne un récap : {"updates": n, "inserts": n, "deletes": n,
    "echecs": n, "cartes": n}.
    - echecs = groupes invalides pour lesquels Yugipedia n'a rien renvoyé
      (laissés intacts, à retenter plus tard).
    """
    stats = {"updates": 0, "inserts": 0, "deletes": 0, "echecs": 0, "cartes": 0}

    with sqlite_ctx(db_file) as conn:
        lignes = _lignes_db(conn)
        groupes = defaultdict(list)
        for rowid, cid, set_code, name, rarity in lignes:
            groupes[(cid, set_code)].append(
                {"rowid": rowid, "set_code": set_code, "name": name,
                 "rarity": (rarity or "").strip()}
            )

        for (_cid, set_code), grp in groupes.items():
            invalides = [g for g in grp if _est_invalide(g["rarity"])]
            if not invalides:
                continue
            stats["cartes"] += 1

            card_name = grp[0].get("name", "")
            cibles = get_raretes_carte_set(card_name, set_code) if set_code else None
            if not cibles:
                stats["echecs"] += 1
                continue

            codes_valides_presents = {
                name_to_code(g["rarity"]) for g in grp
                if g["rarity"] and name_to_code(g["rarity"]) is not None
            }
            # Raretés cibles encore absentes (à introduire), ordre Yugipedia
            manquantes = [
                (name_to_code(n), n) for n in cibles
                if name_to_code(n) not in codes_valides_presents
            ]
            # dédoublonne par code en gardant l'ordre
            vus = set()
            manquantes = [(c, n) for c, n in manquantes
                          if not (c in vus or vus.add(c))]

            # 1) Apparier invalide <-> manquante : UPDATE (conserve possession)
            n_pair = min(len(invalides), len(manquantes))
            for i in range(n_pair):
                inv = invalides[i]
                code, nom = manquantes[i]
                conn.execute(
                    "UPDATE cards SET rarity=?, rarity_code=? WHERE rowid=?",
                    (nom, code or "", inv["rowid"]),
                )
                stats["updates"] += 1

            # 2) Invalides en trop (pas de contrepartie) -> DELETE
            for inv in invalides[n_pair:]:
                conn.execute("DELETE FROM cards WHERE rowid=?", (inv["rowid"],))
                stats["deletes"] += 1

            # 3) Manquantes en trop -> INSERT (copie d'une ligne existante du
            #    groupe pour hériter image/atk/etc.)
            for code, nom in manquantes[n_pair:]:
                _inserer_copie(conn, _cid, set_code, nom, code or "")
                stats["inserts"] += 1

        conn.commit()

    return stats


def _inserer_copie(conn, card_image_id, set_code, rarity, rarity_code):
    """Insère une nouvelle rareté en copiant une ligne existante de la même
    carte/set (métadonnées identiques), avec possession remise à zéro."""
    cur = conn.cursor()
    cur.execute(
        "SELECT card_uuid, card_image_uuid, card_image_id, set_code, set_name, "
        "name, name_fr, card_image_url, card_image_small, sort_order, "
        "card_type, atk, def_val, level, attribute, race, extended_art, is_custom "
        "FROM cards WHERE card_image_id IS ? AND set_code IS ? LIMIT 1",
        (card_image_id, set_code),
    )
    base = cur.fetchone()
    if not base:
        return
    (card_uuid, card_image_uuid, c_image_id, s_code, set_name, name, name_fr,
     img_url, img_small, sort_order, card_type, atk, def_val, level,
     attribute, race, extended_art, is_custom) = base
    conn.execute(
        """INSERT INTO cards
           (card_uuid, card_image_uuid, card_image_id, set_code, rarity,
            rarity_code, set_name, name, name_fr, card_image_url,
            card_image_small, sort_order, card_type, atk, def_val, level,
            attribute, race, possessed, quantite, extended_art, is_custom)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,0,?,?)""",
        (card_uuid, card_image_uuid, c_image_id, s_code, rarity, rarity_code,
         set_name, name, name_fr, img_url, img_small, sort_order, card_type,
         atk, def_val, level, attribute, race, extended_art, is_custom),
    )

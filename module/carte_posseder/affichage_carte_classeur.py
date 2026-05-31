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
from tkinter import messagebox
from module.centralisation_dossier import AFFICHER_CARTE, sqlite_ctx
from module.gestion_rarete.tri_carte import sort_cartes
from module.config_langue import get_name_column
from module.config_image_source import load_image_source, YGOPRODECK_IMG_BASE
from urllib.parse import urlparse


def get_image_filename_from_url(url):
    if not url:
        return None
    path = urlparse(url).path
    return os.path.basename(path)


def get_cartes_info(code_set):
    """
    Récupère toutes les cartes d'un classeur avec leurs URLs d'image résolues.

    OPTIMISATION P4 : la source d'images est chargée UNE SEULE FOIS (pas par
    carte), et la résolution d'URL est inlinée pour éviter 500 appels de
    fonction redondants.
    """
    cartes = []
    db_path = os.path.join(AFFICHER_CARTE, code_set, f"{code_set}.db")
    if not os.path.exists(db_path):
        raise FileNotFoundError(f"La base de données du classeur {code_set} n'existe pas.")

    name_col = get_name_column()

    # ★ P4 : charger la source une seule fois
    image_source = load_image_source()

    with sqlite_ctx(db_path) as conn:
        cursor = conn.cursor()

        cursor.execute("PRAGMA table_info(cards)")
        cols = {row[1] for row in cursor.fetchall()}

        if name_col == "name_fr" and "name_fr" not in cols:
            name_col = "name"

        col_expr = f"COALESCE(NULLIF({name_col}, ''), name)" if name_col == "name_fr" else "name"

        # Une carte est affichée si elle a une image OU si c'est un ajout
        # manuel (is_custom=1) — ce dernier cas couvre les artworks manquants
        # ajoutés volontairement sans image (placeholder). La colonne
        # is_custom peut être absente sur de très anciens classeurs : on
        # ne référence la condition que si elle existe.
        custom_clause = " OR is_custom = 1" if "is_custom" in cols else ""

        custom_select = ", COALESCE(is_custom, 0) AS is_custom" if "is_custom" in cols else ""
        # extended_art : TOUJOURS présent dans le résultat (littéral 0 si la
        # colonne n'existe pas sur un très ancien classeur). Indispensable au
        # badge Overframe ET au filtre "N raretés par artwork" (Option 2).
        ext_expr = "COALESCE(extended_art, 0)" if "extended_art" in cols else "0"
        cursor.execute(f"""
            SELECT rowid, card_image_url, {col_expr}, rarity,
                   set_code, possessed, set_name,
                   card_image_id, COALESCE(quantite, 0) AS quantite,
                   COALESCE(sort_order, 0)              AS sort_order{custom_select},
                   {ext_expr} AS extended_art
            FROM cards
            WHERE card_image_url IS NOT NULL
               OR card_image_id  IS NOT NULL{custom_clause}
            ORDER BY sort_order, rarity
        """)
        has_custom_col = "is_custom" in cols
        for row in cursor.fetchall():
            if has_custom_col:
                (rowid, url, name, rarity, code, possessed, set_name,
                 img_id, quantite, sort_order, is_custom, extended_art) = row
            else:
                (rowid, url, name, rarity, code, possessed, set_name,
                 img_id, quantite, sort_order, extended_art) = row
                is_custom = 0

            # ★ P4 : résolution URL inlinée — évite appel fonction par carte
            if image_source == "YGOPRODECK":
                effective_url = YGOPRODECK_IMG_BASE.format(img_id) if img_id else (url or None)
            else:  # YUGIPEDIA
                effective_url = url or None

            img_filename = get_image_filename_from_url(effective_url)
            cartes.append({
                "rowid":          rowid,
                "image_filename": img_filename,
                "card_image_id":  img_id,
                "name":           name,
                "rarity":         rarity,
                "code":           code,
                "set_code":       code,
                "set_rarity":     rarity,
                "set_name":       set_name,
                "possessed":      possessed,
                "quantite":       max(0, quantite if quantite else 0),
                "sort_order":     sort_order,
                "is_custom":      is_custom,
                "extended_art":   int(extended_art or 0),
            })
    cartes = sort_cartes(cartes)
    return cartes

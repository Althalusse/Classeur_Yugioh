"""
dialog_carte.py — Dialog d'édition d'une carte (spec §Dialog Édition de carte).

- Image grande (w=128px, ratio 59:86)
- Nom FR/EN + badges code + rareté + stats
- Toggle possessed (CTkSwitch)
- Quantité (CTkEntry)
- Qualité (CTkComboBox)
- Modifications instantanées, fermeture par ✕, clic extérieur ou Échap
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
import customtkinter as ctk
from PIL import Image, ImageEnhance

from module.theme import C
from module.ui.composants import gold_button, secondary_button, styled_combobox
from module.gestion_img.gestion_image_classeur import get_image_path, get_placeholder_image
from module.gestion_img.cache_images import get_or_load_pil_image
from module.carte_posseder.gestion_carte_posseder import (
    update_quantite_by_rowid, update_qualite_by_rowid,
)
from module.centralisation_dossier import CLASSEUR_FOLDER
from module.config_langue import load_langue
from module.logger_app import log

QUALITE_OPTIONS = [
    "", "Mint (M)", "Near Mint (NM)", "Lightly Played (LP)",
    "Moderately Played (MP)", "Heavily Played (HP)", "Damaged (DMG)",
]
_QUALITE_MAP = {
    "Mint (M)": "Mint", "Near Mint (NM)": "Near Mint",
    "Lightly Played (LP)": "Lightly Played",
    "Moderately Played (MP)": "Moderately Played",
    "Heavily Played (HP)": "Heavily Played",
    "Damaged (DMG)": "Damaged", "": "",
}
_QUALITE_REV = {v: k for k, v in _QUALITE_MAP.items()}


class DialogCarte(ctk.CTkToplevel):
    """
    Dialog d'édition d'une carte (ouvert au clic sur la carte dans la grille).
    Sauvegarde immédiate à chaque modification.
    """

    IMG_W = 128
    IMG_H = int(128 * 86 / 59)   # ratio 59:86 ≈ 187px

    def __init__(self, parent, carte: dict, classeur_code: str,
                 on_update=None):
        super().__init__(parent)
        self.title(carte.get("name", "Carte"))
        self.geometry("480x500")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        self._carte      = carte
        self._code       = classeur_code
        self._db_path    = os.path.join(CLASSEUR_FOLDER, classeur_code,
                                         f"{classeur_code}.db")
        self._on_update  = on_update
        self._img_ref    = None

        self._build()
        self.bind("<Escape>", lambda e: self.destroy())
        # Y4 : l'auto-fermeture au clic extérieur a été retirée (la méthode
        # _on_focus_out ne faisait rien). L'utilisateur ferme via ✕ ou Escape.

    def _build(self):
        carte = self._carte
        use_fr = load_langue() == "FR"
        nom    = (carte.get("name_fr") or carte.get("name", "")) if use_fr else carte.get("name", "")
        code   = carte.get("set_code", "") or carte.get("code", "")
        rarity = carte.get("rarity", "")

        # Bouton fermer
        ctk.CTkButton(
            self, text="✕", width=30, height=30,
            fg_color="transparent", hover_color=C["bg_hover"],
            text_color=C["text2"], font=("Segoe UI", 14),
            command=self.destroy, corner_radius=4,
        ).place(relx=1.0, rely=0.0, anchor="ne", x=-8, y=8)

        # Corps principal
        body = ctk.CTkFrame(self, fg_color="transparent")
        body.pack(fill="both", expand=True, padx=20, pady=(16, 20))

        # ── Ligne image + infos ───────────────────────────────────────────
        top_row = ctk.CTkFrame(body, fg_color="transparent")
        top_row.pack(fill="x", pady=(0, 12))

        # Image
        # M1 : utilisation du cache PIL partagé (cache_images.get_or_load_pil_image)
        # au lieu d'un Image.open() direct. Bénéfice : si la même carte a déjà
        # été chargée dans le visualiseur, on évite une 2e lecture disque.
        img_path = get_image_path(self._code, carte.get("image_filename") or "")
        pil = get_or_load_pil_image(img_path)
        if pil is not None:
            try:
                pil_resized = pil.resize((self.IMG_W, self.IMG_H), Image.LANCZOS)
                self._img_ref = ctk.CTkImage(pil_resized, size=(self.IMG_W, self.IMG_H))
                ctk.CTkLabel(top_row, image=self._img_ref, text="",
                             corner_radius=8).pack(side="left", padx=(0, 16))
            except Exception:
                ctk.CTkLabel(top_row, text="🃏", font=("Segoe UI", 48),
                             text_color=C["text3"]).pack(side="left", padx=(0, 16))
        else:
            # Fichier absent ou illisible → placeholder texte (emoji carte)
            ctk.CTkLabel(top_row, text="🃏", font=("Segoe UI", 48),
                         text_color=C["text3"]).pack(side="left", padx=(0, 16))

        # Infos texte
        info = ctk.CTkFrame(top_row, fg_color="transparent")
        info.pack(side="left", fill="both", expand=True)

        ctk.CTkLabel(
            info, text=nom,
            font=("Playfair Display", 15, "bold"),
            text_color=C["text"],
            wraplength=290, justify="left",
        ).pack(anchor="w")

        badges_row = ctk.CTkFrame(info, fg_color="transparent")
        badges_row.pack(anchor="w", pady=4)
        if code:
            ctk.CTkLabel(badges_row, text=code,
                         fg_color=C["bg3"], text_color=C["gold"],
                         font=("JetBrains Mono", 11),
                         corner_radius=4, padx=6, pady=2).pack(side="left", padx=(0, 4))
        if rarity:
            ctk.CTkLabel(badges_row, text=rarity,
                         fg_color=C["bg3"], text_color=C["text2"],
                         font=("Outfit", 11),
                         corner_radius=4, padx=6, pady=2).pack(side="left")

        # Stats (si disponibles)
        stats_parts = []
        if carte.get("card_type"):
            stats_parts.append(carte["card_type"])
        atk = carte.get("atk")
        def_val = carte.get("def_val")
        if atk is not None:
            stats_parts.append(f"ATK/{atk}")
        if def_val is not None:
            stats_parts.append(f"DEF/{def_val}")
        if stats_parts:
            ctk.CTkLabel(
                info,
                text="  ".join(stats_parts),
                font=("JetBrains Mono", 11),
                text_color=C["text3"],
            ).pack(anchor="w", pady=(2, 0))

        # ── Séparateur ────────────────────────────────────────────────────
        ctk.CTkFrame(body, height=1, fg_color=C["border"],
                     corner_radius=0).pack(fill="x", pady=12)

        # ── Champs édition ────────────────────────────────────────────────
        fields = ctk.CTkFrame(body, fg_color="transparent")
        fields.pack(fill="x")
        fields.columnconfigure(1, weight=1)

        row_i = 0

        # Possessed toggle
        ctk.CTkLabel(fields, text="Possédée",
                     font=("Outfit", 13), text_color=C["text2"]).grid(
            row=row_i, column=0, sticky="w", pady=8)
        self._switch = ctk.CTkSwitch(
            fields, text="",
            progress_color=C["gold"],
            button_color=C["text"],
            button_hover_color=C["text2"],
            fg_color=C["bg3"],
            onvalue=1, offvalue=0,
            command=self._on_toggle,
            width=44, height=24,
        )
        self._switch.grid(row=row_i, column=1, sticky="e", pady=8)
        if carte.get("quantite", 0) > 0:
            self._switch.select()
        row_i += 1

        # Quantité
        ctk.CTkLabel(fields, text="Quantité",
                     font=("Outfit", 13), text_color=C["text2"]).grid(
            row=row_i, column=0, sticky="w", pady=8)
        self._qty_var = ctk.StringVar(value=str(max(0, carte.get("quantite", 0))))
        qty_entry = ctk.CTkEntry(
            fields, textvariable=self._qty_var,
            width=80, height=36,
            fg_color=C["bg3"], border_color=C["border2"],
            text_color=C["text"], font=("JetBrains Mono", 14),
            justify="center", corner_radius=4,
        )
        qty_entry.grid(row=row_i, column=1, sticky="e", pady=8)
        qty_entry.bind("<Return>", lambda e: self._on_qty_change())
        qty_entry.bind("<FocusOut>", lambda e: self._on_qty_change())
        row_i += 1

        # Qualité
        ctk.CTkLabel(fields, text="Qualité",
                     font=("Outfit", 13), text_color=C["text2"]).grid(
            row=row_i, column=0, sticky="w", pady=8)
        current_q  = carte.get("qualite") or ""
        display_q  = _QUALITE_REV.get(current_q, current_q)
        self._qual_var = ctk.StringVar(value=display_q or "")
        qual_cb = styled_combobox(
            fields,
            values=QUALITE_OPTIONS,
            variable=self._qual_var,
            width=200, state="readonly",
        )
        qual_cb.grid(row=row_i, column=1, sticky="e", pady=8)
        qual_cb.configure(command=lambda val: self._on_qual_change(val))

    # ── Callbacks ─────────────────────────────────────────────────────────

    def _on_toggle(self):
        on = self._switch.get()
        qty = 1 if on else 0
        self._qty_var.set(str(qty))
        self._save_qty(qty)

    def _on_qty_change(self):
        try:
            qty = max(0, int(self._qty_var.get()))
        except ValueError:
            qty = 0
        self._qty_var.set(str(qty))
        if qty > 0:
            self._switch.select()
        else:
            self._switch.deselect()
        self._save_qty(qty)

    def _on_qual_change(self, display_val: str):
        real_val = _QUALITE_MAP.get(display_val, display_val)
        rowid    = self._carte.get("rowid")
        if rowid is None:
            return
        try:
            update_qualite_by_rowid(self._db_path, rowid, real_val)
            if self._on_update:
                self._on_update()
        except Exception as e:
            log.warning(f"dialog_carte qualite: {e}")

    def _save_qty(self, qty: int):
        rowid = self._carte.get("rowid")
        if rowid is None:
            return
        try:
            update_quantite_by_rowid(self._db_path, rowid, qty)
            self._carte["quantite"]  = qty
            self._carte["possessed"] = 1 if qty > 0 else 0
            if self._on_update:
                self._on_update()
        except Exception as e:
            log.warning(f"dialog_carte qty: {e}")

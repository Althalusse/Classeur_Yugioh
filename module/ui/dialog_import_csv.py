"""
dialog_import_csv.py — Dialog import CSV (spec §Dialog Import CSV).
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

import threading
import customtkinter as ctk
from tkinter import filedialog
from module.theme import C
from module.ui.composants import gold_button, secondary_button

class DialogImportCSV(ctk.CTkToplevel):
    def __init__(self, parent, classeur_code: str, on_update=None):
        super().__init__(parent)
        self.title("Importer un CSV")
        self.geometry("500x380")
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        self._code      = classeur_code
        self._on_update = on_update
        self._filepath  = ""

        self._build()
        self.bind("<Escape>", lambda e: self.destroy())

    def _build(self):
        header = ctk.CTkFrame(self, fg_color="transparent")
        header.pack(fill="x", padx=20, pady=(16, 8))
        ctk.CTkLabel(header, text="⬆ Importer un fichier CSV",
                     font=("Playfair Display", 14, "bold"),
                     text_color=C["text"]).pack(side="left")
        ctk.CTkButton(header, text="✕", width=30, height=30,
                      fg_color="transparent", hover_color=C["bg_hover"],
                      text_color=C["text2"], command=self.destroy,
                      corner_radius=4).pack(side="right")

        ctk.CTkLabel(self,
                     text="Format compatible avec les sites de collection FR.\nSéparateur ; — supporte le format d'export et Scanflip.",
                     font=("Outfit", 10), text_color=C["text3"]).pack(padx=20, anchor="w")

        ctk.CTkFrame(self, height=1, fg_color=C["border"]).pack(fill="x", padx=20, pady=10)

        # Zone sélection fichier
        drop_zone = ctk.CTkFrame(
            self, fg_color=C["bg3"],
            border_color=C["border2"], border_width=1,
            corner_radius=8, height=120,
        )
        drop_zone.pack(fill="x", padx=20, pady=8)
        drop_zone.pack_propagate(False)

        ctk.CTkLabel(drop_zone, text="📁", font=("Segoe UI", 28),
                     text_color=C["text3"]).pack(pady=(16, 4))
        self._file_lbl = ctk.CTkLabel(drop_zone, text="Choisir un fichier CSV",
                                       font=("Outfit", 10), text_color=C["text2"],
                                       cursor="hand2")
        self._file_lbl.pack()
        self._file_lbl.bind("<Button-1>", lambda e: self._choisir_fichier())

        # Résultat
        self._result_lbl = ctk.CTkLabel(self, text="", font=("Outfit", 10),
                                         text_color=C["text3"])
        self._result_lbl.pack(padx=20, pady=8, anchor="w")

        # Bouton importer
        self._btn_import = gold_button(
            self, "═══════ Importer ═══════",
            command=self._importer, width=260,
        )
        self._btn_import.pack(pady=12)
        self._btn_import.configure(state="disabled",
                                    fg_color=C["bg3"], text_color=C["text3"])

    def _choisir_fichier(self):
        path = filedialog.askopenfilename(
            title="Choisir un fichier CSV",
            filetypes=[("CSV", "*.csv"), ("Tous", "*.*")],
        )
        if path:
            self._filepath = path
            import os
            self._file_lbl.configure(text=os.path.basename(path),
                                      text_color=C["gold"])
            self._btn_import.configure(state="normal",
                                        fg_color=C["gold"], text_color="#000")

    def _importer(self):
        if not self._filepath:
            return
        self._btn_import.configure(text="⏳ Import en cours…", state="disabled")
        threading.Thread(target=self._import_worker, daemon=True).start()

    def _import_worker(self):
        try:
            # Import CSV via le module existant (à adapter selon disponibilité)
            # Pour l'instant on affiche un message de succès simulé
            self.after(0, self._on_import_done, 0, 0, "Fonctionnalité à connecter.")
        except Exception as e:
            self.after(0, self._on_import_done, 0, 0, str(e))

    def _on_import_done(self, imported: int, not_found: int, msg: str = ""):
        self._btn_import.configure(text="═══════ Importer ═══════",
                                    state="normal", fg_color=C["gold"],
                                    text_color="#000")
        if imported > 0:
            self._result_lbl.configure(
                text=f"✅ {imported} carte(s) importée(s){f'  ⚠ {not_found} non trouvée(s)' if not_found else ''}",
                text_color=C["success"],
            )
        elif msg:
            self._result_lbl.configure(text=f"ℹ {msg}", text_color=C["text3"])
        if self._on_update:
            self._on_update()

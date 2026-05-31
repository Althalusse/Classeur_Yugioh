"""
ecran_donation.py — Écran « Contribution » (Ko-fi).

Pourquoi un sous-processus ? pywebview exige que webview.start() tourne sur
le THREAD PRINCIPAL. Or le mainloop Tkinter / CustomTkinter occupe déjà ce
thread principal : lancer pywebview dans le même process provoque un
conflit (freeze / crash). On lance donc kofi_viewer.py comme processus
indépendant (subprocess.Popen + CREATE_NO_WINDOW), qui obtient son propre
thread principal. Si pywebview est absent → fallback navigateur.

kofi_viewer.py doit être présent à la racine du projet (et copié à côté du
.exe via --add-data "kofi_viewer.py;." en build PyInstaller).

Enregistré dans NavigationController sous la clé "donation".
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
import sys
import subprocess
import webbrowser

import customtkinter as ctk

from module.theme import C
from module.ui.composants import Navbar, gold_button
from module.i18n import t
from module.logger_app import log

KOFI_PAGE = "https://ko-fi.com/althalusse"


def _args_relance_kofi() -> list:
    """Arguments pour relancer l'application en mode viewer Ko-fi (--kofi).

    Compatible PyInstaller --onefile : on relance le .exe lui-même. main.py
    intercepte --kofi tout en haut et lance pywebview dans CE process dédié
    (sans mainloop Tkinter). Un seul .exe, aucun fichier externe.

    - Mode frozen : [<exe>, "--kofi"]
    - Mode dev    : [<python>, <racine>/main.py, "--kofi"]
    """
    if getattr(sys, "frozen", False):
        return [sys.executable, "--kofi"]
    here = os.path.dirname(os.path.abspath(__file__))      # module/ui
    root = os.path.dirname(os.path.dirname(here))          # racine projet
    return [sys.executable, os.path.join(root, "main.py"), "--kofi"]


def _ouvrir_kofi():
    """Relance l'app en mode --kofi (fenêtre pywebview), sinon navigateur."""
    try:
        # env_relance() purge les variables PyInstaller (_MEIPASS2/_PYI) pour
        # que le process Ko-fi extraie son propre dossier temporaire en onefile
        # (sinon il partagerait le _MEIxxxxx du parent, supprimé à la fermeture).
        from module.utilitaire.redemarrage import env_relance
        subprocess.Popen(
            _args_relance_kofi(),
            env=env_relance(),
            creationflags=(
                subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
            ),
        )
    except Exception as e:
        log.warning(f"EcranDonation._ouvrir_kofi: {e}")
        webbrowser.open(KOFI_PAGE)


class EcranDonation(ctk.CTkFrame):
    """Écran Contribution / Ko-fi."""

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._build()

    def _build(self):
        Navbar(
            self,
            title=t("tab.contribution"),
            show_back=True,
            back_command=self._retour,
        ).pack(fill="x")

        # pywebview disponible ?
        try:
            import webview  # type: ignore  # noqa: F401
            webview_dispo = True
        except ImportError:
            webview_dispo = False

        centre = ctk.CTkFrame(self, fg_color="transparent")
        centre.place(relx=0.5, rely=0.5, anchor="center")

        ctk.CTkLabel(
            centre, text="☕", font=("Segoe UI", 56), text_color=C["gold"],
        ).pack(pady=(0, 8))
        ctk.CTkLabel(
            centre, text=t("kofi.title"),
            font=("Georgia", 18, "bold"), text_color=C["text"],
        ).pack(pady=(0, 6))
        ctk.CTkLabel(
            centre, text=t("kofi.desc"),
            font=("Outfit", 11), text_color=C["text2"],
        ).pack(pady=(0, 28))

        if webview_dispo:
            gold_button(
                centre, t("kofi.open"), command=_ouvrir_kofi, width=240,
            ).pack(pady=(0, 10))
            ctk.CTkLabel(
                centre, text=t("kofi.integrated"),
                font=("Outfit", 9), text_color=C["text3"],
            ).pack()
        else:
            gold_button(
                centre, t("kofi.open_browser"),
                command=lambda: webbrowser.open(KOFI_PAGE), width=300,
            ).pack(pady=(0, 10))
            ctk.CTkLabel(
                centre, text=t("kofi.install_hint"),
                font=("Outfit", 9), text_color=C["text3"],
            ).pack()

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")

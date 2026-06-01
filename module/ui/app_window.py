"""
app_window.py — Contrôleur de navigation principal (frame-based, pas de Notebook).
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

import sys
import customtkinter as ctk

from module.theme import C, setup_ctk
from module.ui.ecran_accueil import EcranAccueil
from module.ui.ecran_selecteur_set import EcranSelecteurSet
from module.ui.ecran_classeur import EcranClasseur
from module.ui.ecran_options import EcranOptions
from module.ui.ecran_statistique import EcranStatistiques
from module.ui.ecran_inventaire import EcranInventaire
from module.ui.ecran_donation import EcranDonation
from module.ui.ecran_merci import EcranMerci
from module.utilitaire.raccourcis import activer_raccourcis


def build_app(root):
    """Configure la fenêtre principale et lance la navigation."""
    setup_ctk()

    # Fond global
    try:
        root.configure(fg_color=C["bg"])   # CTk
    except Exception:
        try:
            root.configure(bg=C["bg"])      # tk.Tk fallback
        except Exception:
            pass

    # Options globales tkinter
    root.option_add("*Background",              C["bg"])
    root.option_add("*Foreground",              C["text"])
    root.option_add("*Listbox.Background",      C["bg2"])
    root.option_add("*Listbox.Foreground",      C["text"])
    root.option_add("*Listbox.SelectBackground", C["gold"])
    root.option_add("*Listbox.SelectForeground", "#000000")
    root.option_add("*Canvas.Background",       C["bg"])
    root.option_add("*Entry.Background",        C["bg2"])
    root.option_add("*Entry.Foreground",        C["text"])
    root.option_add("*Entry.InsertBackground",  C["gold"])

    activer_raccourcis(root)

    nav = NavigationController(root)
    nav.pack(fill="both", expand=True)

    root.protocol("WM_DELETE_WINDOW", lambda: _on_close(root))


class NavigationController(ctk.CTkFrame):
    """Affiche un seul écran à la fois."""

    def __init__(self, parent):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._screens: dict[str, ctk.CTkFrame] = {}
        self._current: str | None = None
        self._build_screens()
        self.navigate_to("accueil")

    def _build_screens(self):
        nav = self.navigate_to
        self._screens["accueil"]       = EcranAccueil(self,       navigate_to=nav)
        self._screens["selecteur_set"] = EcranSelecteurSet(self,  navigate_to=nav)
        self._screens["classeur"]      = EcranClasseur(self,      navigate_to=nav)
        self._screens["options"]       = EcranOptions(self,       navigate_to=nav)
        self._screens["statistique"]   = EcranStatistiques(self,  navigate_to=nav)
        self._screens["inventaire"]    = EcranInventaire(self,    navigate_to=nav)
        self._screens["donation"]      = EcranDonation(self,      navigate_to=nav)
        self._screens["merci"]         = EcranMerci(self,         navigate_to=nav)

    def navigate_to(self, screen: str, **kwargs):
        if self._current:
            self._screens[self._current].pack_forget()

        target = self._screens.get(screen)
        if target is None:
            return

        self._current = screen
        target.pack(fill="both", expand=True)

        # Diffère l'action de chargement au prochain tour de boucle d'événements.
        # Sans ça, le thread spawné par rafraichir() peut appeler .after(0, ...)
        # AVANT que mainloop soit démarré, ce qui fait planter sous Windows.
        if   screen == "accueil":
            target.after_idle(target.rafraichir)
        elif screen == "selecteur_set":
            target.after_idle(target.charger)
        elif screen == "classeur":
            code = kwargs.get("code", "")
            if code:
                target.after_idle(lambda c=code: target.charger(c))
        elif screen == "options":
            target.after_idle(target.charger)
        elif screen == "statistique":
            target.after_idle(target.charger)
        elif screen == "inventaire":
            target.after_idle(target.charger)


def _on_close(root):
    root.destroy()
    sys.exit(0)

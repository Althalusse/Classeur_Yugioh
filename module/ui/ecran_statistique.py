"""
ecran_statistique.py — Écran Statistiques globales (tous classeurs).

Enveloppe d'écran (navbar + retour accueil) autour de
PanneauStatistiquesGlobales. Enregistré dans NavigationController sous la
clé "statistique" ; chargé via navigate_to("statistique").
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

import customtkinter as ctk

from module.theme import C
from module.ui.composants import Navbar
from module.statistique.statistique_collection import PanneauStatistiquesGlobales
from module.i18n import t


class EcranStatistiques(ctk.CTkFrame):
    """Écran d'ensemble des statistiques de collection."""

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._build()

    def _build(self):
        Navbar(
            self,
            title=t("stats.title"),
            subtitle=t("stats.global_subtitle"),
            show_back=True,
            back_command=self._retour,
        ).pack(fill="x")

        self._panneau = PanneauStatistiquesGlobales(self)
        self._panneau.pack(fill="both", expand=True)

    def charger(self):
        """Appelé par NavigationController à l'affichage de l'écran."""
        self._panneau.lancer()

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")

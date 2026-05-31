"""
ecran_merci.py — Écran « Merci » : crédits des API et ressources externes
utilisées par l'application, avec lien vers chaque site.

Sources listées (réellement utilisées dans le code) :
  - YGOPRODeck  : base de données cartes/sets + images (db./images.ygoprodeck.com)
  - Yugipedia   : wiki — raretés officielles + covers (yugipedia.com)
  - YGOJSON     : données de cartes open source (github iconmaster5326/YGOJSON)
  - Scanflip    : convention de format import/export de collection (scanflip.fr)

Enregistré dans NavigationController sous la clé "merci".
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

import webbrowser

import customtkinter as ctk

from module.theme import C
from module.ui.composants import Navbar, secondary_button
from module.i18n import t
from module.logger_app import log

# (nom, url, clé i18n de description). Nom et URL identiques FR/EN → en dur.
_SOURCES = [
    ("YGOPRODeck", "https://ygoprodeck.com",                       "thanks.ygoprodeck.desc"),
    ("Yugipedia",  "https://yugipedia.com",                        "thanks.yugipedia.desc"),
    ("YGOJSON",    "https://github.com/iconmaster5326/YGOJSON",    "thanks.ygojson.desc"),
    ("Scanflip",   "https://scanflip.fr",                          "thanks.scanflip.desc"),
]


def _ouvrir(url: str):
    try:
        webbrowser.open(url)
    except Exception as e:
        log.warning(f"EcranMerci: ouverture {url}: {e}")


class EcranMerci(ctk.CTkFrame):
    """Page de remerciements / crédits des sources de données."""

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._build()

    def _build(self):
        Navbar(
            self,
            title=t("thanks.title"),
            subtitle=t("thanks.subtitle"),
            show_back=True,
            back_command=self._retour,
        ).pack(fill="x")

        liste = ctk.CTkScrollableFrame(self, fg_color="transparent",
                                       corner_radius=0)
        liste.pack(fill="both", expand=True, padx=24, pady=(16, 16))

        ctk.CTkLabel(
            liste, text="🙏", font=("Segoe UI", 44), text_color=C["gold"],
        ).pack(pady=(4, 6))
        ctk.CTkLabel(
            liste, text=t("thanks.intro"), font=("Outfit", 12),
            text_color=C["text2"], wraplength=720, justify="center",
        ).pack(pady=(0, 18))

        for nom, url, desc_key in _SOURCES:
            self._carte_source(liste, nom, url, t(desc_key))

        # Mention légale / non-affiliation
        ctk.CTkLabel(
            liste, text=t("thanks.disclaimer"), font=("Outfit", 9),
            text_color=C["text3"], wraplength=720, justify="center",
        ).pack(pady=(18, 4))

    def _carte_source(self, parent, nom: str, url: str, desc: str):
        carte = ctk.CTkFrame(
            parent, fg_color=C["bg2"],
            border_color=C["border"], border_width=1, corner_radius=8,
        )
        carte.pack(fill="x", pady=6)

        inner = ctk.CTkFrame(carte, fg_color="transparent")
        inner.pack(fill="x", padx=16, pady=12)

        # Texte (nom + description) à gauche, bouton à droite
        txt = ctk.CTkFrame(inner, fg_color="transparent")
        txt.pack(side="left", fill="x", expand=True)
        ctk.CTkLabel(
            txt, text=nom, anchor="w",
            font=("Georgia", 15, "bold"), text_color=C["gold"],
        ).pack(anchor="w")
        ctk.CTkLabel(
            txt, text=desc, anchor="w", justify="left",
            font=("Outfit", 11), text_color=C["text2"], wraplength=520,
        ).pack(anchor="w", pady=(2, 0))

        btn = secondary_button(
            inner, t("thanks.visit"),
            command=lambda u=url: _ouvrir(u), width=150,
        )
        btn.configure(height=34)
        btn.pack(side="right", padx=(12, 0))

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")

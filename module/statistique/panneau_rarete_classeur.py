"""
panneau_rarete_classeur.py — Panneau dépliable « Statistiques par rareté »
intégré dans l'écran classeur.

Principe : aucune requête disque. Les cartes du classeur ouvert sont déjà
chargées en mémoire dans EcranClasseur._cartes (chaque dict expose `rarity`
et `quantite`). Le panneau agrège ces données instantanément, groupées par
rareté, et les ordonne selon l'ordre logique de raretes_reference.RARETES
(Commune → Ghost Rare).

Possédée = quantite > 0 (même convention que la navbar du classeur).

API publique :
    p = PanneauRareteClasseur(parent)
    p.pack(fill="x")
    p.set_cartes(liste_de_cartes)   # (re)calcule + (re)affiche

Le panneau est REPLIÉ par défaut pour ne pas pousser la zone des cartes.
Un en-tête cliquable (▸ / ▾) le déplie/replie et affiche un résumé global
même replié.
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

from collections import defaultdict

import customtkinter as ctk

from module.theme import C
from module.ui.composants import progress_bar, separator
from module.gestion_rarete.raretes_reference import name_to_code, RARETES
from module.i18n import t
from module.logger_app import log


# Ordre logique des raretés (index dans RARETES, qui est insertion-ordered :
# Commune en premier → Ghost Rare en dernier). Les raretés inconnues sont
# renvoyées en fin de liste.
_ORDRE_RARETE = {code: i for i, code in enumerate(RARETES.keys())}
_RANG_INCONNU = len(_ORDRE_RARETE) + 1


def _rang_rarete(rarity: str) -> int:
    code = name_to_code(rarity or "")
    return _ORDRE_RARETE.get(code, _RANG_INCONNU)


def agreger_par_rarete(cartes: list) -> list:
    """Agrège les cartes par rareté.

    Retourne une liste triée de dicts :
        {"rarete": str, "total": int, "possedees": int, "pourcentage": float}
    Le tri suit l'ordre logique des raretés, puis l'ordre alphabétique.
    """
    agg: dict = defaultdict(lambda: {"total": 0, "possedees": 0})
    for c in cartes:
        rarete = (c.get("rarity") or "").strip() or "—"
        bloc = agg[rarete]
        bloc["total"] += 1
        if (c.get("quantite") or 0) > 0:
            bloc["possedees"] += 1

    resultat = []
    for rarete, bloc in agg.items():
        total = bloc["total"]
        poss = bloc["possedees"]
        resultat.append({
            "rarete":      rarete,
            "total":       total,
            "possedees":   poss,
            "pourcentage": (poss / total * 100) if total > 0 else 0.0,
        })
    resultat.sort(key=lambda r: (_rang_rarete(r["rarete"]), r["rarete"].lower()))
    return resultat


class PanneauRareteClasseur(ctk.CTkFrame):
    """Panneau dépliable affichant les cartes possédées par rareté."""

    def __init__(self, parent, **kw):
        kw.setdefault("fg_color", C["bg2"])
        kw.setdefault("corner_radius", 0)
        super().__init__(parent, **kw)

        self._cartes: list = []
        self._expanded = False

        # En-tête cliquable (toggle déplier/replier + résumé global)
        self._header = ctk.CTkButton(
            self,
            text="",
            command=self._toggle,
            anchor="w",
            fg_color="transparent",
            hover_color=C["bg_hover"],
            text_color=C["text"],
            font=("Outfit", 13, "bold"),
            corner_radius=0,
            border_width=0,
            height=40,
        )
        self._header.pack(fill="x", padx=16, pady=(2, 2))

        # Corps (liste des raretés) — masqué tant que replié
        self._body = ctk.CTkFrame(self, fg_color="transparent")

        self._maj_header()

    # ── API ────────────────────────────────────────────────────────────────

    def set_cartes(self, cartes: list):
        """(Re)définit les cartes et rafraîchit l'affichage."""
        self._cartes = cartes or []
        self._maj_header()
        if self._expanded:
            self._rendre_corps()

    # ── Interne ──────────────────────────────────────────────────────────────

    def _toggle(self):
        self._expanded = not self._expanded
        if self._expanded:
            self._rendre_corps()
            self._body.pack(fill="x", padx=16, pady=(0, 8))
        else:
            self._body.pack_forget()
        self._maj_header()

    def _maj_header(self):
        fleche = "▾" if self._expanded else "▸"
        total = len(self._cartes)
        poss = sum(1 for c in self._cartes if (c.get("quantite") or 0) > 0)
        pct = (poss / total * 100) if total > 0 else 0.0
        resume = f"   {poss} / {total} ({pct:.0f}%)" if total else ""
        self._header.configure(text=f"{fleche}  {t('stats.by_rarity')}{resume}")

    def _rendre_corps(self):
        for w in self._body.winfo_children():
            w.destroy()

        if not self._cartes:
            ctk.CTkLabel(
                self._body, text=t("stats.binder_empty"),
                font=("Outfit", 12), text_color=C["text3"],
            ).pack(anchor="w", pady=6)
            return

        lignes = agreger_par_rarete(self._cartes)

        # En-tête de colonnes
        entete = ctk.CTkFrame(self._body, fg_color="transparent")
        entete.pack(fill="x", pady=(2, 4))
        ctk.CTkLabel(
            entete, text=t("stats.collection"), width=215, anchor="w",
            font=("Outfit", 11, "bold"), text_color=C["text3"],
        ).pack(side="left")
        ctk.CTkLabel(
            entete, text="", anchor="w",
            font=("Outfit", 11, "bold"), text_color=C["text3"],
        ).pack(side="left", fill="x", expand=True)
        ctk.CTkLabel(
            entete, text=f"{t('stats.possessed')} / {t('stats.total')}",
            width=175, anchor="e",
            font=("Outfit", 11, "bold"), text_color=C["text3"],
        ).pack(side="left")

        for ligne in lignes:
            row = ctk.CTkFrame(self._body, fg_color="transparent")
            row.pack(fill="x", pady=3)

            ctk.CTkLabel(
                row, text=ligne["rarete"], width=215, anchor="w",
                font=("Outfit", 12), text_color=C["text"],
            ).pack(side="left")

            barre = progress_bar(row, height=14)
            barre.pack(side="left", fill="x", expand=True, padx=10)
            try:
                barre.set(min(1.0, ligne["pourcentage"] / 100))
            except Exception:
                barre.set(0)

            detail = (f"{ligne['possedees']} / {ligne['total']} "
                      f"({ligne['pourcentage']:.0f}%)")
            ctk.CTkLabel(
                row, text=detail, width=175, anchor="e",
                font=("JetBrains Mono", 11), text_color=C["text2"],
            ).pack(side="left")

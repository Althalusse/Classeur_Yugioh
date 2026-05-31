"""
statistique_collection.py — UI des statistiques globales (tous classeurs).

Réécriture CustomTkinter du module historique (anciennement ttk + cache
disque). Le cache `stats_cache.json` a été retiré au ticket M5 : le calcul
est désormais relancé à la demande dans un thread de fond (0 freeze UI), ce
qui suffit largement tant que le nombre de classeurs reste raisonnable.

Données : get_stats_collection() (service) renvoie par classeur le total,
le nombre de possédées, le pourcentage, et la décomposition par rareté.

API publique :
    panneau = PanneauStatistiquesGlobales(parent)
    panneau.pack(fill="both", expand=True)
    panneau.lancer()        # lance / relance le calcul en arrière-plan
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

from module.theme import C
from module.ui.composants import (
    StatCard, separator, progress_bar, secondary_button, search_entry,
)
from module.statistique.statistique_collection_service import get_stats_collection
from module.statistique.panneau_rarete_classeur import _rang_rarete
from module.i18n import t
from module.logger_app import log


class PanneauStatistiquesGlobales(ctk.CTkFrame):
    """Vue d'ensemble : cartes récap + liste des classeurs avec détail rareté."""

    def __init__(self, parent, **kw):
        kw.setdefault("fg_color", C["bg"])
        kw.setdefault("corner_radius", 0)
        super().__init__(parent, **kw)

        self._destroyed = False
        self._details_ouverts: set = set()   # noms de classeurs dont le détail est déplié
        self._stats_courantes: list = []
        self._search_var = ctk.StringVar()
        self._search_after = None

        # ── Cartes récap ────────────────────────────────────────────────
        recap = ctk.CTkFrame(self, fg_color="transparent")
        recap.pack(fill="x", padx=24, pady=(18, 0))
        recap.columnconfigure((0, 1, 2, 3), weight=1)

        self._card_classeurs = StatCard(recap, "—", t("stats.collection"))
        self._card_classeurs.grid(row=0, column=0, padx=8, sticky="ew")
        self._card_total = StatCard(recap, "—", t("stats.total"))
        self._card_total.grid(row=0, column=1, padx=8, sticky="ew")
        self._card_poss = StatCard(recap, "—", t("stats.possessed"))
        self._card_poss.grid(row=0, column=2, padx=8, sticky="ew")
        self._card_pct = StatCard(recap, "—", t("stats.completion"))
        self._card_pct.grid(row=0, column=3, padx=8, sticky="ew")

        # ── Barre statut + rafraîchir ─────────────────────────────────────
        barre = ctk.CTkFrame(self, fg_color="transparent")
        barre.pack(fill="x", padx=24, pady=(14, 6))
        self._lbl_statut = ctk.CTkLabel(
            barre, text="", font=("Outfit", 12), text_color=C["text3"],
        )
        self._lbl_statut.pack(side="left")
        secondary_button(
            barre, t("btn.refresh_stats"), command=self.lancer, width=240,
        ).pack(side="right")

        # ── Recherche (filtre la liste des classeurs par nom) ──────────────
        search_row = ctk.CTkFrame(self, fg_color="transparent")
        search_row.pack(fill="x", padx=24, pady=(0, 8))
        search_entry(
            search_row, textvariable=self._search_var,
            placeholder=t("search.binder"),
        ).pack(fill="x")
        self._search_var.trace_add("write", self._on_recherche)

        separator(self).pack(fill="x", padx=24, pady=(0, 8))

        # ── Liste scrollable des classeurs ─────────────────────────────────
        self._liste = ctk.CTkScrollableFrame(self, fg_color="transparent",
                                              corner_radius=0)
        self._liste.pack(fill="both", expand=True, padx=16, pady=(0, 16))

        self.bind("<Destroy>", self._on_destroy)

    # ── API ────────────────────────────────────────────────────────────────

    def lancer(self):
        """Lance (ou relance) le calcul des statistiques en arrière-plan."""
        self._lbl_statut.configure(text=t("stats.calculating"))
        for w in self._liste.winfo_children():
            w.destroy()
        ctk.CTkLabel(
            self._liste, text=t("stats.loading"),
            font=("Outfit", 13), text_color=C["text2"],
        ).pack(pady=30)
        threading.Thread(target=self._calculer, daemon=True).start()

    # ── Interne ──────────────────────────────────────────────────────────────

    def _safe_after(self, *args, **kwargs):
        if self._destroyed:
            return
        try:
            if not self.winfo_exists():
                return
            self.after(*args, **kwargs)
        except RuntimeError:
            pass
        except Exception as e:
            log.warning(f"PanneauStatistiquesGlobales._safe_after: {e}")

    def _calculer(self):
        try:
            stats = get_stats_collection()
        except Exception as e:
            log.warning(f"PanneauStatistiquesGlobales._calculer: {e}")
            stats = []
        self._safe_after(0, self._render, stats)

    def _render(self, stats: list):
        self._stats_courantes = stats

        # Récap global — sur TOUTE la collection (pas le filtre).
        nb = len(stats)
        total = sum(s.get("total", 0) for s in stats)
        poss = sum(s.get("possedees", 0) for s in stats)
        pct = (poss / total * 100) if total > 0 else 0.0
        self._card_classeurs.update_value(str(nb))
        self._card_total.update_value(f"{total:,}")
        self._card_poss.update_value(f"{poss:,}")
        self._card_pct.update_value(f"{pct:.0f}%")

        from datetime import datetime
        self._lbl_statut.configure(
            text=f"{t('stats.updated')} {datetime.now().strftime('%H:%M:%S')}"
        )

        self._render_liste()

    def _render_liste(self):
        """(Re)construit la liste des classeurs selon la recherche courante."""
        q = self._search_var.get().strip().lower()
        stats = self._stats_courantes
        if q:
            stats = [s for s in stats if q in s.get("nom", "").lower()]

        for w in self._liste.winfo_children():
            w.destroy()

        if not self._stats_courantes:
            ctk.CTkLabel(
                self._liste, text=t("stats.no_binders"),
                font=("Georgia", 16, "bold"), text_color=C["text2"],
            ).pack(pady=40)
            return

        if not stats:
            ctk.CTkLabel(
                self._liste, text=t("search.no_results"),
                font=("Outfit", 13), text_color=C["text3"],
            ).pack(pady=40)
            return

        for stat in stats:
            self._render_ligne(stat)

    def _on_recherche(self, *_):
        """Anti-rebond avant de re-filtrer la liste."""
        if self._search_after:
            try:
                self.after_cancel(self._search_after)
            except Exception:
                pass
        self._search_after = self.after(200, self._render_liste)

    def _render_ligne(self, stat: dict):
        nom = stat.get("nom", "")
        total = stat.get("total", 0)
        poss = stat.get("possedees", 0)
        pct = stat.get("pourcentage", 0.0)
        raretes = stat.get("raretes", {}) or {}

        carte = ctk.CTkFrame(
            self._liste, fg_color=C["bg2"],
            border_color=C["border"], border_width=1, corner_radius=8,
        )
        carte.pack(fill="x", pady=4)

        # Ligne principale
        ligne = ctk.CTkFrame(carte, fg_color="transparent")
        ligne.pack(fill="x", padx=14, pady=10)

        ctk.CTkLabel(
            ligne, text=nom, width=230, anchor="w",
            font=("Outfit", 15, "bold"), text_color=C["text"],
        ).pack(side="left")

        barre = progress_bar(ligne, height=16)
        barre.pack(side="left", fill="x", expand=True, padx=10)
        try:
            barre.set(min(1.0, pct / 100))
        except Exception:
            barre.set(0)

        ctk.CTkLabel(
            ligne, text=f"{poss} / {total} ({pct:.0f}%)", width=185, anchor="e",
            font=("JetBrains Mono", 13), text_color=C["text2"],
        ).pack(side="left", padx=(0, 8))

        if raretes:
            btn = secondary_button(
                ligne, t("btn.details"),
                command=lambda n=nom, c=carte, r=raretes: self._toggle_details(n, c, r),
                width=110, font=("Outfit", 12),
            )
            btn.configure(height=34)
            btn.pack(side="left")

        # Détail rareté ré-ouvert si l'utilisateur l'avait déplié avant un refresh
        if nom in self._details_ouverts and raretes:
            self._afficher_details(carte, raretes)

    def _toggle_details(self, nom: str, carte: ctk.CTkFrame, raretes: dict):
        existant = getattr(carte, "_bloc_details", None)
        if existant is not None and existant.winfo_exists():
            existant.destroy()
            carte._bloc_details = None
            self._details_ouverts.discard(nom)
            return
        self._details_ouverts.add(nom)
        self._afficher_details(carte, raretes)

    def _afficher_details(self, carte: ctk.CTkFrame, raretes: dict):
        bloc = ctk.CTkFrame(carte, fg_color="transparent")
        bloc.pack(fill="x", padx=12, pady=(0, 8))
        carte._bloc_details = bloc

        separator(bloc).pack(fill="x", pady=(0, 6))

        items = sorted(
            raretes.items(),
            key=lambda kv: (_rang_rarete(kv[0]), kv[0].lower()),
        )
        for rarete, data in items:
            row = ctk.CTkFrame(bloc, fg_color="transparent")
            row.pack(fill="x", pady=2)
            ctk.CTkLabel(
                row, text=rarete, width=215, anchor="w",
                font=("Outfit", 12), text_color=C["text2"],
            ).pack(side="left")
            pb = progress_bar(row, height=12)
            pb.pack(side="left", fill="x", expand=True, padx=10)
            try:
                pb.set(min(1.0, data.get("pourcentage", 0) / 100))
            except Exception:
                pb.set(0)
            ctk.CTkLabel(
                row,
                text=(f"{data.get('possedees', 0)} / {data.get('total', 0)} "
                      f"({data.get('pourcentage', 0):.0f}%)"),
                width=175, anchor="e",
                font=("JetBrains Mono", 11), text_color=C["text3"],
            ).pack(side="left")

    def _on_destroy(self, event=None):
        if event is not None and event.widget is not self:
            return
        self._destroyed = True

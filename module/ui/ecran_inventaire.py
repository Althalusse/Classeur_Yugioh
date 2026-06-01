"""
ecran_inventaire.py — Écran « Inventaire » : toutes les cartes possédées.

Enveloppe d'écran (navbar + retour accueil) autour d'un tableau listant chaque
carte marquée comme possédée (possessed = 1) dans n'importe quel classeur. On y
ajuste l'ÉTAT de chaque carte : qualité et quantité, ou retrait de l'inventaire.

Choix technique — ttk.Treeview stylé
─────────────────────────────────────
Un inventaire peut contenir des milliers de lignes. CustomTkinter n'offre pas de
tableau virtualisé ; empiler des CTkFrame par ligne figerait l'UI. ttk.Treeview
est virtualisé et gère ces volumes sans effort. On l'habille (thème « clam » +
styles nommés) aux couleurs du thème global C pour rester cohérent visuellement
avec le reste de l'application CTk. Le thème ttk « clam » est requis sous Windows
pour que les couleurs de fond du Treeview soient effectivement appliquées.

Identification des lignes : (classeur, rowid). Même clé que dialog_carte, donc une
modification ici agit sur la même ligne que dans le visualiseur de classeur.

Enregistré dans NavigationController sous la clé « inventaire » ; chargé via
navigate_to("inventaire") → charger().
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
import tkinter as tk
from tkinter import ttk

import customtkinter as ctk

from module.theme import C, scaled_font
from module.ui.composants import (
    Navbar, StatCard, gold_button, secondary_button,
    styled_combobox, search_entry, separator,
)
from module.utilitaire.dialogs import afficher_warning
import module.inventaire.inventaire_service as svc
# Source unique des libellés de qualité (cohérence avec le visualiseur binder).
from module.ui.dialog_carte import QUALITE_OPTIONS, _QUALITE_MAP, _QUALITE_REV
from module.i18n import t
from module.logger_app import log


_TOUS = "(Tous)"

# Taille d'un playset : nombre d'exemplaires d'une même impression (carte +
# rareté) considéré comme « complet » pour le jeu. Au-delà, ce sont des surplus.
PLAYSET_SIZE = 3

# Colonnes du tableau : (clé logique, clé i18n de l'en-tête, largeur, ancrage).
# L'ancrage s'applique au CONTENU *et* à l'en-tête (cohérence visuelle) :
# texte → "w" (aligné à gauche), valeurs numériques → "center".
_COLONNES = [
    ("name",     "inv.card_name",     220, "w"),
    ("variante", "inv.col_variante",  140, "w"),
    ("set_name", "inv.col_set",       170, "w"),
    ("set_code", "inv.code_card",     110, "w"),
    ("rarity",   "rarity.col_name",   150, "w"),
    ("quantite", "inv.quantity_lbl",   80, "center"),
    ("playset",  "inv.col_playset",    80, "center"),
    ("surplus",  "inv.col_surplus",    80, "center"),
    ("qualite",  "inv.quality_lbl",   150, "w"),
    ("classeur", "anomaly.col_binder", 100, "w"),
]
# Index d'affichage Treeview (#1..#N) pour bbox lors de l'édition inline.
_COL_DISPLAY = {clef: f"#{i + 1}" for i, (clef, *_rest) in enumerate(_COLONNES)}

# Colonnes triées numériquement.
_COLONNES_NUM = {"quantite", "surplus"}


def _playset_surplus(quantite: int):
    """Retourne (libellé_playset_canon, surplus) pour une quantité donnée.

    - playset complet dès `quantite >= PLAYSET_SIZE`.
    - surplus = exemplaires au-delà d'un playset (jamais négatif).
    Le libellé Oui/Non est résolu à l'affichage via i18n.
    """
    complet = quantite >= PLAYSET_SIZE
    surplus = max(0, quantite - PLAYSET_SIZE)
    return complet, surplus


class EcranInventaire(ctk.CTkFrame):
    """Écran de gestion de l'inventaire global des cartes possédées."""

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to

        self._cartes: list[dict] = []        # toutes les cartes possédées
        self._iid_to_carte: dict[str, dict] = {}   # iid Treeview → dict carte
        self._sort_col = "name"
        self._sort_reverse = False
        self._inline_widget = None           # widget d'édition inline courant

        self._build()

    # ─────────────────────────────────────────────────────────────────────
    # Construction de l'interface
    # ─────────────────────────────────────────────────────────────────────

    def _build(self):
        Navbar(
            self,
            title=t("inv.title"),
            subtitle=t("inv.subtitle"),
            show_back=True,
            back_command=self._retour,
        ).pack(fill="x")

        # ── Bandeau statistiques ────────────────────────────────────────
        stats = ctk.CTkFrame(self, fg_color="transparent")
        stats.pack(fill="x", padx=24, pady=(18, 0))
        stats.columnconfigure((0, 1), weight=1)
        self._stat_uniques = StatCard(stats, "0", t("inv.total_cards"))
        self._stat_uniques.grid(row=0, column=0, padx=8, sticky="ew")
        self._stat_copies = StatCard(stats, "0", t("inv.total_copies"))
        self._stat_copies.grid(row=0, column=1, padx=8, sticky="ew")

        # ── Filtres ─────────────────────────────────────────────────────
        self._build_filtres()

        separator(self).pack(fill="x", padx=24, pady=(14, 0))

        # ── Tableau (ttk.Treeview stylé) ────────────────────────────────
        self._build_table()

        # ── Barre inférieure : compteur + actions de masse ──────────────
        self._build_barre_actions()

    def _build_filtres(self):
        bar = ctk.CTkFrame(self, fg_color="transparent")
        bar.pack(fill="x", padx=24, pady=(16, 0))

        def groupe(parent):
            """Petit conteneur vertical : libellé en haut, contrôle en bas."""
            g = ctk.CTkFrame(parent, fg_color="transparent")
            g.pack(side="left", padx=(0, 10))
            return g

        def caption(parent, key):
            ctk.CTkLabel(
                parent, text=t(key), font=scaled_font("Outfit", 10),
                text_color=C["text3"], anchor="w",
            ).pack(anchor="w", pady=(0, 2))

        # Recherche par nom
        g = groupe(bar)
        caption(g, "inv.f_name")
        self._nom_var = ctk.StringVar()
        # NB : avec un textvariable lié, CTkEntry n'affiche pas son
        # placeholder — d'où le libellé explicite au-dessus.
        search_entry(
            g, textvariable=self._nom_var,
            placeholder=t("inv.search_name"), width=220,
        ).pack(anchor="w")
        self._nom_var.trace_add("write", self._on_filtre_change)

        # Recherche par code
        g = groupe(bar)
        caption(g, "inv.f_code")
        self._code_var = ctk.StringVar()
        search_entry(
            g, textvariable=self._code_var,
            placeholder=t("inv.search_code"), width=140,
        ).pack(anchor="w")
        self._code_var.trace_add("write", self._on_filtre_change)

        # Combos (remplis dynamiquement au chargement)
        g = groupe(bar)
        caption(g, "inv.f_binder")
        self._classeur_var = ctk.StringVar(value=_TOUS)
        self._combo_classeur = styled_combobox(
            g, values=[_TOUS], variable=self._classeur_var, width=150,
            state="readonly", command=lambda _v: self._appliquer_filtres(),
        )
        self._combo_classeur.pack(anchor="w")

        g = groupe(bar)
        caption(g, "inv.f_set")
        self._set_var = ctk.StringVar(value=_TOUS)
        self._combo_set = styled_combobox(
            g, values=[_TOUS], variable=self._set_var, width=180,
            state="readonly", command=lambda _v: self._appliquer_filtres(),
        )
        self._combo_set.pack(anchor="w")

        g = groupe(bar)
        caption(g, "inv.f_rarity")
        self._rarete_var = ctk.StringVar(value=_TOUS)
        self._combo_rarete = styled_combobox(
            g, values=[_TOUS], variable=self._rarete_var, width=150,
            state="readonly", command=lambda _v: self._appliquer_filtres(),
        )
        self._combo_rarete.pack(anchor="w")

        g = groupe(bar)
        caption(g, "inv.f_quality")
        self._qualite_var = ctk.StringVar(value=_TOUS)
        self._combo_qualite = styled_combobox(
            g, values=[_TOUS], variable=self._qualite_var, width=160,
            state="readonly", command=lambda _v: self._appliquer_filtres(),
        )
        self._combo_qualite.pack(anchor="w")

        # Bouton aligné en bas du groupe (sous une ligne de libellé vide pour
        # le mettre à la même hauteur que les contrôles).
        g = groupe(bar)
        ctk.CTkLabel(g, text=" ", font=scaled_font("Outfit", 10)).pack(
            anchor="w", pady=(0, 2))
        secondary_button(
            g, t("btn.reset_filters"), command=self._reset_filtres, width=130,
        ).pack(anchor="w")

    def _build_table(self):
        # Conteneur CTk qui héberge le Treeview ttk + scrollbars.
        wrap = ctk.CTkFrame(self, fg_color=C["bg2"], corner_radius=8)
        wrap.pack(fill="both", expand=True, padx=24, pady=(14, 0))
        wrap.grid_rowconfigure(0, weight=1)
        wrap.grid_columnconfigure(0, weight=1)

        self._configurer_style_treeview()

        cols = tuple(clef for clef, *_ in _COLONNES)
        self._tree = ttk.Treeview(
            wrap, columns=cols, show="headings",
            selectmode="extended", style="Inventaire.Treeview",
        )
        for clef, i18n_key, width, anchor in _COLONNES:
            self._tree.heading(
                clef, text=t(i18n_key), anchor=anchor,
                command=lambda c=clef: self._sort_by(c),
            )
            self._tree.column(clef, width=width, minwidth=50, anchor=anchor,
                             stretch=False)

        yscroll = ttk.Scrollbar(wrap, orient="vertical",
                                command=self._tree.yview)
        xscroll = ttk.Scrollbar(wrap, orient="horizontal",
                                command=self._tree.xview)
        self._tree.configure(yscrollcommand=yscroll.set,
                             xscrollcommand=xscroll.set)
        self._tree.grid(row=0, column=0, sticky="nsew", padx=(8, 0), pady=8)
        yscroll.grid(row=0, column=1, sticky="ns", pady=8, padx=(0, 8))
        xscroll.grid(row=1, column=0, sticky="ew", padx=8, pady=(0, 8))

        # Interactions
        self._tree.bind("<Double-1>", self._on_double_click)
        self._tree.bind("<Button-3>", self._on_right_click)
        self._tree.bind("<<TreeviewSelect>>", lambda e: self._maj_barre_actions())
        # Tout clic/défilement ferme une édition inline ouverte.
        self._tree.bind("<Button-1>", self._fermer_inline, add="+")
        self._tree.bind("<MouseWheel>", self._fermer_inline, add="+")

    def _configurer_style_treeview(self):
        """Habille le Treeview ttk aux couleurs du thème global.

        On bascule le thème ttk sur « clam » : sous Windows, les thèmes natifs
        (« vista ») ignorent les couleurs de fond du Treeview. « clam » les
        respecte. Cet écran étant le seul à utiliser un Treeview dans l'app
        active, le changement de thème global est sans effet de bord visible.
        """
        style = ttk.Style()
        try:
            style.theme_use("clam")
        except Exception:
            pass

        row_h = max(24, scaled_font("Segoe UI", 11)[1] * 2 + 6)
        style.configure(
            "Inventaire.Treeview",
            background=C["bg2"],
            foreground=C["text"],
            fieldbackground=C["bg2"],
            rowheight=row_h,
            borderwidth=0,
            font=scaled_font("Segoe UI", 11),
        )
        style.configure(
            "Inventaire.Treeview.Heading",
            background=C["bg3"],
            foreground=C["gold"],
            font=scaled_font("Segoe UI", 11, "bold"),
            relief="flat",
            borderwidth=0,
            padding=(8, 6),
        )
        style.map(
            "Inventaire.Treeview",
            background=[("selected", C["gold"])],
            foreground=[("selected", "#000000")],
        )
        style.map(
            "Inventaire.Treeview.Heading",
            background=[("active", C["bg_hover"])],
            foreground=[("active", C["gold_hover"])],
        )

    def _build_barre_actions(self):
        bar = ctk.CTkFrame(self, fg_color="transparent")
        bar.pack(fill="x", padx=24, pady=(8, 16))

        self._lbl_compteur = ctk.CTkLabel(
            bar, text="", font=scaled_font("Outfit", 11),
            text_color=C["text3"],
        )
        self._lbl_compteur.pack(side="left")

        self._btn_retirer = secondary_button(
            bar, t("inv.remove"), command=self._retirer_selection, width=200,
        )
        self._btn_retirer.pack(side="right", padx=(8, 0))
        self._btn_quantite = secondary_button(
            bar, t("inv.set_quantity"), command=self._dialog_quantite_masse,
            width=190,
        )
        self._btn_quantite.pack(side="right", padx=(8, 0))
        self._btn_qualite = gold_button(
            bar, t("inv.set_quality"), command=self._dialog_qualite_masse,
            width=190,
        )
        self._btn_qualite.pack(side="right", padx=(8, 0))
        self._maj_barre_actions()

    # ─────────────────────────────────────────────────────────────────────
    # Chargement des données (thread → UI)
    # ─────────────────────────────────────────────────────────────────────

    def charger(self):
        """Appelé par NavigationController à l'affichage de l'écran."""
        self._fermer_inline()
        threading.Thread(target=self._load_data, daemon=True).start()

    def _load_data(self):
        try:
            cartes = svc.get_cartes_possedees()
        except Exception as e:
            log.warning(f"EcranInventaire._load_data: {e}")
            cartes = []
        self._safe_after(0, self._render, cartes)

    def _render(self, cartes: list):
        self._cartes = cartes

        # Statistiques globales (toute la collection, pas le filtre)
        total_uniques = len(cartes)
        total_copies = sum(c.get("quantite", 0) for c in cartes)
        self._stat_uniques.update_value(f"{total_uniques:,}")
        self._stat_copies.update_value(f"{total_copies:,}")

        # Alimente les combos de filtre à partir des données présentes
        self._remplir_combos()

        # Affiche le tableau filtré
        self._appliquer_filtres()

    def _remplir_combos(self):
        def vals(champ):
            return sorted({(c.get(champ) or "") for c in self._cartes
                           if (c.get(champ) or "")})

        self._maj_combo(self._combo_classeur, self._classeur_var,
                        [_TOUS] + vals("classeur"))
        self._maj_combo(self._combo_set, self._set_var,
                        [_TOUS] + vals("set_name"))
        self._maj_combo(self._combo_rarete, self._rarete_var,
                        [_TOUS] + vals("rarity"))

        # Qualités : valeurs canoniques présentes → libellés d'affichage.
        qualites_presentes = sorted(
            {(c.get("qualite") or "") for c in self._cartes}
        )
        options_q = [_TOUS]
        if "" in qualites_presentes:
            options_q.append(t("inv.quality_none"))
        for canon in qualites_presentes:
            if canon:
                options_q.append(_QUALITE_REV.get(canon, canon))
        self._maj_combo(self._combo_qualite, self._qualite_var, options_q)

    @staticmethod
    def _maj_combo(combo, var, values):
        """Met à jour les valeurs d'un combo en préservant la sélection si
        elle existe toujours, sinon retombe sur la 1ère valeur ((Tous))."""
        current = var.get()
        combo.configure(values=values)
        var.set(current if current in values else values[0])

    # ─────────────────────────────────────────────────────────────────────
    # Filtrage / rendu du tableau
    # ─────────────────────────────────────────────────────────────────────

    def _qualite_filtre_canon(self) -> str:
        """Convertit la valeur du combo qualité en critère pour le service."""
        disp = self._qualite_var.get()
        if disp == _TOUS or not disp:
            return ""
        if disp == t("inv.quality_none"):
            return "__VIDE__"
        return _QUALITE_MAP.get(disp, disp)

    def _appliquer_filtres(self):
        self._fermer_inline()
        filtrees = svc.filtrer_cartes(
            self._cartes,
            rarete=self._rarete_var.get(),
            code=self._code_var.get(),
            set_name=self._set_var.get(),
            classeur=self._classeur_var.get(),
            qualite=self._qualite_filtre_canon(),
            nom=self._nom_var.get(),
            valeur_tous=_TOUS,
        )
        self._remplir_tree(filtrees)

    def _remplir_tree(self, cartes: list):
        # Conserve la sélection logique (classeur, rowid) à travers le refill.
        sel_keys = {
            (c["classeur"], c["rowid"])
            for iid in self._tree.selection()
            if (c := self._iid_to_carte.get(iid))
        }

        self._tree.delete(*self._tree.get_children())
        self._iid_to_carte = {}

        nouveaux_iids = []
        for c in cartes:
            iid = self._tree.insert("", "end", values=self._row_values(c))
            self._iid_to_carte[iid] = c
            if (c["classeur"], c["rowid"]) in sel_keys:
                nouveaux_iids.append(iid)

        if nouveaux_iids:
            self._tree.selection_set(nouveaux_iids)

        self._appliquer_tri_courant()

        n = len(cartes)
        self._lbl_compteur.configure(text=t("inv.shown_count", n=f"{n:,}"))
        self._maj_barre_actions()

    @staticmethod
    def _variante_label(c: dict) -> str:
        """Contenu de la colonne « Variante » : « Art 2 », « Overframe », ou
        « Art 2 · Overframe » (vide pour une impression normale unique)."""
        parts = []
        if c.get("n_arts", 1) > 1 and c.get("art_rank"):
            parts.append(t("inv.art_n", n=c["art_rank"]))
        if c.get("overframe"):
            parts.append(t("inv.overframe"))
        return " · ".join(parts)

    def _row_values(self, c: dict) -> tuple:
        canon = c.get("qualite") or ""
        qual_disp = _QUALITE_REV.get(canon, canon)
        qte = c.get("quantite", 0)
        complet, surplus = _playset_surplus(qte)
        playset_disp = t("inv.yes") if complet else t("inv.no")
        return (
            c.get("name", ""),
            self._variante_label(c),
            c.get("set_name", ""),
            c.get("set_code", ""),
            c.get("rarity", ""),
            qte,
            playset_disp,
            surplus,
            qual_disp,
            c.get("classeur", ""),
        )

    def _maj_ligne_quantite(self, iid: str, qte: int):
        """Met à jour, pour une ligne, la quantité ET les colonnes dérivées
        (Playset / Surplus) afin qu'elles restent cohérentes après édition."""
        complet, surplus = _playset_surplus(qte)
        self._tree.set(iid, "quantite", qte)
        self._tree.set(iid, "playset", t("inv.yes") if complet else t("inv.no"))
        self._tree.set(iid, "surplus", surplus)

    def _on_filtre_change(self, *_):
        # Les entrées texte filtrent en direct (les listes sont déjà en mémoire,
        # donc pas besoin d'anti-rebond agressif).
        self._appliquer_filtres()

    def _reset_filtres(self):
        self._nom_var.set("")
        self._code_var.set("")
        self._classeur_var.set(_TOUS)
        self._set_var.set(_TOUS)
        self._rarete_var.set(_TOUS)
        self._qualite_var.set(_TOUS)
        self._appliquer_filtres()

    # ─────────────────────────────────────────────────────────────────────
    # Tri par colonne
    # ─────────────────────────────────────────────────────────────────────

    def _sort_by(self, col: str):
        if self._sort_col == col:
            self._sort_reverse = not self._sort_reverse
        else:
            self._sort_col = col
            self._sort_reverse = False
        self._appliquer_tri_courant()

    def _appliquer_tri_courant(self):
        col = self._sort_col
        numerique = col in _COLONNES_NUM

        def cle(iid):
            val = self._tree.set(iid, col)
            if numerique:
                try:
                    return (0, int(val))
                except (TypeError, ValueError):
                    return (0, 0)
            return (0, str(val).lower())

        items = list(self._tree.get_children(""))
        items.sort(key=cle, reverse=self._sort_reverse)
        for index, iid in enumerate(items):
            self._tree.move(iid, "", index)

        # Flèche de tri dans l'en-tête
        for clef, i18n_key, *_ in _COLONNES:
            arrow = ""
            if clef == col:
                arrow = " ▲" if not self._sort_reverse else " ▼"
            self._tree.heading(clef, text=t(i18n_key) + arrow)

    # ─────────────────────────────────────────────────────────────────────
    # Édition inline (double-clic)
    # ─────────────────────────────────────────────────────────────────────

    def _fermer_inline(self, event=None):
        if self._inline_widget is not None:
            try:
                self._inline_widget.destroy()
            except Exception:
                pass
            self._inline_widget = None

    def _on_double_click(self, event):
        self._fermer_inline()
        iid = self._tree.identify_row(event.y)
        col_disp = self._tree.identify_column(event.x)
        if not iid:
            return
        carte = self._iid_to_carte.get(iid)
        if not carte:
            return

        if col_disp == _COL_DISPLAY["quantite"]:
            self._editer_quantite_inline(iid, carte, col_disp)
        elif col_disp == _COL_DISPLAY["qualite"]:
            self._editer_qualite_inline(iid, carte, col_disp)

    def _bbox_or_none(self, iid, col_disp):
        try:
            return self._tree.bbox(iid, col_disp)
        except Exception:
            return None

    def _editer_quantite_inline(self, iid, carte, col_disp):
        box = self._bbox_or_none(iid, col_disp)
        if not box:
            return
        x, y, w, h = box
        entry = tk.Entry(self._tree, justify="center")
        entry.place(x=x, y=y, width=w, height=h)
        entry.insert(0, str(carte.get("quantite", 1)))
        entry.select_range(0, tk.END)
        entry.focus_set()
        self._inline_widget = entry

        def valider(_e=None):
            # Garde : si l'éditeur a déjà été fermé (clic ailleurs détruisant
            # le widget), on ne tente pas de lire un widget détruit.
            if entry is not self._inline_widget:
                return
            raw = entry.get().strip()
            try:
                qte = int(raw)
            except ValueError:
                afficher_warning(t("inv.qty_int_required"), t("err.title"))
                return
            if qte < 1:
                afficher_warning(t("inv.qty_min_1"), t("err.title"))
                return
            self._fermer_inline()
            if svc.set_quantite(carte["classeur"], carte["rowid"], qte):
                carte["quantite"] = qte
                self._maj_ligne_quantite(iid, qte)
                self._maj_stats_apres_edition()

        entry.bind("<Return>", valider)
        entry.bind("<Escape>", lambda e: self._fermer_inline())
        entry.bind("<FocusOut>", valider)

    def _editer_qualite_inline(self, iid, carte, col_disp):
        box = self._bbox_or_none(iid, col_disp)
        if not box:
            return
        x, y, w, h = box
        combo = ttk.Combobox(self._tree, values=QUALITE_OPTIONS,
                             state="readonly")
        combo.place(x=x, y=y, width=max(w, 150), height=h)
        canon = carte.get("qualite") or ""
        combo.set(_QUALITE_REV.get(canon, ""))
        combo.focus_set()
        self._inline_widget = combo

        def valider(_e=None):
            if combo is not self._inline_widget:
                return
            disp = combo.get()
            nouveau_canon = _QUALITE_MAP.get(disp, "")
            self._fermer_inline()
            if svc.set_qualite(carte["classeur"], carte["rowid"], nouveau_canon):
                carte["qualite"] = nouveau_canon
                self._tree.set(iid, "qualite",
                               _QUALITE_REV.get(nouveau_canon, nouveau_canon))

        combo.bind("<<ComboboxSelected>>", valider)
        combo.bind("<Escape>", lambda e: self._fermer_inline())

    # ─────────────────────────────────────────────────────────────────────
    # Sélection multiple — actions de masse
    # ─────────────────────────────────────────────────────────────────────

    def _cartes_selectionnees(self) -> list:
        return [self._iid_to_carte[iid] for iid in self._tree.selection()
                if iid in self._iid_to_carte]

    def _maj_barre_actions(self):
        n = len(self._tree.selection())
        etat = "normal" if n > 0 else "disabled"
        for b in (self._btn_qualite, self._btn_quantite, self._btn_retirer):
            try:
                b.configure(state=etat)
            except Exception:
                pass

    def _on_right_click(self, event):
        iid = self._tree.identify_row(event.y)
        if iid and iid not in self._tree.selection():
            self._tree.selection_set(iid)
        if not self._tree.selection():
            return

        n = len(self._tree.selection())
        menu = tk.Menu(self._tree, tearoff=0)
        menu.add_command(
            label=t("inv.n_selected", n=n), state="disabled",
        )
        menu.add_separator()

        sous_qualite = tk.Menu(menu, tearoff=0)
        for disp in QUALITE_OPTIONS:
            if disp == "":
                continue
            canon = _QUALITE_MAP.get(disp, disp)
            sous_qualite.add_command(
                label=disp,
                command=lambda c=canon: self._appliquer_qualite_masse(c),
            )
        sous_qualite.add_separator()
        sous_qualite.add_command(
            label=t("inv.quality_clear"),
            command=lambda: self._appliquer_qualite_masse(""),
        )
        menu.add_cascade(label=t("inv.set_quality"), menu=sous_qualite)
        menu.add_command(label=t("inv.set_quantity"),
                         command=self._dialog_quantite_masse)
        menu.add_separator()
        menu.add_command(label=t("inv.remove"),
                         command=self._retirer_selection)

        try:
            menu.tk_popup(event.x_root, event.y_root)
        finally:
            menu.grab_release()

    def _dialog_qualite_masse(self):
        cartes = self._cartes_selectionnees()
        if not cartes:
            return
        DialogChoixQualite(
            self.winfo_toplevel(), len(cartes),
            on_valide=self._appliquer_qualite_masse,
        )

    def _appliquer_qualite_masse(self, canon: str):
        disp = _QUALITE_REV.get(canon, canon)
        for iid in self._tree.selection():
            c = self._iid_to_carte.get(iid)
            if not c:
                continue
            if svc.set_qualite(c["classeur"], c["rowid"], canon):
                c["qualite"] = canon
                self._tree.set(iid, "qualite", disp)

    def _dialog_quantite_masse(self):
        cartes = self._cartes_selectionnees()
        if not cartes:
            return
        DialogChoixQuantite(
            self.winfo_toplevel(), len(cartes),
            on_valide=self._appliquer_quantite_masse,
        )

    def _appliquer_quantite_masse(self, qte: int):
        for iid in self._tree.selection():
            c = self._iid_to_carte.get(iid)
            if not c:
                continue
            if svc.set_quantite(c["classeur"], c["rowid"], qte):
                c["quantite"] = qte
                self._maj_ligne_quantite(iid, qte)
        self._maj_stats_apres_edition()

    def _retirer_selection(self):
        cartes = self._cartes_selectionnees()
        if not cartes:
            return
        DialogConfirmRetrait(
            self.winfo_toplevel(), len(cartes),
            on_confirm=lambda: self._retirer_confirme(list(cartes)),
        )

    def _retirer_confirme(self, cartes: list):
        for c in cartes:
            if svc.retirer_de_inventaire(c["classeur"], c["rowid"]):
                try:
                    self._cartes.remove(c)
                except ValueError:
                    pass
        # Rebuild complet : les lignes retirées disparaissent + stats à jour.
        self._stat_uniques.update_value(f"{len(self._cartes):,}")
        self._stat_copies.update_value(
            f"{sum(x.get('quantite', 0) for x in self._cartes):,}"
        )
        self._appliquer_filtres()

    def _maj_stats_apres_edition(self):
        """Recalcule le nombre d'exemplaires (les uniques ne changent pas ici)."""
        self._stat_copies.update_value(
            f"{sum(c.get('quantite', 0) for c in self._cartes):,}"
        )

    # ─────────────────────────────────────────────────────────────────────
    # Divers
    # ─────────────────────────────────────────────────────────────────────

    def _retour(self):
        self._fermer_inline()
        if self._navigate_to:
            self._navigate_to("accueil")

    def _safe_after(self, *args, **kwargs):
        """Wrapper défensif autour de self.after() depuis un thread worker."""
        try:
            if not self.winfo_exists():
                return
        except Exception:
            return
        try:
            self.after(*args, **kwargs)
        except RuntimeError:
            pass
        except Exception as e:
            log.warning(f"EcranInventaire._safe_after: {e}")


# ─────────────────────────────────────────────────────────────────────────────
# Petites fenêtres modales (qualité / quantité / confirmation retrait)
# ─────────────────────────────────────────────────────────────────────────────

class _BaseDialog(ctk.CTkToplevel):
    """Toplevel CTk centrée sur la fenêtre racine, fermable par Échap."""

    def __init__(self, parent, w: int, h: int, titre: str):
        super().__init__(parent)
        self.title(titre)
        self.geometry(f"{w}x{h}")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()
        try:
            self.update_idletasks()
            x = parent.winfo_x() + (parent.winfo_width() - w) // 2
            y = parent.winfo_y() + (parent.winfo_height() - h) // 2
            self.geometry(f"+{x}+{y}")
        except Exception:
            pass
        self.bind("<Escape>", lambda e: self.destroy())


class DialogChoixQualite(_BaseDialog):
    """Choix d'une qualité à appliquer à la sélection."""

    def __init__(self, parent, nb: int, on_valide):
        super().__init__(parent, 320, 200, t("inv.set_quality"))
        self._on_valide = on_valide

        ctk.CTkLabel(
            self, text=t("inv.quality_for", n=nb),
            font=scaled_font("Outfit", 12), text_color=C["text"],
        ).pack(pady=(20, 10))

        self._var = ctk.StringVar(value="")
        styled_combobox(
            self, values=QUALITE_OPTIONS, variable=self._var,
            width=240, state="readonly",
        ).pack(pady=4)

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=18)
        secondary_button(row, t("btn.cancel"), command=self.destroy,
                         width=110).pack(side="left", padx=6)
        gold_button(row, t("btn.apply"), command=self._appliquer,
                    width=120).pack(side="left", padx=6)

    def _appliquer(self):
        disp = self._var.get()
        canon = _QUALITE_MAP.get(disp, "")
        self.destroy()
        self._on_valide(canon)


class DialogChoixQuantite(_BaseDialog):
    """Saisie d'une quantité à appliquer à la sélection."""

    def __init__(self, parent, nb: int, on_valide):
        super().__init__(parent, 320, 190, t("inv.set_quantity"))
        self._on_valide = on_valide

        ctk.CTkLabel(
            self, text=t("inv.quantity_for", n=nb),
            font=scaled_font("Outfit", 12), text_color=C["text"],
        ).pack(pady=(20, 10))

        self._var = ctk.StringVar(value="1")
        entry = ctk.CTkEntry(
            self, textvariable=self._var, width=100, justify="center",
            fg_color=C["bg3"], border_color=C["border2"], border_width=1,
            text_color=C["text"], font=scaled_font("JetBrains Mono", 13),
        )
        entry.pack(pady=4)
        entry.focus_set()
        entry.bind("<Return>", lambda e: self._appliquer())

        self._err = ctk.CTkLabel(self, text="", font=scaled_font("Outfit", 10),
                                 text_color=C["danger_text"])
        self._err.pack()

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=12)
        secondary_button(row, t("btn.cancel"), command=self.destroy,
                         width=110).pack(side="left", padx=6)
        gold_button(row, t("btn.apply"), command=self._appliquer,
                    width=120).pack(side="left", padx=6)

    def _appliquer(self):
        try:
            qte = int(self._var.get().strip())
        except ValueError:
            self._err.configure(text=t("inv.qty_int_required"))
            return
        if qte < 1:
            self._err.configure(text=t("inv.qty_min_1"))
            return
        self.destroy()
        self._on_valide(qte)


class DialogConfirmRetrait(_BaseDialog):
    """Confirmation avant de retirer des cartes de l'inventaire."""

    def __init__(self, parent, nb: int, on_confirm):
        super().__init__(parent, 380, 200, t("inv.remove"))
        self._on_confirm = on_confirm

        ctk.CTkLabel(
            self, text=t("inv.remove_title"),
            font=scaled_font("Georgia", 14, "bold"), text_color=C["text"],
        ).pack(pady=(20, 8))
        ctk.CTkLabel(
            self, text=t("inv.remove_confirm", n=nb),
            font=scaled_font("Outfit", 11), text_color=C["text2"],
            wraplength=340, justify="center",
        ).pack(pady=(0, 4))

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=20)
        secondary_button(row, t("btn.cancel"), command=self.destroy,
                         width=120).pack(side="left", padx=8)
        gold_button(row, t("btn.confirm"),
                    command=self._confirmer, width=120).pack(side="left", padx=8)

    def _confirmer(self):
        self.destroy()
        self._on_confirm()

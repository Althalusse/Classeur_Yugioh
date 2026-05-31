"""
anomalie_ui.py — Onglet "Anomalies API".

Sélection multiple : clic, Shift+clic, Ctrl+clic, glisser-déposer.
Clic droit : menu contextuel pour corriger / annuler.
Tri par défaut : set_code croissant.
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

import tkinter as tk
from tkinter import ttk

from module.utilitaire.dialogs import afficher_info, afficher_warning
import module.anomalie.anomalie_service as svc
from module.i18n import t


class AnomalieFrame:
    def __init__(self, parent, refresh_callback=None):
        self.parent = parent
        self.refresh_callback = refresh_callback
        self._anomalies_by_id = {}
        self._sort_col     = "set_code"
        self._sort_reverse = False
        self._drag_start   = None
        self._build_ui()
        self._charger()

    # ─────────────────────────────────────────────────────────────────────────
    # Construction UI
    # ─────────────────────────────────────────────────────────────────────────

    def _emit_changed(self):
        """Émet <<ClasseursChanged>> sur la fenêtre racine en plus du callback local."""
        try:
            self.parent.winfo_toplevel().event_generate("<<ClasseursChanged>>")
        except Exception:
            pass
        if self.refresh_callback:
            self.refresh_callback()

    def _build_ui(self):
        self.parent.columnconfigure(0, weight=1)
        self.parent.rowconfigure(1, weight=1)

        # ── Barre supérieure ─────────────────────────────────────────────────
        top = ttk.Frame(self.parent)
        top.grid(row=0, column=0, columnspan=2, sticky="ew", padx=6, pady=(6, 2))

        ttk.Label(top, text="Classeur :").pack(side="left", padx=(0, 4))
        self.combo_prefix = ttk.Combobox(top, width=14, state="readonly")
        self.combo_prefix.pack(side="left", padx=(0, 8))
        self.combo_prefix.bind("<<ComboboxSelected>>", lambda e: self._charger())

        ttk.Button(top, text=t("anomaly.btn_scan"), command=self._scanner).pack(side="left", padx=4)

        self.lbl_info = ttk.Label(top, text="", foreground="gray")
        self.lbl_info.pack(side="left", padx=12, fill="x", expand=True)

        # ── Treeview ─────────────────────────────────────────────────────────
        cols = (t("inv.col_binder"), "carte", "art", "set_code", "rarete", "statut")
        self.tree = ttk.Treeview(
            self.parent, columns=cols, show="headings",
            selectmode="extended", height=22
        )
        self.tree.heading(t("inv.col_binder"), text=t("anomaly.col_binder"),     command=lambda: self._sort_by(t("inv.col_binder")))
        self.tree.heading("carte",    text=t("anomaly.col_card"),        command=lambda: self._sort_by("carte"))
        self.tree.heading("art",      text="Art",          command=lambda: self._sort_by("art"))
        self.tree.heading("set_code", text="Set manquant ▲", command=lambda: self._sort_by("set_code"))
        self.tree.heading("rarete",   text=t("rarity.col_name"), command=lambda: self._sort_by("rarete"))
        self.tree.heading("statut",   text=t("anomaly.col_status"),       command=lambda: self._sort_by("statut"))
        self.tree.column(t("inv.col_binder"),  width=80,  anchor="center")
        self.tree.column("carte",     width=200)
        self.tree.column("art",       width=50,  anchor="center")
        self.tree.column("set_code",  width=120)
        self.tree.column("rarete",    width=160)
        self.tree.column("statut",    width=90,  anchor="center")
        self.tree.grid(row=1, column=0, sticky="nsew", padx=6, pady=2)

        # Sélection native + glisser
        self.tree.bind("<<TreeviewSelect>>",   self._on_selection_change)
        self.tree.bind("<ButtonPress-1>",      self._on_drag_start)
        self.tree.bind("<B1-Motion>",          self._on_drag_motion)
        self.tree.bind("<ButtonRelease-1>",    self._on_drag_release)
        # Clic droit
        self.tree.bind("<Button-3>",           self._on_right_click)

        sb = ttk.Scrollbar(self.parent, orient="vertical", command=self.tree.yview)
        self.tree.configure(yscrollcommand=sb.set)
        sb.grid(row=1, column=1, sticky="ns", pady=2)
        self.parent.columnconfigure(1, weight=0)

        # ── Barre inférieure ─────────────────────────────────────────────────
        self.lbl_selection = ttk.Label(
            self.parent, text=t("anomaly.none_selected"), foreground="gray"
        )
        self.lbl_selection.grid(row=2, column=0, sticky="w", padx=8, pady=(2, 0))

        bot = ttk.Frame(self.parent)
        bot.grid(row=3, column=0, columnspan=2, sticky="ew", padx=6, pady=(4, 6))
        ttk.Button(bot, text=t("anomaly.btn_fix_sel"),
                   command=self._corriger_selection).pack(side="left", padx=4)
        ttk.Button(bot, text=t("btn.correct_all"),
                   command=self._corriger_tout).pack(side="left", padx=4)
        ttk.Button(bot, text=t("anomaly.btn_cancel_sel"),
                   command=self._annuler_selection).pack(side="right", padx=4)

    # ─────────────────────────────────────────────────────────────────────────
    # Données
    # ─────────────────────────────────────────────────────────────────────────

    def _charger(self):
        prefix = self.combo_prefix.get()
        filtre = prefix if prefix and prefix != "Tous" else None
        anomalies = svc.lire_anomalies(prefix_filtre=filtre)
        self._anomalies_by_id = {a["id"]: a for a in anomalies}
        # Tri par défaut : set_code croissant
        anomalies.sort(key=lambda a: a.get("missing_set_code", "").lower())
        self._remplir_tree(anomalies)
        self._refresh_combo_prefixes()

    def _refresh_combo_prefixes(self):
        prefixes = ["Tous"] + svc.lire_prefixes_avec_anomalies()
        current = self.combo_prefix.get()
        self.combo_prefix["values"] = prefixes
        if current not in prefixes:
            self.combo_prefix.set("Tous")

    def _scanner(self):
        self.lbl_info.config(text="Scan en cours...", foreground="orange")
        self.parent.update()
        try:
            nb = svc.scanner_et_stocker_anomalies()
            self._charger()
            self.lbl_info.config(
                text=f"Scan terminé — {nb} anomalie(s) en base.",
                foreground="gray"
            )
        except Exception as e:
            self.lbl_info.config(text=f"Erreur : {e}", foreground="red")

    def _remplir_tree(self, anomalies: list):
        self.tree.delete(*self.tree.get_children())
        for a in anomalies:
            corrige = bool(a.get("corrige"))
            statut  = t("anomaly.fixed") if corrige else t("anomaly.missing")
            tag     = "corrige" if corrige else "manquant"
            self.tree.insert("", "end",
                values=(
                    a["set_code_prefix"],
                    a["name"],
                    f"Art {a['art_index']}",
                    a["missing_set_code"],
                    a["missing_set_rarity"],
                    statut,
                ),
                tags=(tag, str(a["id"]))
            )
        self.tree.tag_configure("corrige",  foreground="green")
        self.tree.tag_configure("manquant", foreground="orange")

        nb_total   = len(anomalies)
        nb_corrige = sum(1 for a in anomalies if a.get("corrige"))
        self.lbl_info.config(
            text=f"{nb_total} anomalie(s) — {nb_corrige} corrigée(s),"
                 f" {nb_total - nb_corrige} restante(s).",
            foreground="gray"
        )
        self._update_selection_label()

    # ─────────────────────────────────────────────────────────────────────────
    # Sélection & glisser
    # ─────────────────────────────────────────────────────────────────────────

    def _on_selection_change(self, event=None):
        self._update_selection_label()

    def _update_selection_label(self):
        n = len(self.tree.selection())
        self.lbl_selection.config(text=f"{n} ligne(s) sélectionnée(s)")

    def _on_drag_start(self, event):
        item = self.tree.identify_row(event.y)
        if not item:
            return
        self._drag_start = item
        # Si Ctrl/Shift non enfoncés, sélection simple
        if not (event.state & 0x0004) and not (event.state & 0x0001):
            self.tree.selection_set(item)

    def _on_drag_motion(self, event):
        if not self._drag_start:
            return
        target = self.tree.identify_row(event.y)
        if not target:
            return
        items = self.tree.get_children()
        if self._drag_start not in items or target not in items:
            return
        i0 = items.index(self._drag_start)
        i1 = items.index(target)
        lo, hi = min(i0, i1), max(i0, i1)
        self.tree.selection_set(items[lo:hi + 1])

    def _on_drag_release(self, event):
        self._drag_start = None

    def _get_selected_anomalies(self) -> list:
        result = []
        for item in self.tree.selection():
            try:
                aid = int(self.tree.item(item)["tags"][-1])
                if aid in self._anomalies_by_id:
                    result.append(self._anomalies_by_id[aid])
            except (ValueError, IndexError):
                pass
        return result

    # ─────────────────────────────────────────────────────────────────────────
    # Clic droit — menu contextuel
    # ─────────────────────────────────────────────────────────────────────────

    def _on_right_click(self, event):
        item = self.tree.identify_row(event.y)
        if not item:
            return
        # Si l'item cliqué n'est pas dans la sélection, on le sélectionne seul
        if item not in self.tree.selection():
            self.tree.selection_set(item)

        selection = self._get_selected_anomalies()
        n = len(selection)

        menu = tk.Menu(self.tree, tearoff=0)
        menu.add_command(
            label=f"✅ Corriger {n} ligne(s)",
            command=self._corriger_selection
        )
        menu.add_command(
            label=f"🗑 Annuler correction {n} ligne(s)",
            command=self._annuler_selection
        )
        menu.add_separator()
        menu.add_command(
            label=t("btn.correct_all"),
            command=self._corriger_tout
        )
        menu.tk_popup(event.x_root, event.y_root)

    # ─────────────────────────────────────────────────────────────────────────
    # Tri
    # ─────────────────────────────────────────────────────────────────────────

    def _sort_by(self, col: str):
        if self._sort_col == col:
            self._sort_reverse = not self._sort_reverse
        else:
            self._sort_col = col
            self._sort_reverse = False

        items = [(self.tree.set(item, col), item)
                 for item in self.tree.get_children()]
        items.sort(reverse=self._sort_reverse,
                   key=lambda x: x[0].lower() if x[0] else "")
        for i, (_, item) in enumerate(items):
            self.tree.move(item, "", i)

        col_labels = {
            t("inv.col_binder"): t("anomaly.col_binder"), "carte": t("anomaly.col_card"), "art": "Art",
            "set_code": t("anomaly.col_set_missing"), "rarete": t("rarity.col_name"), "statut": t("anomaly.col_status")
        }
        for c_name, c_label in col_labels.items():
            arrow = (" ▲" if not self._sort_reverse else " ▼") if c_name == col else ""
            self.tree.heading(c_name, text=c_label + arrow)

    # ─────────────────────────────────────────────────────────────────────────
    # Actions
    # ─────────────────────────────────────────────────────────────────────────

    def _corriger_selection(self):
        selection = self._get_selected_anomalies()
        if not selection:
            afficher_warning(t("anomaly.select_one"))
            return
        nb, touches = svc.corriger_anomalies(selection)
        msg = f"{nb} ligne(s) Art B insérée(s)."
        if touches:
            msg += f"\nClasseur(s) mis à jour : {', '.join(touches)}."
        afficher_info(msg)
        self._charger()
        self._emit_changed()

    def _corriger_tout(self):
        non_corrigees = [a for a in self._anomalies_by_id.values()
                         if not a.get("corrige")]
        if not non_corrigees:
            afficher_info(t("anomaly.all_fixed"))
            return
        nb, touches = svc.corriger_anomalies(non_corrigees)
        msg = f"{nb} ligne(s) Art B insérée(s)."
        if touches:
            msg += f"\nClasseur(s) mis à jour : {', '.join(touches)}."
        afficher_info(msg)
        self._charger()
        self._emit_changed()

    def _annuler_selection(self):
        selection = self._get_selected_anomalies()
        if not selection:
            afficher_warning(t("anomaly.select_one"))
            return
        count = sum(
            1 for a in selection
            if a.get("corrige") and svc.annuler_correction(a)
        )
        afficher_info(f"{count} correction(s) annulée(s).")
        self._charger()
        self._emit_changed()

"""
raccourcis.py — Raccourcis clavier globaux de l'application.

Point d'entrée unique : appeler activer_raccourcis(root) depuis app_window.py.

Raccourcis gérés ici
─────────────────────────────────────────────────────────────
  Ctrl+C   Copie la cellule active d'un Treeview.
           (Entry / Text : comportement natif Windows conservé)

  Ctrl+A   Sélectionne tout dans Entry, Text et Treeview.
           Les bindings locaux redondants ont été supprimés.

  Entrée   • Sur un ttk.Button focalisé → l'active (comme Espace).
           • Sur un champ de recherche / pagination du visualiseur
             classeur → déclenche la commande associée.
           • Sur Entry dans un dialog (quantité, qualité, page goto)
             → les bindings locaux restent gérés au niveau du dialog.

Raccourcis laissés locaux (cycle de vie lié au widget)
─────────────────────────────────────────────────────────────
  Escape   Fermeture de dialogs (quantité, qualité, combobox édition)
  Return   Validation inline dans les dialogs d'édition de l'inventaire
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


# ─────────────────────────────────────────────────────────────────────────────
# Helpers internes
# ─────────────────────────────────────────────────────────────────────────────

def _widget_focus(root):
    try:
        return root.focus_get()
    except Exception:
        return None


def _treeview_copier(tree, root):
    """Copie la valeur de la colonne active dans un Treeview."""
    sel = tree.selection()
    if not sel:
        return
    col = getattr(tree, "_last_col_clicked", None)
    valeurs = tree.item(sel[0], "values")
    if not valeurs:
        return
    if col is not None:
        try:
            idx = list(tree["columns"]).index(col)
            texte = str(valeurs[idx])
        except (ValueError, IndexError):
            texte = "  ".join(str(v) for v in valeurs)
    else:
        texte = "  ".join(str(v) for v in valeurs)
    root.clipboard_clear()
    root.clipboard_append(texte)


# ─────────────────────────────────────────────────────────────────────────────
# Handlers
# ─────────────────────────────────────────────────────────────────────────────

def _handler_copier(root):
    def _copier(event=None):
        w = _widget_focus(root)
        # Entry / Text : natif Windows — ne pas surcharger
        if isinstance(w, (tk.Entry, tk.Text)):
            return
        if isinstance(w, ttk.Treeview):
            _treeview_copier(w, root)
            return "break"
    return _copier


def _handler_select_all(root):
    def _select_all(event=None):
        w = _widget_focus(root)
        if w is None:
            return
        if isinstance(w, tk.Entry):
            w.select_range(0, tk.END)
            w.icursor(tk.END)
            return "break"
        elif isinstance(w, tk.Text):
            w.tag_add(tk.SEL, "1.0", tk.END)
            return "break"
        elif isinstance(w, ttk.Treeview):
            try:
                w.selection_set(w.get_children())
            except Exception:
                pass
            return "break"
    return _select_all


def _handler_entree(root):
    """
    Touche Entrée globale.
    - ttk.Button focalisé → invoque le bouton (équivalent Espace).
    - Autres widgets     → laisse l'événement se propager.
    """
    def _entree(event=None):
        w = _widget_focus(root)
        if isinstance(w, ttk.Button):
            w.invoke()
            return "break"
        # Entry / Text / Treeview : les bindings locaux ou natifs prennent le relais
    return _entree


# ─────────────────────────────────────────────────────────────────────────────
# Point d'entrée public
# ─────────────────────────────────────────────────────────────────────────────

def activer_raccourcis(root: tk.Tk) -> None:
    """
    Enregistre tous les raccourcis globaux sur la fenêtre racine.
    À appeler une seule fois depuis build_app() dans app_window.py.
    """
    root.bind_all("<Control-c>", _handler_copier(root))
    root.bind_all("<Control-a>", _handler_select_all(root))
    root.bind_all("<Return>",    _handler_entree(root))


__all__ = ["activer_raccourcis"]

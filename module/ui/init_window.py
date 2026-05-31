"""
init_window.py — Fenêtre d'initialisation de la base de données.

Version corrigée : utilise un Toplevel modal (grab_set) avec wait_window()
mais SANS appeler mainloop à l'intérieur. L'appelant doit déjà être dans
la mainloop principale (via root.after(...)).
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

import queue
import threading
import tkinter as tk
from tkinter import ttk
from tkinter.scrolledtext import ScrolledText

from module.BDD_creation import run_init
from module.i18n import t
from module.logger_app import log


def show_init_window() -> bool:
    """
    Ouvre la fenêtre d'initialisation de la base de données (modale).

    Cette fonction doit être appelée AVANT la création de la fenêtre
    principale CTk (depuis main.py PHASE 1), avec une mini tk.Tk() cachée
    comme parent. Sinon, sous Windows, la maximisation de la racine CTk
    en cours masque la Toplevel d'init pendant toute la durée de run_init().

    Retourne True si l'initialisation a réussi, False sinon.
    """
    parent = tk._default_root
    if parent is None:
        log.error("show_init_window: aucune fenêtre racine — abandon")
        return False

    # IMPORTANT : on ne touche PAS à l'état du parent. S'il est withdrawn,
    # c'est intentionnel (mini-racine cachée dans main.py). Le deiconify()
    # ferait apparaître une boîte blanche vide à l'utilisateur.
    win = tk.Toplevel(parent)
    win.title(t("init.title"))

    # Taille + position centrée en une seule geometry() pour éviter les
    # flashs visuels
    W, H = 600, 400
    try:
        sw = parent.winfo_screenwidth()
        sh = parent.winfo_screenheight()
    except Exception:
        sw, sh = 1920, 1080
    x = (sw - W) // 2
    y = (sh - H) // 2
    win.geometry(f"{W}x{H}+{x}+{y}")
    win.resizable(False, False)

    # Fond sombre pour cohérence visuelle avec le reste de l'app
    try:
        win.configure(bg="#0A0B10")
    except Exception:
        pass

    # IMPORTANT : pas de transient(parent) ici. Avec un parent withdrawn
    # (mini-racine cachée en Phase 1), transient() lierait la Toplevel à
    # un parent invisible et Windows refuserait de l'afficher correctement.

    # Empêche la fermeture manuelle pendant l'init
    win.protocol("WM_DELETE_WINDOW", lambda: None)

    # ────────────────────────────────────────────────────────────────────
    # Forçage visibilité — séquence testée sous Windows 10/11 :
    #   1. deiconify       : sortir du state 'withdrawn' par défaut
    #   2. update_idletasks: faire traiter les events par le window manager
    #   3. lift            : remonter dans l'ordre Z
    #   4. -topmost (500ms): flag TOPMOST temporaire pour forcer l'affichage
    #   5. focus_force     : donne le focus clavier
    # ────────────────────────────────────────────────────────────────────
    try:
        win.deiconify()
        win.update_idletasks()
        win.lift()
        win.attributes("-topmost", True)
        win.after(500, lambda: win.attributes("-topmost", False))
        win.focus_force()
    except Exception as e:
        log.warning("show_init_window: mise en premier plan a échoué: %s", e)

    ttk.Label(
        win,
        text=t("init.title"),
        font=("Segoe UI", 13, "bold"),
        background="#0A0B10", foreground="#FFFFFF",
    ).pack(pady=10)

    text_area = ScrolledText(
        win, state="disabled", font=("Consolas", 11), height=15,
        bg="#12141D", fg="#FFFFFF",
        insertbackground="#D4AF37",
    )
    text_area.pack(expand=True, fill="both", padx=20, pady=(0, 20))

    msg_queue: queue.Queue = queue.Queue()
    result = [False]
    done   = [False]

    def _afficher_message(texte, couleur):
        try:
            text_area.config(state="normal")
            text_area.insert(tk.END, f"➔ {texte}\n", couleur)
            text_area.tag_config(couleur, foreground=couleur)
            text_area.see(tk.END)
            text_area.config(state="disabled")
        except tk.TclError:
            pass   # widget détruit

    def _pomper_queue():
        try:
            while True:
                item = msg_queue.get_nowait()
                if item is None:
                    _on_init_done()
                    return
                texte, couleur = item
                _afficher_message(texte, couleur)
        except queue.Empty:
            pass
        if not done[0]:
            try:
                win.after(100, _pomper_queue)
            except tk.TclError:
                pass

    def log_msg(msg, couleur="blue"):
        msg_queue.put((msg, couleur))

    def _thread_target():
        try:
            result[0] = run_init(log_msg)
        except Exception:
            log.exception("run_init a levé une exception")
            result[0] = False
        msg_queue.put(None)

    def _on_init_done():
        if done[0]:
            return
        done[0] = True
        try:
            text_area.config(state="normal")
            if result[0]:
                text_area.insert(tk.END, "\n✅ Initialisation complète.\n", "green")
                text_area.tag_config("green", foreground="green")
                delay = 1500
            else:
                text_area.insert(tk.END, "\n⚠ Initialisation échouée — l'application démarrera sans BDD.\n", "orange")
                text_area.tag_config("orange", foreground="orange")
                delay = 2000
            text_area.config(state="disabled")
            text_area.see(tk.END)
        except tk.TclError:
            pass

        try:
            win.after(delay, _close)
        except tk.TclError:
            _close()

    def _close():
        try:
            win.grab_release()
        except Exception:
            pass
        try:
            win.destroy()
        except Exception:
            pass

    threading.Thread(target=_thread_target, daemon=True).start()
    win.after(100, _pomper_queue)

    try:
        win.wait_window()
    except tk.TclError:
        pass

    return result[0]

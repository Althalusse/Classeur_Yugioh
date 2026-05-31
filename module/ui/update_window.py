"""
update_window.py — Fenêtre CTk de mise à jour de la base de données.

Pendant à init_window.py (utilisé au tout premier démarrage), cette fenêtre
gère le flux de MAJ après que l'application soit déjà lancée :

  1. Vérification de la version distante (via controle_version_database_api)
  2. Si MAJ disponible → confirmation utilisateur (versions local/remote)
  3. Backup auto + run_init + écriture de last_update.txt
  4. Logs live pendant la MAJ

Différences avec init_window :
  - 100% CTk (pas de ttk/tk pur), cohérent avec le reste de la nouvelle UI.
  - Lancée APRÈS la racine principale, pas avant — donc parent classique
    (la fenêtre principale CTk) sans hack de visibilité Windows.
  - Affiche d'abord un écran "Vérification" puis un écran "Confirmation"
    avant de lancer la MAJ proprement dite.

Architecture des états :
  STATE_CHECKING   → spinner + "Vérification de la version distante..."
  STATE_UP_TO_DATE → ✅ "Base à jour" + bouton Fermer
  STATE_AVAILABLE  → 🔄 versions affichées + boutons Annuler / Mettre à jour
  STATE_UPDATING   → log live + barre de progression indéterminée
  STATE_SUCCESS    → ✅ "MAJ réussie" + bouton Fermer
  STATE_ERROR      → ❌ message d'erreur + bouton Fermer
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
import customtkinter as ctk

from module.theme import C
from module.ui.composants import gold_button, secondary_button
from module.version.controle_version_database_api import (
    check_for_updates, update_database,
)


# États de la machine
STATE_CHECKING   = "checking"
STATE_UP_TO_DATE = "up_to_date"
STATE_AVAILABLE  = "available"
STATE_UPDATING   = "updating"
STATE_SUCCESS    = "success"
STATE_ERROR      = "error"


class UpdateWindow(ctk.CTkToplevel):
    """
    Fenêtre modale de mise à jour de la base de données.

    Usage : `UpdateWindow(parent)` — affichée immédiatement, vérifie
    automatiquement la version distante au lancement.
    """

    W = 640
    H = 460

    def __init__(self, parent):
        super().__init__(parent)
        self.title("Mise à jour de la base de données")
        self.configure(fg_color=C["bg"])
        self.resizable(False, False)

        # Centrage
        try:
            sw = parent.winfo_screenwidth()
            sh = parent.winfo_screenheight()
        except Exception:
            sw, sh = 1920, 1080
        x = (sw - self.W) // 2
        y = (sh - self.H) // 2
        self.geometry(f"{self.W}x{self.H}+{x}+{y}")

        # Modale tant qu'une MAJ n'est pas en cours (pendant la MAJ on
        # libère pour ne pas bloquer l'UI principale).
        try:
            self.transient(parent)
            self.grab_set()
        except Exception:
            pass

        # État interne
        self._state         = STATE_CHECKING
        self._update_thread = None
        self._msg_queue: queue.Queue = queue.Queue()
        self._destroyed     = False

        # Container principal
        self._container = ctk.CTkFrame(self, fg_color="transparent")
        self._container.pack(fill="both", expand=True, padx=24, pady=20)

        # Header (toujours présent)
        ctk.CTkLabel(
            self._container,
            text="🔄  Mise à jour de la base de données",
            font=("Outfit", 16, "bold"),
            text_color=C["text"],
        ).pack(anchor="w", pady=(0, 16))

        # Zone de contenu (changeante selon l'état)
        self._content = ctk.CTkFrame(self._container, fg_color=C["bg2"], corner_radius=4)
        self._content.pack(fill="both", expand=True)

        # Barre d'actions (changeante selon l'état)
        self._actions = ctk.CTkFrame(self._container, fg_color="transparent")
        self._actions.pack(fill="x", pady=(16, 0))

        # Démarrage : vérification immédiate en arrière-plan
        self.protocol("WM_DELETE_WINDOW", self._on_close)
        self._render_checking()
        threading.Thread(target=self._check_remote_version, daemon=True).start()

    # ─── Rendu par état ─────────────────────────────────────────────────────

    def _clear_content(self):
        """Vide les zones content et actions avant de les redessiner."""
        for w in self._content.winfo_children():
            w.destroy()
        for w in self._actions.winfo_children():
            w.destroy()

    def _render_checking(self):
        self._state = STATE_CHECKING
        self._clear_content()
        inner = ctk.CTkFrame(self._content, fg_color="transparent")
        inner.place(relx=0.5, rely=0.5, anchor="center")
        ctk.CTkLabel(
            inner,
            text="⏳",
            font=("Segoe UI", 32),
            text_color=C["gold"],
        ).pack(pady=(0, 8))
        ctk.CTkLabel(
            inner,
            text="Vérification de la version distante...",
            font=("Outfit", 12),
            text_color=C["text2"],
        ).pack()
        # Pas de boutons à ce stade

    def _render_up_to_date(self, infos):
        self._state = STATE_UP_TO_DATE
        self._clear_content()
        inner = ctk.CTkFrame(self._content, fg_color="transparent")
        inner.place(relx=0.5, rely=0.5, anchor="center")
        ctk.CTkLabel(
            inner,
            text="✅",
            font=("Segoe UI", 32),
            text_color=C["success"] if "success" in C else C["gold"],
        ).pack(pady=(0, 8))
        ctk.CTkLabel(
            inner,
            text="La base de données est à jour.",
            font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(pady=(0, 4))
        version = infos.get("local") or infos.get("remote") or "—"
        ctk.CTkLabel(
            inner,
            text=f"Version actuelle : {version}",
            font=("Outfit", 11),
            text_color=C["text3"],
        ).pack()

        secondary_button(
            self._actions, "Fermer", command=self._on_close
        ).pack(side="right")

    def _render_available(self, infos):
        self._state = STATE_AVAILABLE
        self._clear_content()
        inner = ctk.CTkFrame(self._content, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=24, pady=24)

        ctk.CTkLabel(
            inner,
            text="🔔  Une mise à jour est disponible",
            font=("Outfit", 14, "bold"),
            text_color=C["gold"],
        ).pack(anchor="w", pady=(0, 12))

        # Tableau versions
        for label, key, color in (
            ("Version locale  :",  "local",  C["text2"]),
            ("Version distante :", "remote", C["gold"]),
        ):
            row = ctk.CTkFrame(inner, fg_color="transparent")
            row.pack(fill="x", pady=4)
            ctk.CTkLabel(
                row, text=label, font=("Outfit", 11),
                text_color=C["text3"], width=140, anchor="w",
            ).pack(side="left")
            ctk.CTkLabel(
                row, text=str(infos.get(key, "—")),
                font=("JetBrains Mono", 11) if "JetBrains Mono" in str(C) else ("Consolas", 11),
                text_color=color,
            ).pack(side="left")

        ctk.CTkLabel(
            inner,
            text=("\nLa mise à jour va :\n"
                  "  • Sauvegarder la base actuelle dans backups/\n"
                  "  • Re-télécharger les données YGOJSON et YGOPRODeck\n"
                  "  • Reconstruire cardinfo.db (vos classeurs ne sont PAS affectés)"),
            font=("Outfit", 11),
            text_color=C["text2"],
            justify="left",
        ).pack(anchor="w", pady=(16, 0))

        # Boutons : Annuler à gauche, Mettre à jour à droite (action principale)
        secondary_button(
            self._actions, "Annuler", command=self._on_close
        ).pack(side="left")
        gold_button(
            self._actions, "Mettre à jour", command=self._start_update
        ).pack(side="right")

    def _render_error(self, message):
        self._state = STATE_ERROR
        self._clear_content()
        inner = ctk.CTkFrame(self._content, fg_color="transparent")
        inner.place(relx=0.5, rely=0.5, anchor="center")
        ctk.CTkLabel(
            inner,
            text="❌",
            font=("Segoe UI", 32),
            text_color=C["danger"],
        ).pack(pady=(0, 8))
        ctk.CTkLabel(
            inner,
            text="Erreur lors de la vérification",
            font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(pady=(0, 4))
        ctk.CTkLabel(
            inner,
            text=str(message),
            font=("Outfit", 11),
            text_color=C["text2"],
            wraplength=self.W - 100,
            justify="center",
        ).pack(padx=24)

        secondary_button(
            self._actions, "Fermer", command=self._on_close
        ).pack(side="right")

    def _render_updating(self):
        self._state = STATE_UPDATING
        self._clear_content()
        # La fenêtre devient non-modale pendant la MAJ : l'utilisateur peut
        # naviguer ailleurs, mais ne peut pas relancer une autre MAJ.
        try:
            self.grab_release()
        except Exception:
            pass

        ctk.CTkLabel(
            self._content,
            text="Mise à jour en cours…",
            font=("Outfit", 13, "bold"),
            text_color=C["gold"],
        ).pack(anchor="w", padx=16, pady=(12, 8))

        # Zone de log (CTkTextbox = équivalent CTk de ScrolledText)
        self._log_text = ctk.CTkTextbox(
            self._content,
            fg_color=C["bg3"],
            text_color=C["text"],
            font=("Consolas", 10),
            wrap="word",
        )
        self._log_text.pack(fill="both", expand=True, padx=16, pady=(0, 16))
        self._log_text.configure(state="disabled")

        # Pas de bouton pendant la MAJ : éviter d'interrompre run_init
        ctk.CTkLabel(
            self._actions,
            text="⚠ Ne fermez pas cette fenêtre pendant la mise à jour.",
            font=("Outfit", 10, "italic"),
            text_color=C["text3"],
        ).pack(side="left")

    def _render_success(self):
        self._state = STATE_SUCCESS
        self._clear_content()
        inner = ctk.CTkFrame(self._content, fg_color="transparent")
        inner.place(relx=0.5, rely=0.5, anchor="center")
        ctk.CTkLabel(
            inner,
            text="✅",
            font=("Segoe UI", 32),
            text_color=C["gold"],
        ).pack(pady=(0, 8))
        ctk.CTkLabel(
            inner,
            text="Mise à jour réussie",
            font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(pady=(0, 4))
        ctk.CTkLabel(
            inner,
            text="La base de données a été mise à jour avec succès.",
            font=("Outfit", 11),
            text_color=C["text2"],
        ).pack()

        gold_button(
            self._actions, "Fermer", command=self._on_close
        ).pack(side="right")

    # ─── Logique métier ─────────────────────────────────────────────────────

    def _check_remote_version(self):
        """Thread daemon : check_for_updates() puis renvoie le résultat sur
        le thread UI via after()."""
        try:
            update_available, infos = check_for_updates()
        except Exception as e:
            self._safe_after(0, self._render_error, f"Exception : {e}")
            return

        if "error" in infos:
            self._safe_after(0, self._render_error, infos["error"])
        elif update_available:
            self._safe_after(0, self._render_available, infos)
        else:
            self._safe_after(0, self._render_up_to_date, infos)

    def _start_update(self):
        """Lance la mise à jour (callback du bouton 'Mettre à jour')."""
        self._render_updating()
        self._update_thread = threading.Thread(
            target=self._update_worker, daemon=True,
        )
        self._update_thread.start()
        # Démarrage du pompage des messages depuis le thread MAJ
        self.after(100, self._pump_messages)

    def _update_worker(self):
        """Thread daemon : exécute update_database() avec callback de log."""
        def _log_callback(msg, couleur="blue"):
            self._msg_queue.put((msg, couleur))

        try:
            success, message = update_database(log=_log_callback)
        except Exception as e:
            success, message = False, f"Exception : {e}"

        # Sentinelle de fin pour signaler au pump_messages qu'on a terminé
        self._msg_queue.put(("__DONE__", success, message))

    def _pump_messages(self):
        """Vide la queue de messages et les affiche dans la zone de log."""
        if self._destroyed:
            return
        try:
            while True:
                item = self._msg_queue.get_nowait()
                if isinstance(item, tuple) and item and item[0] == "__DONE__":
                    _, success, message = item
                    if success:
                        # Petit délai pour que l'utilisateur voie le dernier
                        # message de log avant de basculer sur l'écran final
                        self.after(800, self._render_success)
                    else:
                        self.after(800, lambda m=message: self._render_error(m))
                    return
                texte, couleur = item
                self._append_log(texte, couleur)
        except queue.Empty:
            pass
        # Replanifier le pump tant que la fenêtre vit
        if not self._destroyed:
            self.after(100, self._pump_messages)

    def _append_log(self, texte, couleur):
        if self._destroyed:
            return
        try:
            self._log_text.configure(state="normal")
            # Translation des couleurs textuelles en hex
            color_hex = {
                "blue":   C["text"],
                "green":  C["gold"],
                "red":    C["danger"],
                "orange": C["warning_text"],
            }.get(couleur, C["text"])
            # Tag par couleur (créé une fois par couleur, idempotent)
            tag = f"col_{couleur}"
            self._log_text.tag_config(tag, foreground=color_hex)
            self._log_text.insert("end", f"➔ {texte}\n", tag)
            self._log_text.see("end")
            self._log_text.configure(state="disabled")
        except Exception:
            pass

    # ─── Cycle de vie ───────────────────────────────────────────────────────

    def _on_close(self):
        """Ferme la fenêtre (sauf pendant la MAJ pour éviter d'interrompre)."""
        if self._state == STATE_UPDATING:
            return  # Bouton désactivé pendant la MAJ
        self._destroyed = True
        try:
            self.grab_release()
        except Exception:
            pass
        try:
            self.destroy()
        except Exception:
            pass

    def _safe_after(self, delay, fn, *args):
        """Wrapper défensif : ignore les after() sur fenêtres détruites."""
        if self._destroyed:
            return
        try:
            self.after(delay, fn, *args)
        except Exception:
            pass


def show_update_window(parent):
    """
    Helper public : ouvre la fenêtre de MAJ et la rend modale.
    À appeler depuis un bouton de l'UI principale.
    """
    win = UpdateWindow(parent)
    win.focus_force()
    return win

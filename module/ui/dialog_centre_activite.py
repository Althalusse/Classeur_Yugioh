"""
dialog_centre_activite.py — Fenêtre "Centre d'activité" : visualisation
en temps réel des tâches de la file d'attente (FileAttenteClasseur).

Architecture
────────────
  - CTkToplevel NON-MODALE — l'utilisateur peut continuer à interagir
    avec l'app pendant que la fenêtre est ouverte.
  - Singleton léger : une seule instance ouverte à la fois (la 2e tentative
    d'ouverture re-focus la fenêtre existante au lieu d'en créer une autre).
  - Auto-refresh : s'enregistre comme callback du singleton
    FileAttenteClasseur (`definir_callback_refresh`). Les notifications du
    worker (depuis un thread daemon) sont marshalées vers le main thread
    Tk via `widget.after(0, ...)`.
  - Affichage par tâche : icône statut + code + nom + phase + barre de
    progression + message + bouton 🚫 (annulation, si applicable).
  - Bouton "🧹 Vider les terminées" : appelle vider_termines() du singleton.
  - Vide ⇒ affichage d'un message "Aucune activité en cours".

Callback chain
──────────────
1. Worker termine une étape → tache.statut/progression/message changent
2. Worker appelle self._notifier() → callback enregistré (cette dialog)
3. Le callback marshall vers le main thread via widget.after(0, _refresh)
4. _refresh prend un snapshot et reconstruit la liste

Note : l'ancien callback du singleton (s'il y en avait un) est restauré
quand la fenêtre se ferme, pour ne pas casser EcranClasseur._poll_dl_progress
qui n'utilise PAS ce mécanisme (il poll directement la liste self.taches).
En pratique aucun autre listener n'est enregistré actuellement, mais on
respecte le contrat au cas où.
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
from module.ui.composants import gold_button, secondary_button, progress_bar
from module.img_dl.file_attente_classeur import (
    FileAttenteClasseur, StatutTache, PhaseTache,
)
from module.logger_app import log


# Référence faible vers la dialog active (None si aucune)
_active_dialog: "DialogCentreActivite | None" = None


# Couleurs par statut (utilisent la palette C du thème)
_STATUT_COLOR = {
    StatutTache.EN_ATTENTE: C.get("text3", "#646982"),
    StatutTache.EN_COURS:   C.get("gold",  "#D4AF37"),
    StatutTache.TERMINE:    C.get("success", "#2D8A4E"),
    StatutTache.ERREUR:     C.get("danger", "#9D1A2A"),
    StatutTache.ANNULE:     C.get("warning", "#B07D1A"),
}

# Libellé court de phase pour ne pas surcharger l'UI
_PHASE_LABEL = {
    PhaseTache.INITIAL:        "",
    PhaseTache.CREATION:       "Création",
    PhaseTache.TELECHARGEMENT: "Téléchargement",
    PhaseTache.TERMINEE:       "",
}


class _LigneTache(ctk.CTkFrame):
    """Une ligne de la liste = une tâche.

    On ne reconstruit pas la ligne à chaque refresh : on update les widgets
    en place pour éviter le flicker et préserver le focus/scroll. La
    fonction `update_from(tache)` est idempotente et bon marché.
    """

    def __init__(self, parent, tache):
        super().__init__(
            parent,
            fg_color=C["bg2"],
            corner_radius=6,
            border_width=1,
            border_color=C["border2"],
        )
        self._tache = tache

        # Ligne 1 : icône + code + nom + phase
        top = ctk.CTkFrame(self, fg_color="transparent")
        top.pack(fill="x", padx=10, pady=(8, 2))

        self._lbl_icone = ctk.CTkLabel(
            top, text="⏳", font=("Segoe UI", 14),
            text_color=C["text"], width=24,
        )
        self._lbl_icone.pack(side="left")

        self._lbl_code = ctk.CTkLabel(
            top, text=tache.code,
            font=("JetBrains Mono", 11, "bold"),
            text_color=C["gold"], width=80, anchor="w",
        )
        self._lbl_code.pack(side="left", padx=(4, 8))

        # Nom optionnel — souvent vide pour les tâches d'images de routine
        self._lbl_nom = ctk.CTkLabel(
            top, text="", anchor="w",
            font=("Outfit", 10),
            text_color=C["text2"],
        )
        self._lbl_nom.pack(side="left", fill="x", expand=True)

        self._lbl_phase = ctk.CTkLabel(
            top, text="",
            font=("Outfit", 9),
            text_color=C["text3"],
        )
        self._lbl_phase.pack(side="right")

        # Ligne 2 : barre de progression + pourcentage
        mid = ctk.CTkFrame(self, fg_color="transparent")
        mid.pack(fill="x", padx=10, pady=(0, 2))

        self._pbar = progress_bar(mid)
        self._pbar.pack(side="left", fill="x", expand=True, padx=(0, 8))

        self._lbl_pct = ctk.CTkLabel(
            mid, text="0 %",
            font=("JetBrains Mono", 9),
            text_color=C["text3"], width=44, anchor="e",
        )
        self._lbl_pct.pack(side="right")

        # Ligne 3 : message d'état (textuel)
        self._lbl_msg = ctk.CTkLabel(
            self, text="", anchor="w", justify="left",
            font=("Outfit", 9),
            text_color=C["text3"],
            wraplength=480,
        )
        self._lbl_msg.pack(fill="x", padx=10, pady=(0, 8))

        # Premier rendu
        self.update_from(tache)

    def update_from(self, tache):
        """Met à jour les widgets selon l'état courant de la tâche.

        Sûr à appeler depuis le main thread uniquement. Les attributs de
        `tache` peuvent être lus depuis n'importe quel thread car ce sont
        des assignations Python atomiques (mots-clés simples, pas de
        structures complexes mutées en place).
        """
        self._tache = tache
        statut = tache.statut

        # Icône + couleur (extraite du libellé Enum, ex "✅ Terminé")
        icone_text = statut.value.split(" ", 1)[0] if " " in statut.value else "•"
        couleur = _STATUT_COLOR.get(statut, C["text"])
        self._lbl_icone.configure(text=icone_text, text_color=couleur)
        self._lbl_code.configure(text=tache.code)

        # Nom : préfixé par "—" si présent, sinon vide
        nom = (tache.nom or "").strip()
        if nom:
            display = nom if len(nom) <= 60 else (nom[:57] + "…")
            self._lbl_nom.configure(text=f"— {display}")
        else:
            self._lbl_nom.configure(text="")

        # Phase : seulement quand pertinent (création/téléchargement)
        self._lbl_phase.configure(text=_PHASE_LABEL.get(tache.phase, ""))

        # Barre de progression + %
        prog = max(0, min(100, tache.progression or 0))
        try:
            self._pbar.set(prog / 100.0)
        except Exception:
            pass
        # Couleur de la barre selon statut (gold par défaut, vert si fini,
        # rouge si erreur)
        try:
            if statut == StatutTache.TERMINE:
                self._pbar.configure(progress_color=C["success"])
            elif statut == StatutTache.ERREUR:
                self._pbar.configure(progress_color=C["danger"])
            elif statut == StatutTache.ANNULE:
                self._pbar.configure(progress_color=C["warning"])
            else:
                self._pbar.configure(progress_color=C["gold"])
        except Exception:
            pass
        self._lbl_pct.configure(text=f"{prog} %")

        # Message
        msg = (tache.message or "").strip()
        # Tronquer pour ne pas exploser la hauteur de la ligne sur les
        # messages d'erreur très longs (les utilisateurs voient l'idée,
        # le détail complet va dans les logs).
        if len(msg) > 180:
            msg = msg[:177] + "…"
        self._lbl_msg.configure(text=msg)


class DialogCentreActivite(ctk.CTkToplevel):
    """Fenêtre flottante listant les tâches de FileAttenteClasseur.

    Non-modale (pas de grab_set), redimensionnable verticalement. Reste
    en avant-plan via `attributes("-topmost", True)` mais l'utilisateur
    peut la mettre derrière en cliquant ailleurs (le topmost est appliqué
    seulement au moment de l'ouverture, pas en continu — sinon ça gêne).
    """

    W_WINDOW = 560
    H_WINDOW = 480

    def __init__(self, parent):
        super().__init__(parent)
        self.title("Centre d'activité")
        self.configure(fg_color=C["bg"])
        self.minsize(420, 280)

        # Centrage écran
        try:
            sw = parent.winfo_screenwidth()
            sh = parent.winfo_screenheight()
        except Exception:
            sw, sh = 1920, 1080
        x = (sw - self.W_WINDOW) // 2
        y = (sh - self.H_WINDOW) // 2
        self.geometry(f"{self.W_WINDOW}x{self.H_WINDOW}+{x}+{y}")

        # transient = la dialog suit le parent dans le z-order ; pas de
        # grab_set car on veut explicitement laisser l'utilisateur
        # interagir avec le reste de l'app.
        try:
            self.transient(parent)
        except Exception:
            pass

        # File singleton + callback
        self._file = FileAttenteClasseur()
        # On garde l'ancien callback pour le restaurer à la fermeture, au
        # cas (futur) où un autre composant en aurait enregistré un.
        self._previous_callback = self._file._callback_refresh
        self._destroyed = False
        # Map code-de-tâche → ligne UI déjà créée (pour update en place)
        self._lignes: dict[int, _LigneTache] = {}  # id(tache) -> ligne

        self._build()

        # Branchement callback
        self._file.definir_callback_refresh(self._on_file_refresh)

        # Premier rendu
        self._refresh()

        # Cycle de vie
        self.protocol("WM_DELETE_WINDOW", self._on_close)
        self.bind("<Escape>", lambda e: self._on_close())

        # Topmost ponctuel pour s'assurer que l'utilisateur la voit, puis
        # on relâche pour ne pas bloquer le travail dans d'autres fenêtres.
        try:
            self.attributes("-topmost", True)
            self.after(400, lambda: self.attributes("-topmost", False))
        except Exception:
            pass

    # ── Construction UI ──────────────────────────────────────────────────

    def _build(self):
        # Header
        header = ctk.CTkFrame(self, fg_color=C["bg2"], corner_radius=0, height=56)
        header.pack(fill="x")
        header.pack_propagate(False)

        ctk.CTkLabel(
            header, text="📥  Centre d'activité",
            font=("Playfair Display", 14, "bold"),
            text_color=C["text"],
        ).pack(side="left", padx=16, pady=10)

        self._lbl_compteur = ctk.CTkLabel(
            header, text="",
            font=("Outfit", 10),
            text_color=C["text3"],
        )
        self._lbl_compteur.pack(side="left", padx=8)

        secondary_button(
            header, "🧹 Vider les terminées",
            command=self._vider_termines,
            width=180,
        ).pack(side="right", padx=12, pady=8)

        # Bordure bas du header
        ctk.CTkFrame(
            self, height=1, fg_color=C["border"], corner_radius=0,
        ).pack(fill="x")

        # Zone scrollable des tâches
        self._scroll = ctk.CTkScrollableFrame(
            self, fg_color="transparent", corner_radius=0,
        )
        self._scroll.pack(fill="both", expand=True, padx=12, pady=12)

        # Label "vide" affiché quand aucune tâche
        self._lbl_vide = ctk.CTkLabel(
            self._scroll,
            text=("Aucune activité en cours.\n\n"
                  "Les téléchargements d'images et les créations de "
                  "classeurs (manuelles ou via import CSV) apparaîtront "
                  "ici en temps réel."),
            font=("Outfit", 11),
            text_color=C["text3"],
            justify="center",
            wraplength=420,
        )
        # packé/dépacké dans _refresh selon présence de tâches

    # ── Refresh / data binding ───────────────────────────────────────────

    def _on_file_refresh(self):
        """Callback appelé par FileAttenteClasseur depuis n'importe quel thread.

        On marshalle vers le main thread Tk via after(0, _refresh).
        Si la fenêtre est en cours de destruction ou déjà détruite,
        on ignore silencieusement.
        """
        if self._destroyed:
            return
        try:
            if not self.winfo_exists():
                return
        except Exception:
            return
        try:
            self.after(0, self._refresh)
        except RuntimeError:
            # mainloop n'est plus actif (app en fermeture)
            pass
        except Exception:
            pass

    def _refresh(self):
        """Reconstruit/rafraîchit la liste des tâches depuis le snapshot.

        Stratégie d'update :
          - On obtient un snapshot ordonné des tâches (du plus ancien au
            plus récent).
          - Pour chaque tâche déjà présente : on update sa ligne en place.
          - Pour chaque tâche nouvelle : on crée une ligne (pack à la fin).
          - Pour chaque tâche disparue (vidée) : on détruit sa ligne.

        Cette stratégie évite le flicker visuel et préserve la position
        de scroll lors des updates fréquents pendant les téléchargements.
        """
        if self._destroyed:
            return
        try:
            taches = self._file.snapshot_taches()
        except Exception as e:
            log.warning(f"DialogCentreActivite._refresh: {e}")
            return

        # Compteur en header
        n_actives = sum(
            1 for t in taches
            if t.statut in (StatutTache.EN_ATTENTE, StatutTache.EN_COURS)
        )
        n_total = len(taches)
        if n_total == 0:
            self._lbl_compteur.configure(text="")
        elif n_actives == 0:
            self._lbl_compteur.configure(text=f"{n_total} terminée(s)")
        else:
            self._lbl_compteur.configure(
                text=f"{n_actives} en cours / {n_total} au total"
            )

        # Liste vide ?
        if not taches:
            # Détruire toutes les lignes existantes
            for ligne in list(self._lignes.values()):
                try:
                    ligne.destroy()
                except Exception:
                    pass
            self._lignes.clear()
            # Afficher le label vide
            self._lbl_vide.pack(pady=40, padx=20)
            return

        # On a des tâches → masquer le label vide
        try:
            self._lbl_vide.pack_forget()
        except Exception:
            pass

        ids_courants = set()
        for tache in taches:
            tid = id(tache)
            ids_courants.add(tid)
            ligne = self._lignes.get(tid)
            if ligne is None:
                # Nouvelle tâche → créer la ligne
                ligne = _LigneTache(self._scroll, tache)
                ligne.pack(fill="x", pady=4)
                self._lignes[tid] = ligne
            else:
                # Update en place
                ligne.update_from(tache)

        # Cleanup : tâches qui ont disparu de la liste (suite à un
        # vider_termines par exemple)
        for tid in list(self._lignes.keys()):
            if tid not in ids_courants:
                try:
                    self._lignes[tid].destroy()
                except Exception:
                    pass
                del self._lignes[tid]

    # ── Actions ──────────────────────────────────────────────────────────

    def _vider_termines(self):
        """Retire les tâches finies (terminées/erreur/annulées) de la file."""
        try:
            self._file.vider_termines()
        except Exception as e:
            log.warning(f"DialogCentreActivite._vider_termines: {e}")
        # _refresh sera déclenché automatiquement par le callback du singleton

    # ── Cycle de vie ─────────────────────────────────────────────────────

    def _on_close(self):
        if self._destroyed:
            return
        self._destroyed = True
        # Restaurer l'ancien callback (probablement None)
        try:
            self._file.definir_callback_refresh(self._previous_callback)
        except Exception:
            pass
        # Libérer la référence singleton-de-dialog
        global _active_dialog
        if _active_dialog is self:
            _active_dialog = None
        try:
            self.destroy()
        except Exception:
            pass


# ─────────────────────────────────────────────────────────────────────────────
# Point d'entrée public
# ─────────────────────────────────────────────────────────────────────────────

def show_centre_activite(parent):
    """Ouvre la fenêtre Centre d'activité (ou re-focus si déjà ouverte).

    Args:
        parent : widget parent CTk (typiquement la fenêtre racine).

    Returns:
        L'instance DialogCentreActivite (existante ou nouvelle).
    """
    global _active_dialog

    # Si déjà ouverte ET pas détruite, on la re-focus
    if _active_dialog is not None:
        try:
            if _active_dialog.winfo_exists() and not _active_dialog._destroyed:
                _active_dialog.deiconify()
                _active_dialog.lift()
                try:
                    _active_dialog.focus_set()
                except Exception:
                    pass
                return _active_dialog
        except Exception:
            # La référence pointe vers un widget cassé — on en crée une
            # nouvelle ci-dessous.
            _active_dialog = None

    _active_dialog = DialogCentreActivite(parent)
    return _active_dialog

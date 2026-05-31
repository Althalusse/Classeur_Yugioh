"""
composants.py — Composants UI réutilisables (spec UI).
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
from module.logger_app import log


# ─── Boutons ───────────────────────────────────────────────────────────────

def gold_button(parent, text, command=None, width=None, **kw) -> ctk.CTkButton:
    kw.setdefault("fg_color",    C["gold"])
    kw.setdefault("hover_color", C["gold_hover"])
    kw.setdefault("text_color",  "#000000")
    kw.setdefault("font",        ("Outfit", 11, "bold"))
    kw.setdefault("corner_radius", 0)
    kw.setdefault("border_width",  0)
    kw.setdefault("height",        38)
    if width: kw["width"] = width
    return ctk.CTkButton(parent, text=text, command=command, **kw)


def secondary_button(parent, text, command=None, width=None, **kw) -> ctk.CTkButton:
    kw.setdefault("fg_color",     "transparent")
    kw.setdefault("hover_color",  C["bg_hover"])
    kw.setdefault("text_color",   C["text"])
    kw.setdefault("border_color", C["border2"])
    kw.setdefault("border_width", 1)
    kw.setdefault("font",         ("Outfit", 11))
    kw.setdefault("corner_radius", 0)
    kw.setdefault("height",        38)
    if width: kw["width"] = width
    return ctk.CTkButton(parent, text=text, command=command, **kw)


def icon_button(parent, text, command=None, **kw) -> ctk.CTkButton:
    kw.setdefault("fg_color",     C["bg_hover"])
    kw.setdefault("hover_color",  C["bg3"])
    kw.setdefault("text_color",   C["text"])
    kw.setdefault("border_color", C["border2"])
    kw.setdefault("border_width", 1)
    kw.setdefault("font",         ("Segoe UI", 10))
    kw.setdefault("corner_radius", 4)
    kw.setdefault("height",        34)
    return ctk.CTkButton(parent, text=text, command=command, **kw)


def danger_button(parent, text, command=None, **kw) -> ctk.CTkButton:
    kw.setdefault("fg_color",    C["danger"])
    kw.setdefault("hover_color", C["danger_hover"])
    kw.setdefault("text_color",  C["text"])
    kw.setdefault("font",        ("Outfit", 11, "bold"))
    kw.setdefault("corner_radius", 0)
    kw.setdefault("height",        38)
    return ctk.CTkButton(parent, text=text, command=command, **kw)


# ─── Inputs ────────────────────────────────────────────────────────────────

def search_entry(parent, textvariable=None, placeholder="Rechercher...", **kw):
    kw.setdefault("placeholder_text",       placeholder)
    kw.setdefault("placeholder_text_color", C["text3"])
    kw.setdefault("fg_color",    C["bg2"])
    kw.setdefault("border_color", C["border2"])
    kw.setdefault("border_width", 1)
    kw.setdefault("text_color",   C["text"])
    kw.setdefault("font",         ("Outfit", 11))
    kw.setdefault("corner_radius", 4)
    kw.setdefault("height",        40)
    return ctk.CTkEntry(parent, textvariable=textvariable, **kw)


def styled_combobox(parent, values=None, variable=None, **kw):
    kw.setdefault("fg_color",           C["bg2"])
    kw.setdefault("border_color",        C["border2"])
    kw.setdefault("border_width",        1)
    kw.setdefault("button_color",        C["bg3"])
    kw.setdefault("button_hover_color",  C["bg_hover"])
    kw.setdefault("dropdown_fg_color",   C["bg3"])
    kw.setdefault("dropdown_hover_color", C["bg_hover"])
    kw.setdefault("text_color",          C["text"])
    kw.setdefault("dropdown_text_color", C["text"])
    kw.setdefault("font",                ("Outfit", 11))
    kw.setdefault("dropdown_font",       ("Outfit", 10))
    kw.setdefault("corner_radius",        4)
    kw.setdefault("height",               38)
    return ctk.CTkComboBox(parent, values=values or [], variable=variable, **kw)


def progress_bar(parent, **kw):
    kw.setdefault("fg_color",       C["bg_hover"])
    kw.setdefault("progress_color", C["gold"])
    kw.setdefault("corner_radius",  3)
    kw.setdefault("height",         6)
    return ctk.CTkProgressBar(parent, **kw)


def separator(parent, orient="horizontal"):
    if orient == "horizontal":
        return ctk.CTkFrame(parent, height=1, fg_color=C["border"], corner_radius=0)
    return ctk.CTkFrame(parent, width=1, fg_color=C["border"], corner_radius=0)


# ─── StatCard ──────────────────────────────────────────────────────────────

class StatCard(ctk.CTkFrame):
    """Panneau stat avec une grande valeur et un label."""

    def __init__(self, parent, number: str, label: str, **kw):
        kw.setdefault("fg_color",      C["bg2"])
        kw.setdefault("border_color",  C["border2"])
        kw.setdefault("border_width",  1)
        kw.setdefault("corner_radius", 8)
        super().__init__(parent, **kw)
        self.columnconfigure(0, weight=1)

        self._number_lbl = ctk.CTkLabel(
            self, text=number,
            font=("Georgia", 26, "bold"),
            text_color=C["text"],
        )
        self._number_lbl.pack(pady=(18, 2))

        ctk.CTkLabel(
            self, text=label.upper(),
            font=("Outfit", 9),
            text_color=C["text3"],
        ).pack(pady=(0, 18))

    def update_value(self, number: str):
        self._number_lbl.configure(text=number)


# ─── Navbar ────────────────────────────────────────────────────────────────

class Navbar(ctk.CTkFrame):
    """
    Barre de navigation.
    right_factory(frame) : callable optionnel appelé avec le cadre droit
                           pour y créer les boutons.
    """

    def __init__(self, parent, title: str = "", subtitle: str = "",
                 show_back: bool = False, back_command=None,
                 right_factory=None, **kw):
        kw.setdefault("fg_color",     C["bg2"])
        kw.setdefault("corner_radius", 0)
        kw.setdefault("border_width",  0)
        kw.setdefault("height",        56)
        super().__init__(parent, **kw)
        self.pack_propagate(False)

        # Gauche
        left = ctk.CTkFrame(self, fg_color="transparent")
        left.pack(side="left", padx=16, pady=8)

        if show_back and back_command:
            ctk.CTkButton(
                left, text="←", command=back_command,
                width=36, height=36,
                fg_color="transparent", hover_color=C["bg_hover"],
                text_color=C["text"], font=("Segoe UI", 16), corner_radius=4,
            ).pack(side="left", padx=(0, 8))

        if title:
            parts = title.rsplit(" ", 1)
            if len(parts) == 2:
                ctk.CTkLabel(left, text=parts[0] + " ",
                             font=("Georgia", 14, "bold"),
                             text_color=C["text"]).pack(side="left")
                ctk.CTkLabel(left, text=parts[1],
                             font=("Georgia", 14, "bold"),
                             text_color=C["gold"]).pack(side="left")
            else:
                ctk.CTkLabel(left, text=title,
                             font=("Georgia", 14, "bold"),
                             text_color=C["text"]).pack(side="left")
        if subtitle:
            ctk.CTkLabel(left, text=f"  {subtitle}",
                         font=("Outfit", 10),
                         text_color=C["text3"]).pack(side="left")

        # Droite
        if right_factory:
            right = ctk.CTkFrame(self, fg_color="transparent")
            right.pack(side="right", padx=16, pady=8)
            right_factory(right)

        # Bordure bas
        ctk.CTkFrame(self, height=1, fg_color=C["border"],
                     corner_radius=0).pack(side="bottom", fill="x")


# ─── Badge (niveau module) ────────────────────────────────────────────────

def badge(parent, text, color=None, **kw):
    """Petit label stylé type 'badge' (quantité, rareté, etc.)."""
    kw.setdefault("fg_color",   color or C.get("bg3", "#12141D"))
    kw.setdefault("text_color", C.get("text", "#ffffff"))
    kw.setdefault("corner_radius", 6)
    kw.setdefault("font", ("Outfit", 10, "bold"))
    kw.setdefault("height", 20)
    # padx n'existe pas sur CTkLabel — on utilise corner_radius + padding via pack
    return ctk.CTkLabel(parent, text=text, **kw)


# ─── Bouton "Centre d'activité" avec badge live ────────────────────────────

class CentreActiviteButton(ctk.CTkFrame):
    """Bouton "📥 Activité" + badge avec compteur de tâches actives.

    Clique → ouvre `dialog_centre_activite.show_centre_activite(parent)`.
    Le compteur se met à jour automatiquement via le callback du singleton
    FileAttenteClasseur.

    Pourquoi un Frame conteneur plutôt qu'un Button avec badge superposé ?
    CTkButton ne permet pas de superposer un widget enfant proprement, et
    on évite la complexité d'un Canvas. Frame + Button + Label = simple
    et maintenable.

    IMPORTANT — partage du callback singleton :
        FileAttenteClasseur n'a qu'un seul `_callback_refresh` à la fois.
        Si DialogCentreActivite est ouverte, elle écrase ce callback à
        l'ouverture et le restaure à la fermeture. Pendant qu'elle est
        ouverte, ce bouton ne reçoit donc PAS de notifications du
        singleton et son badge devient stale. Pour pallier ça, on poll
        également toutes les 1.5 s en arrière-plan : c'est largement
        assez réactif pour un compteur (vs ~200 ms pour la liste détaillée
        de la dialog elle-même), et coûte juste un appel à
        nb_total_actives() (sum sur une liste typiquement < 20 items).
    """

    POLL_INTERVAL_MS = 1500

    def __init__(self, parent, get_root_callback=None, **kw):
        """
        Args:
            parent : widget parent CTk.
            get_root_callback : callable() → widget racine pour la dialog.
                                Si None, on utilise self.winfo_toplevel().
        """
        kw.setdefault("fg_color", "transparent")
        kw.setdefault("corner_radius", 0)
        super().__init__(parent, **kw)

        self._get_root_callback = get_root_callback
        self._destroyed = False
        self._poll_after_id = None

        # Bouton principal
        self._btn = ctk.CTkButton(
            self, text="📥  Activité",
            command=self._on_click,
            fg_color=C["bg_hover"],
            hover_color=C["bg3"],
            text_color=C["text"],
            border_color=C["border2"],
            border_width=1,
            font=("Segoe UI", 10),
            corner_radius=4,
            height=34,
        )
        self._btn.pack(side="left")

        # Badge compteur (caché tant que 0 tâche)
        self._badge = ctk.CTkLabel(
            self,
            text="0",
            fg_color=C["gold"],
            text_color="#000000",
            corner_radius=8,
            font=("Outfit", 9, "bold"),
            width=18, height=18,
        )
        # Pas de pack initial → caché par défaut

        # Branchement file pour le 1er rendu + polling périodique
        try:
            from module.img_dl.file_attente_classeur import FileAttenteClasseur
            self._file = FileAttenteClasseur()
        except Exception:
            self._file = None

        self._tick()  # premier tick immédiat
        self._schedule_next_tick()

        # Cleanup à la destruction
        self.bind("<Destroy>", self._on_destroy)

    def _on_click(self):
        """Ouvre le centre d'activité. Le widget racine est obtenu via le
        callback fourni (utile dans les écrans enfants), ou par
        winfo_toplevel() si aucun callback."""
        try:
            if self._get_root_callback is not None:
                root = self._get_root_callback()
            else:
                root = self.winfo_toplevel()
            from module.ui.dialog_centre_activite import show_centre_activite
            show_centre_activite(root)
        except Exception as e:
            log.warning(f"CentreActiviteButton._on_click: {e}")

    def _tick(self):
        """Met à jour le badge selon nb_total_actives()."""
        if self._destroyed or self._file is None:
            return
        try:
            n = self._file.nb_total_actives()
        except Exception:
            return
        try:
            if n <= 0:
                # Cache le badge
                self._badge.pack_forget()
            else:
                # Affiche le badge avec le nombre (+ "+" si > 9 pour ne pas
                # déformer la pastille)
                texte = "9+" if n > 9 else str(n)
                self._badge.configure(text=texte)
                # Pack à droite du bouton (avec petit décalage)
                if not self._badge.winfo_ismapped():
                    self._badge.pack(side="left", padx=(4, 0))
        except Exception:
            pass

    def _schedule_next_tick(self):
        if self._destroyed:
            return
        try:
            self._poll_after_id = self.after(
                self.POLL_INTERVAL_MS, self._poll_loop
            )
        except Exception:
            pass

    def _poll_loop(self):
        if self._destroyed:
            return
        self._tick()
        self._schedule_next_tick()

    def _on_destroy(self, event=None):
        # event peut être déclenché pour des enfants (CTkButton, etc.)
        # — on ne réagit qu'à la destruction de self.
        if event is not None and event.widget is not self:
            return
        self._destroyed = True
        if self._poll_after_id is not None:
            try:
                self.after_cancel(self._poll_after_id)
            except Exception:
                pass

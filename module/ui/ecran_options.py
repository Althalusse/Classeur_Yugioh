"""
ecran_options.py — Écran Options (préférences utilisateur).

Sections :
  🌐 Langue            — nom cartes (FR/EN) + interface (FR/EN, redémarrage)
  🖼 Source d'images   — YGOPRODeck (HD) / Yugipedia (par print)
  📑 Tri des cartes    — drag & drop sur les 3 critères (numero/rareté/artwork)
  👁 Affichage         — filtre "une rareté par numéro+artwork" (visuel)
  ⭐ Priorité raretés  — drag & drop sur toutes les raretés connues
  📐 Grille par défaut — cols × lignes (3-10) appliquée aux nouveaux classeurs
  🔧 Maintenance       — init BDD, vider cache, scan anomalies global

Le drag & drop est implémenté en custom (pas de dépendance externe) : pour
chaque item, on capture <Button-1>/<B1-Motion>/<ButtonRelease-1> et on
détermine la position cible en comparant les coordonnées Y du curseur
au centre de chaque voisin.
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

import os
import threading
import customtkinter as ctk

from module.theme import C, get_font_scale, save_font_scale, font_scale_bounds
from module.ui.composants import (
    gold_button, secondary_button, icon_button,
    Navbar, separator,
)
from module.config_langue import load_langue, save_langue
from module.config_image_source import load_image_source, save_image_source
from module.config import preferences
from module.gestion_img.cache_images import clear_cache
from module.gestion_rarete.gestion_rarete_service import (
    load_rarity_priorities, save_rarity_priorities,
    get_default_priorities, get_all_rarities_from_db,
)
from module.i18n import t, get_current_lang, save_ui_langue
from module.logger_app import log


# ─────────────────────────────────────────────────────────────────────────────
# Liste drag & drop réutilisable
# ─────────────────────────────────────────────────────────────────────────────

class DragDropList(ctk.CTkFrame):
    """Liste d'items réordonnables par drag & drop.

    Chaque item est un CTkFrame avec un handle ☰ à gauche, un label au centre.
    L'utilisateur peut attraper un item et le glisser vers le haut/bas —
    la position est recalculée à chaque mouvement en comparant le curseur
    aux centres Y des autres items.

    Args:
        parent         : widget parent
        items          : liste initiale d'items (ordre)
        label_fn       : fn(item) → texte à afficher
        on_reorder     : callback appelée avec la nouvelle liste après drop
        item_height    : hauteur d'un item en pixels
    """

    ITEM_HEIGHT = 38
    GAP         = 4

    def __init__(self, parent, items: list, label_fn=None, on_reorder=None,
                 item_height: int | None = None):
        super().__init__(parent, fg_color=C["bg3"], corner_radius=8,
                         border_color=C["border"], border_width=1)
        self._items: list       = list(items)
        self._label_fn          = label_fn or (lambda x: str(x))
        self._on_reorder        = on_reorder
        self._item_height       = item_height or self.ITEM_HEIGHT

        # widgets par item (pour pouvoir les déplacer avec place())
        self._item_widgets: dict = {}
        # État du drag
        self._drag_item         = None
        self._drag_start_y      = 0
        self._drag_start_index  = -1
        self._drag_offset_y     = 0

        self._rebuild()

    def _rebuild(self):
        """(Re)construit tous les items et les positionne avec place()."""
        # Détruire les anciens widgets
        for w in self._item_widgets.values():
            try:
                w.destroy()
            except Exception:
                pass
        self._item_widgets = {}

        for i, item in enumerate(self._items):
            w = self._make_item_widget(item)
            self._item_widgets[id(item)] = w
            # NOTE : CustomTkinter interdit 'width'/'height' dans .place() —
            # ils doivent être fixés dans le constructeur du widget. La
            # hauteur est déjà appliquée via height=self._item_height dans
            # _make_item_widget(). On n'utilise ici que x/y/relwidth.
            w.place(x=0, y=self._y_for_index(i), relwidth=1.0)

        # Ajuster la hauteur du frame parent
        total_h = len(self._items) * (self._item_height + self.GAP) + self.GAP
        self.configure(height=max(total_h, self._item_height + 2 * self.GAP))

    def _y_for_index(self, index: int) -> int:
        return self.GAP + index * (self._item_height + self.GAP)

    def _make_item_widget(self, item) -> ctk.CTkFrame:
        frame = ctk.CTkFrame(
            self, fg_color=C["bg_card"],
            corner_radius=6,
            border_color=C["border2"], border_width=1,
            height=self._item_height, cursor="fleur",
        )
        frame.pack_propagate(False)

        # Handle drag
        handle = ctk.CTkLabel(
            frame, text="☰", font=("Segoe UI", 14),
            text_color=C["gold_dim"], width=26,
        )
        handle.pack(side="left", padx=(8, 4))

        # Label
        lbl = ctk.CTkLabel(
            frame, text=self._label_fn(item),
            font=("Outfit", 11), text_color=C["text"], anchor="w",
        )
        lbl.pack(side="left", fill="x", expand=True, padx=4)

        # Bindings sur tout le frame (handle + label compris) pour que le
        # drag marche quel que soit l'endroit où l'utilisateur clique
        for w in (frame, handle, lbl):
            w.bind("<Button-1>",        lambda e, it=item: self._on_drag_start(it, e))
            w.bind("<B1-Motion>",       lambda e, it=item: self._on_drag_motion(it, e))
            w.bind("<ButtonRelease-1>", lambda e, it=item: self._on_drag_end(it, e))

        return frame

    def _on_drag_start(self, item, event):
        self._drag_item        = item
        self._drag_start_y     = event.y_root
        self._drag_start_index = self._items.index(item)
        # Calcul offset : la souris peut être au milieu du widget, pas en haut
        w = self._item_widgets.get(id(item))
        if w:
            try:
                widget_top = w.winfo_rooty()
                self._drag_offset_y = event.y_root - widget_top
                # Remonte visuellement (en "z-order")
                w.lift()
                w.configure(border_color=C["gold"])
            except Exception:
                self._drag_offset_y = 0

    def _on_drag_motion(self, item, event):
        if self._drag_item is None or self._drag_item != item:
            return
        w = self._item_widgets.get(id(item))
        if not w:
            return

        try:
            # Position Y du haut du widget dans le parent
            parent_top = self.winfo_rooty()
            target_y = event.y_root - parent_top - self._drag_offset_y
            # Clamp dans les limites visibles
            max_y = (len(self._items) - 1) * (self._item_height + self.GAP) + self.GAP
            target_y = max(self.GAP, min(target_y, max_y))
            w.place_configure(y=target_y)

            # Détermine l'index cible selon la position
            target_center = target_y + self._item_height / 2
            new_index = 0
            for i in range(len(self._items)):
                center = self._y_for_index(i) + self._item_height / 2
                if target_center > center:
                    new_index = i
            # Si on dépasse le dernier centre, on reste sur len-1 ; sinon on arrondit
            if target_center > self._y_for_index(len(self._items) - 1):
                new_index = len(self._items) - 1

            if new_index != self._items.index(item):
                # Reposition les autres items sans toucher à l'item draggé
                self._swap_to_index(item, new_index)
        except Exception:
            pass

    def _swap_to_index(self, item, new_index: int):
        """Retire l'item et le réinsère à new_index, puis repositionne les
        autres widgets (sans toucher au widget draggé, qui suit la souris)."""
        old_index = self._items.index(item)
        if new_index == old_index:
            return
        self._items.pop(old_index)
        self._items.insert(new_index, item)

        # Repositionne tous les widgets sauf l'item draggé
        for i, it in enumerate(self._items):
            if it is item:
                continue
            w = self._item_widgets.get(id(it))
            if w:
                w.place_configure(y=self._y_for_index(i))

    def _on_drag_end(self, item, event):
        if self._drag_item is None:
            return
        dragged = self._drag_item
        self._drag_item = None

        # Remet le widget draggé à la position finale correspondant à son
        # index actuel dans self._items
        w = self._item_widgets.get(id(dragged))
        if w:
            try:
                idx = self._items.index(dragged)
                w.place_configure(y=self._y_for_index(idx))
                w.configure(border_color=C["border2"])
            except Exception:
                pass

        # Callback avec la nouvelle liste
        if self._on_reorder:
            try:
                self._on_reorder(list(self._items))
            except Exception as e:
                log.warning(f"DragDropList on_reorder : {e}")


# ─────────────────────────────────────────────────────────────────────────────
# Écran Options
# ─────────────────────────────────────────────────────────────────────────────

class EcranOptions(ctk.CTkFrame):

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._build()

    def _build(self):
        # Navbar
        Navbar(
            self, title="Options",
            show_back=True, back_command=self._retour,
        ).pack(fill="x")

        # Conteneur du bandeau "redémarrage requis", placé entre la navbar et
        # le scroll (ordre de packing : navbar → holder → scroll). On NE peut
        # PAS utiliser `before=self._scroll` car CTkScrollableFrame redirige son
        # pack vers un canvas interne (self._scroll n'est pas le widget packé) :
        # on packe donc le holder ici, à sa place définitive.
        #
        # Pour ne PAS réserver d'espace mort quand le bandeau est masqué (un
        # CTkFrame vide occupe ~200 px), le holder est forcé à 0 px de hauteur
        # (pack_propagate False) tant qu'on n'a rien à afficher. À l'affichage,
        # on réactive pack_propagate(True) → il reprend la hauteur du bandeau.
        self._restart_holder = ctk.CTkFrame(self, fg_color="transparent",
                                            height=0)
        self._restart_holder.pack(fill="x")
        self._restart_holder.pack_propagate(False)   # collapsé tant que vide

        self._restart_bar = ctk.CTkFrame(
            self._restart_holder, fg_color=C["bg2"],
            border_color=C["gold"], border_width=1, corner_radius=0,
        )
        # Le bandeau est packé une fois pour toutes DANS le holder ; on montre /
        # cache l'ensemble en jouant sur la hauteur du holder (collapse 0 px).
        self._restart_bar.pack(fill="x")
        inner = ctk.CTkFrame(self._restart_bar, fg_color="transparent")
        inner.pack(fill="x", padx=24, pady=10)
        ctk.CTkLabel(
            inner,
            text="↻ Un redémarrage est nécessaire pour appliquer vos changements.",
            font=("Outfit", 12, "bold"), text_color=C["gold"], anchor="w",
        ).pack(side="left")
        secondary_button(
            inner, "Plus tard", command=self._masquer_redemarrage, width=90,
        ).pack(side="right", padx=(8, 0))
        gold_button(
            inner, "🔄  Redémarrer maintenant",
            command=self._redemarrer, width=220,
        ).pack(side="right", padx=(8, 0))

        # Zone scrollable principale
        self._scroll = ctk.CTkScrollableFrame(
            self, fg_color="transparent", corner_radius=0,
        )
        self._scroll.pack(fill="both", expand=True, padx=24, pady=16)

        self._build_section_langue()
        self._separator()
        self._build_section_font_scale()
        self._separator()
        self._build_section_images()
        self._separator()
        self._build_section_tri_criteres()
        self._separator()
        self._build_section_affichage()
        self._separator()
        self._build_section_rarete()
        self._separator()
        self._build_section_grille()
        self._separator()
        # Section "Régions OCG" retirée : sets japonais toujours inclus.
        self._build_section_maintenance()

    def _separator(self):
        ctk.CTkFrame(
            self._scroll, height=1, fg_color=C["border"], corner_radius=0,
        ).pack(fill="x", pady=20)

    def _section_title(self, text: str):
        ctk.CTkLabel(
            self._scroll, text=text,
            font=("Georgia", 14, "bold"), text_color=C["gold"],
        ).pack(anchor="w", pady=(0, 8))

    def _section_hint(self, text: str):
        ctk.CTkLabel(
            self._scroll, text=text,
            font=("Outfit", 10), text_color=C["text3"],
            justify="left", wraplength=800,
        ).pack(anchor="w", pady=(0, 10))

    # ── Section Taille de police ─────────────────────────────────────────

    def _build_section_font_scale(self):
        """Slider pour régler l'échelle globale des polices + preview live.

        Le changement réel ne s'applique qu'au redémarrage car les polices
        de tous les widgets déjà construits sont figées (theme.FONT est
        chargé une seule fois). La preview montre en direct l'effet que
        la taille sélectionnée aura au prochain démarrage.
        """
        self._section_title("🔤  Taille de police")
        self._section_hint(
            "Ajuste la taille globale des textes dans l'application. "
            "La preview à droite montre la taille qu'auront les textes "
            "après redémarrage. Le changement est sauvegardé "
            "automatiquement — il suffit de redémarrer pour qu'il prenne "
            "effet partout."
        )

        current_scale = get_font_scale()
        fmin, fmax    = font_scale_bounds()

        # Ligne principale : slider (gauche) + preview (droite)
        main = ctk.CTkFrame(self._scroll, fg_color="transparent")
        main.pack(fill="x", pady=(0, 6))
        main.columnconfigure(0, weight=3)
        main.columnconfigure(1, weight=2)

        # ── Colonne gauche : slider ─────────────────────────────────────
        left = ctk.CTkFrame(main, fg_color="transparent")
        left.grid(row=0, column=0, sticky="ew", padx=(0, 20))

        # Ligne 1 : label courant + valeur
        val_row = ctk.CTkFrame(left, fg_color="transparent")
        val_row.pack(fill="x", pady=(0, 8))
        ctk.CTkLabel(
            val_row, text="Échelle :",
            font=("Outfit", 11), text_color=C["text"],
        ).pack(side="left")
        self._lbl_font_value = ctk.CTkLabel(
            val_row, text=f"{current_scale:.2f}×",
            font=("JetBrains Mono", 12, "bold"),
            text_color=C["gold"], width=70,
        )
        self._lbl_font_value.pack(side="left", padx=(8, 0))

        self._lbl_font_preset = ctk.CTkLabel(
            val_row,
            text=self._font_preset_label(current_scale),
            font=("Outfit", 10), text_color=C["text3"],
        )
        self._lbl_font_preset.pack(side="left", padx=(8, 0))

        # Slider
        # Note: CTkSlider number_of_steps définit le nombre de paliers.
        # Avec 13 paliers entre 0.85 et 1.50, on a ~0.05 de granularité.
        self._font_slider = ctk.CTkSlider(
            left,
            from_=fmin, to=fmax,
            number_of_steps=13,
            progress_color=C["gold"],
            button_color=C["gold"],
            button_hover_color=C["gold_hover"],
            fg_color=C["bg3"],
            command=self._on_font_slider_change,
        )
        self._font_slider.set(current_scale)
        self._font_slider.pack(fill="x", pady=(0, 4))

        # Graduations sous le slider
        grad_row = ctk.CTkFrame(left, fg_color="transparent")
        grad_row.pack(fill="x")
        ctk.CTkLabel(
            grad_row, text=f"{fmin:.2f}×",
            font=("JetBrains Mono", 9), text_color=C["text3"],
        ).pack(side="left")
        ctk.CTkLabel(
            grad_row, text="1.00× (défaut)",
            font=("JetBrains Mono", 9), text_color=C["text3"],
        ).pack(side="left", expand=True)
        ctk.CTkLabel(
            grad_row, text=f"{fmax:.2f}×",
            font=("JetBrains Mono", 9), text_color=C["text3"],
        ).pack(side="right")

        # Message "redémarrage requis"
        self._lbl_font_status = ctk.CTkLabel(
            left, text="",
            font=("Outfit", 10), text_color=C["gold_dim"],
        )
        self._lbl_font_status.pack(anchor="w", pady=(10, 0))

        # ── Colonne droite : preview ────────────────────────────────────
        right = ctk.CTkFrame(
            main, fg_color=C["bg3"],
            corner_radius=8,
            border_color=C["border"], border_width=1,
        )
        right.grid(row=0, column=1, sticky="nsew")

        ctk.CTkLabel(
            right, text="APERÇU",
            font=("Outfit", 9, "bold"),
            text_color=C["text3"],
        ).pack(pady=(10, 4))

        # Labels de preview stockés pour pouvoir les reconfigurer live.
        # On choisit des éléments représentatifs : un titre, un label normal,
        # un petit texte secondaire, un texte monospace (code carte).
        self._preview_title = ctk.CTkLabel(
            right, text="Rescue Cat",
            text_color=C["text"],
        )
        self._preview_title.pack(pady=(2, 0))

        self._preview_body = ctk.CTkLabel(
            right, text="Possédée · Quantité · Qualité",
            text_color=C["text2"],
        )
        self._preview_body.pack()

        self._preview_mono = ctk.CTkLabel(
            right, text="RA02-EN001  ·  Ultra Rare",
            text_color=C["gold"],
        )
        self._preview_mono.pack(pady=(0, 10))

        # Applique la taille initiale à la preview
        self._refresh_preview(current_scale)

    def _font_preset_label(self, scale: float) -> str:
        """Retourne un libellé humain correspondant à l'échelle."""
        if scale < 0.95:
            return "(compact)"
        if scale < 1.10:
            return "(normal)"
        if scale < 1.25:
            return "(confortable)"
        if scale < 1.40:
            return "(grand)"
        return "(très grand)"

    def _refresh_preview(self, scale: float):
        """Met à jour les polices des labels de preview en temps réel."""
        def _sc(size: int) -> int:
            return max(7, round(size * scale))

        try:
            self._preview_title.configure(
                font=("Playfair Display", _sc(15), "bold"),
            )
            self._preview_body.configure(
                font=("Outfit", _sc(13)),
            )
            self._preview_mono.configure(
                font=("JetBrains Mono", _sc(11)),
            )
        except Exception as e:
            log.warning(f"_refresh_preview : {e}")

    def _on_font_slider_change(self, value: float):
        """Appelé en continu pendant que l'utilisateur bouge le slider.

        - Met à jour la preview en direct (gratuit : juste 3 configure())
        - Sauvegarde la valeur dans app_config (le prochain démarrage
          utilisera cette échelle pour toute l'UI)
        - Affiche un message "↻ Redémarrer pour appliquer"
        """
        try:
            clamped = save_font_scale(value)
        except Exception as e:
            log.warning(f"save_font_scale : {e}")
            clamped = value

        # Met à jour le label de valeur
        try:
            self._lbl_font_value.configure(text=f"{clamped:.2f}×")
            self._lbl_font_preset.configure(
                text=self._font_preset_label(clamped),
            )
            self._lbl_font_status.configure(
                text="↻ Sauvegardé — redémarrer l'application pour appliquer",
            )
        except Exception:
            pass

        self._demander_redemarrage()

        # Met à jour la preview
        self._refresh_preview(clamped)

    # ── Section Langue ───────────────────────────────────────────────────

    def _build_section_langue(self):
        self._section_title("🌐  Langue")
        self._section_hint(
            "Langue utilisée pour afficher les noms des cartes, et "
            "langue de l'interface de l'application."
        )

        # Langue des noms de cartes
        row1 = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row1.pack(fill="x", pady=(0, 6))
        ctk.CTkLabel(
            row1, text="Noms des cartes :",
            font=("Outfit", 11), text_color=C["text"], width=180, anchor="w",
        ).pack(side="left")
        self._langue_cartes_var = ctk.StringVar(value=load_langue())
        for code in ("FR", "EN"):
            ctk.CTkRadioButton(
                row1, text=code,
                variable=self._langue_cartes_var, value=code,
                command=self._on_langue_cartes_change,
                fg_color=C["gold"], hover_color=C["gold_hover"],
                border_color=C["border"],
                text_color=C["text"], font=("Outfit", 11),
            ).pack(side="left", padx=(0, 16))
        self._lbl_langue_cartes = ctk.CTkLabel(
            row1, text="", font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_langue_cartes.pack(side="left", padx=8)

        # Langue de l'interface
        row2 = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row2.pack(fill="x", pady=(0, 6))
        ctk.CTkLabel(
            row2, text="Interface :",
            font=("Outfit", 11), text_color=C["text"], width=180, anchor="w",
        ).pack(side="left")
        self._langue_ui_var = ctk.StringVar(value=get_current_lang())
        for code in ("FR", "EN"):
            ctk.CTkRadioButton(
                row2, text=code,
                variable=self._langue_ui_var, value=code,
                command=self._on_langue_ui_change,
                fg_color=C["gold"], hover_color=C["gold_hover"],
                border_color=C["border"],
                text_color=C["text"], font=("Outfit", 11),
            ).pack(side="left", padx=(0, 16))
        self._lbl_langue_ui = ctk.CTkLabel(
            row2, text="", font=("Outfit", 10), text_color=C["text3"],
        )
        self._lbl_langue_ui.pack(side="left", padx=8)

    def _on_langue_cartes_change(self):
        lang = self._langue_cartes_var.get()
        save_langue(lang)
        self._lbl_langue_cartes.configure(
            text=f"✓ Enregistré ({lang})", text_color=C["success"],
        )

    def _on_langue_ui_change(self):
        lang = self._langue_ui_var.get()
        save_ui_langue(lang)
        self._lbl_langue_ui.configure(
            text="↻ Redémarrer pour appliquer", text_color=C["gold_dim"],
        )
        self._demander_redemarrage()

    # ── Section Images ───────────────────────────────────────────────────

    def _build_section_images(self):
        self._section_title("🖼  Source des images de cartes")
        self._section_hint(
            "YGOPRODeck fournit des JPEG HD d'artwork principal (couverture "
            "maximale, recommandé). Yugipedia propose des PNG spécifiques à "
            "chaque print (rareté/édition) quand ils existent."
        )
        row = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row.pack(fill="x", pady=(0, 6))
        self._source_var = ctk.StringVar(value=load_image_source())
        for code, label in (("YGOPRODECK", "YGOPRODeck (HD)"),
                            ("YUGIPEDIA",  "Yugipedia (par print)")):
            ctk.CTkRadioButton(
                row, text=label,
                variable=self._source_var, value=code,
                command=self._on_source_change,
                fg_color=C["gold"], hover_color=C["gold_hover"],
                border_color=C["border"],
                text_color=C["text"], font=("Outfit", 11),
            ).pack(side="left", padx=(0, 24))
        self._lbl_source = ctk.CTkLabel(
            row, text="", font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_source.pack(side="left", padx=8)

    def _on_source_change(self):
        src = self._source_var.get()
        save_image_source(src)
        # Vider le cache PIL : les chemins restent valides mais on re-télécharge
        try:
            clear_cache()
        except Exception:
            pass
        self._lbl_source.configure(
            text=f"✓ Source : {src}", text_color=C["success"],
        )

    # ── Section Tri des cartes (drag & drop critères) ────────────────────

    def _build_section_tri_criteres(self):
        self._section_title("📑  Ordre de tri des cartes")
        self._section_hint(
            "Glissez pour réordonner les critères de tri. Le critère du "
            "haut est appliqué en premier, puis les suivants."
        )

        self._tri_list = DragDropList(
            self._scroll,
            items=preferences.get_ordre_tri(),
            label_fn=lambda c: preferences.get_tri_label(c),
            on_reorder=self._on_tri_reorder,
            item_height=40,
        )
        self._tri_list.pack(fill="x", pady=(0, 6))

        self._lbl_tri = ctk.CTkLabel(
            self._scroll, text="",
            font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_tri.pack(anchor="w", pady=(4, 0))

    def _on_tri_reorder(self, new_order: list):
        preferences.save_ordre_tri(new_order)
        self._lbl_tri.configure(
            text="✓ Ordre enregistré — s'applique au prochain affichage d'un classeur",
            text_color=C["success"],
        )

    # ── Section Affichage (filtres visuels du visualiseur) ───────────────

    def _build_section_affichage(self):
        """Préférences d'affichage du visualiseur de classeur.

        Pour l'instant : option "N raretés affichées par numéro+artwork".

        Note de design : le filtre est PURE (côté UI, n'écrit rien dans la
        BDD). L'export Scanflip et toutes les autres opérations continuent
        de voir l'intégralité des cartes. C'est important : un utilisateur
        qui possède 3 raretés différentes de RA02-EN001 ne perd rien en
        réglant N=1, il voit simplement la rareté la plus rare à l'écran.

        La valeur globale est l'option par défaut. Chaque classeur peut
        l'override individuellement via le menu d'actions de l'accueil
        (cf. ecran_accueil.DialogRaretesOverride).
        """
        self._section_title("👁  Affichage du classeur")
        self._section_hint(
            "Pour chaque numéro de carte (et chaque artwork séparément), "
            "limite l'affichage aux N raretés les plus rares. 0 = toutes "
            "les raretés affichées. Les autres raretés restent en base et "
            "sont exportées normalement — c'est un filtre purement visuel. "
            "Chaque classeur peut override cette valeur via son menu "
            "d'actions sur l'accueil."
        )

        row = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row.pack(fill="x", pady=(0, 6))

        # Champ numérique : 0 = toutes / N≥1 = N plus rares
        ctk.CTkLabel(
            row, text="Raretés affichées par carte+artwork :",
            font=("Outfit", 11), text_color=C["text"],
        ).pack(side="left", padx=(0, 8))

        n_min, n_max = preferences.n_raretes_bounds()
        current_n = preferences.get_n_raretes_par_artwork()
        self._var_n_raretes = ctk.StringVar(value=str(current_n))
        self._entry_n_raretes = ctk.CTkEntry(
            row, textvariable=self._var_n_raretes,
            width=60, height=28,
            font=("JetBrains Mono", 11), justify="center",
            fg_color=C["bg3"], border_color=C["border"], border_width=1,
            text_color=C["text"],
        )
        self._entry_n_raretes.pack(side="left", padx=(0, 8))
        self._entry_n_raretes.bind("<Return>",   self._on_n_raretes_validate)
        self._entry_n_raretes.bind("<KP_Enter>", self._on_n_raretes_validate)
        self._entry_n_raretes.bind("<FocusOut>", self._on_n_raretes_validate)

        ctk.CTkLabel(
            row,
            text=f"(0 = toutes · max {n_max} · ex: 3 pour les 3 plus rares)",
            font=("Outfit", 10), text_color=C["text3"],
        ).pack(side="left", padx=(0, 8))

        self._lbl_affichage = ctk.CTkLabel(
            row, text="", font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_affichage.pack(side="left")

    def _on_n_raretes_validate(self, event=None):
        """Sauvegarde la valeur globale N (clampée). Affiche un retour."""
        try:
            raw = self._var_n_raretes.get().strip()
            n_in = int(raw) if raw else 0
        except ValueError:
            n_in = preferences.get_n_raretes_par_artwork()
            self._var_n_raretes.set(str(n_in))
            self._lbl_affichage.configure(
                text="⚠ Valeur numérique requise",
                text_color=C["danger"],
            )
            return
        try:
            n_eff = preferences.save_n_raretes_par_artwork(n_in)
        except Exception as e:
            self._lbl_affichage.configure(
                text=f"⚠ Erreur sauvegarde : {e}",
                text_color=C["danger"],
            )
            return
        # Resynchro champ avec la valeur clampée par save_*
        if str(n_eff) != self._var_n_raretes.get().strip():
            self._var_n_raretes.set(str(n_eff))
        if n_eff == 0:
            msg = "✓ Toutes les raretés affichées"
        elif n_eff == 1:
            msg = "✓ 1 rareté la plus rare par carte+artwork"
        else:
            msg = f"✓ {n_eff} raretés les plus rares par carte+artwork"
        msg += " — effet à la prochaine ouverture d'un classeur"
        self._lbl_affichage.configure(text=msg, text_color=C["success"])

    # ── Section Priorité des raretés (drag & drop) ───────────────────────

    def _build_section_rarete(self):
        self._section_title("⭐  Priorité des raretés")
        self._section_hint(
            "Utilisé uniquement quand « Rareté » fait partie des critères de "
            "tri ci-dessus. Glissez pour réordonner du plus commun (haut) au "
            "plus rare (bas)."
        )

        # Charge l'ordre actuel depuis rarity_config.json
        priorities = load_rarity_priorities()
        if not priorities:
            priorities = get_default_priorities()
            save_rarity_priorities(priorities)

        # S'assure que les raretés de la DB sont toutes présentes (sync)
        all_from_db = get_all_rarities_from_db()
        next_rank = max(priorities.values(), default=0) + 1
        for r in all_from_db:
            if r not in priorities:
                priorities[r] = next_rank
                next_rank += 1
        if len(priorities) != len(load_rarity_priorities()):
            save_rarity_priorities(priorities)

        # Ordonne par priorité croissante
        ordered = sorted(priorities.keys(), key=lambda r: priorities[r])

        self._rarete_list = DragDropList(
            self._scroll,
            items=ordered,
            label_fn=lambda r: r,
            on_reorder=self._on_rarete_reorder,
            item_height=34,
        )
        self._rarete_list.pack(fill="x", pady=(0, 6))

        self._lbl_rarete = ctk.CTkLabel(
            self._scroll, text="",
            font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_rarete.pack(anchor="w", pady=(4, 0))

    def _on_rarete_reorder(self, new_order: list):
        # Régénère un dict {rareté: priorité_1_based}
        priorities = {r: i + 1 for i, r in enumerate(new_order)}
        save_rarity_priorities(priorities)
        self._lbl_rarete.configure(
            text="✓ Priorités enregistrées",
            text_color=C["success"],
        )

    # ── Section Grille par défaut ────────────────────────────────────────

    def _build_section_grille(self):
        self._section_title("📐  Grille du classeur par défaut")
        gmin, gmax = preferences.grille_bounds()
        self._section_hint(
            f"Dimensions appliquées aux nouveaux classeurs. Valeurs entre "
            f"{gmin} et {gmax}. Chaque classeur peut ensuite avoir sa propre "
            f"grille (bouton « Grille N×N » en bas de chaque classeur sur "
            f"l'écran d'accueil)."
        )

        cur_cols, cur_rows = preferences.get_grille_defaut()

        row = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row.pack(fill="x", pady=(0, 6))

        ctk.CTkLabel(
            row, text="Colonnes :",
            font=("Outfit", 11), text_color=C["text"], width=100, anchor="w",
        ).pack(side="left")
        self._grille_cols_var = ctk.StringVar(value=str(cur_cols))
        self._grille_cols_entry = ctk.CTkEntry(
            row, textvariable=self._grille_cols_var, width=60,
            font=("JetBrains Mono", 11), justify="center",
        )
        self._grille_cols_entry.pack(side="left", padx=(0, 16))

        ctk.CTkLabel(
            row, text="Lignes :",
            font=("Outfit", 11), text_color=C["text"], width=80, anchor="w",
        ).pack(side="left")
        self._grille_rows_var = ctk.StringVar(value=str(cur_rows))
        self._grille_rows_entry = ctk.CTkEntry(
            row, textvariable=self._grille_rows_var, width=60,
            font=("JetBrains Mono", 11), justify="center",
        )
        self._grille_rows_entry.pack(side="left", padx=(0, 16))

        gold_button(
            row, "Enregistrer",
            command=self._on_grille_save, width=120,
        ).pack(side="left")

        self._lbl_grille = ctk.CTkLabel(
            row, text="", font=("Outfit", 10), text_color=C["success"],
        )
        self._lbl_grille.pack(side="left", padx=12)

    def _on_grille_save(self):
        try:
            c = int(self._grille_cols_var.get())
            r = int(self._grille_rows_var.get())
        except ValueError:
            self._lbl_grille.configure(
                text="⚠ Valeurs numériques requises",
                text_color=C["danger"],
            )
            return
        c2, r2 = preferences.save_grille_defaut(c, r)
        # Remet les valeurs clampées dans les champs
        self._grille_cols_var.set(str(c2))
        self._grille_rows_var.set(str(r2))
        self._lbl_grille.configure(
            text=f"✓ Enregistré ({c2}×{r2})",
            text_color=C["success"],
        )

    # ── Section Régions OCG ──────────────────────────────────────────────

    # ── Section Maintenance ──────────────────────────────────────────────

    def _build_section_maintenance(self):
        self._section_title("🔧  Maintenance")

        self._section_hint(
            "Initialisation manuelle de la base interne (cardinfo.db) ou "
            "purge du cache d'images. À utiliser en cas de problème."
        )

        row = ctk.CTkFrame(self._scroll, fg_color="transparent")
        row.pack(fill="x", pady=(0, 6))

        icon_button(
            row, "🔧 Initialiser la base",
            command=self._lancer_init_bdd,
        ).pack(side="left", padx=(0, 8))

        icon_button(
            row, "🧹 Vider cache images",
            command=self._vider_cache,
        ).pack(side="left", padx=(0, 8))

        icon_button(
            row, "🔄 Réparer classeurs (set_codes + raretés)",
            command=self._reparer_set_codes,
        ).pack(side="left", padx=(0, 8))

        icon_button(
            row, "☁ Corriger raretés (Yugipedia)",
            command=self._corriger_raretes_yugipedia,
        ).pack(side="left", padx=(0, 8))

        icon_button(
            row, "🖼 Récupérer covers manquants (Yugipedia)",
            command=self._recuperer_covers_yugipedia,
        ).pack(side="left", padx=(0, 8))

        icon_button(
            row, "🃏 Corriger les Overframe (Yugipedia)",
            command=self._corriger_overframe,
        ).pack(side="left", padx=(0, 8))

        self._lbl_maint = ctk.CTkLabel(
            row, text="", font=("Outfit", 10), text_color=C["text3"],
        )
        self._lbl_maint.pack(side="left", padx=12)

    def _lancer_init_bdd(self):
        try:
            from module.ui.init_window import show_init_window
            show_init_window()
            self._lbl_maint.configure(
                text="✓ Base initialisée",
                text_color=C["success"],
            )
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur : {e}",
                text_color=C["danger"],
            )

    def _vider_cache(self):
        try:
            clear_cache()
            self._lbl_maint.configure(
                text="✓ Cache d'images vidé",
                text_color=C["success"],
            )
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur : {e}",
                text_color=C["danger"],
            )

    def _reparer_set_codes(self):
        """Détecte et migre les set_codes non-EN + raretés numériques dans
        les classeurs existants.

        Bug fixes mai 2026 — SDWD :
          - set_codes en FR/IT au lieu d'EN (rectifiés vers EN canonique)
          - raretés stockées comme chiffres "2"/"3" au lieu de "Common"
            (pattern observé : champ qty mal interprété comme rarity dans
            l'ancien pipeline)

        Action sûre et idempotente : ré-exécutable sans risque, ne modifie
        que les set_codes et raretés numériques (les quantités, qualités,
        possessed restent intacts).
        """
        try:
            from module.utilitaire.migration_set_codes import migrer_tous_classeurs
            res = migrer_tous_classeurs()
            n_classeurs = res["classeurs_traites"]
            n_sc        = res["total_migrees_sc"]
            n_conflits  = res["total_conflits_sc"]
            n_rar       = res["total_migrees_rar"]
            if n_classeurs == 0:
                self._lbl_maint.configure(
                    text="✓ Aucun classeur à réparer (set_codes et raretés OK)",
                    text_color=C["success"],
                )
            else:
                parts = [f"{n_classeurs} classeur(s) traité(s)"]
                if n_sc > 0:
                    parts.append(f"{n_sc} set_codes corrigés")
                if n_rar > 0:
                    parts.append(f"{n_rar} raretés '2'/'3' → 'Common'")
                if n_conflits > 0:
                    parts.append(f"{n_conflits} conflits ignorés")
                self._lbl_maint.configure(
                    text="✓ " + " — ".join(parts),
                    text_color=(C["success"] if n_conflits == 0 else C["warning"]),
                )
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur réparation : {e}",
                text_color=C["danger"],
            )

    # ── Navigation ───────────────────────────────────────────────────────

    def _corriger_raretes_yugipedia(self):
        """Corrige, dans tous les classeurs existants, les raretés parasites
        de YGOPRODeck (ex 'New artwork') en interrogeant Yugipedia.

        Réseau → exécuté en arrière-plan (0 freeze UI). La possession des
        cartes est préservée (UPDATE plutôt que suppression quand une rareté
        invalide correspond à une rareté manquante).
        """
        self._lbl_maint.configure(
            text="☁ Correction via Yugipedia en cours…",
            text_color=C["text2"],
        )

        def work():
            import os
            from module.centralisation_dossier import CLASSEUR_FOLDER
            from module.gestion_rarete.correction_rarete import reparer_db
            total = {"updates": 0, "inserts": 0, "deletes": 0,
                     "echecs": 0, "cartes": 0, "classeurs": 0}
            if not os.path.isdir(CLASSEUR_FOLDER):
                return total
            for nom in sorted(os.listdir(CLASSEUR_FOLDER)):
                db = os.path.join(CLASSEUR_FOLDER, nom, f"{nom}.db")
                if not os.path.isfile(db):
                    continue
                try:
                    s = reparer_db(db)
                except Exception as e:
                    from module.logger_app import log
                    log.warning(f"Correction Yugipedia {nom}: {e}")
                    continue
                if any(s[k] for k in ("updates", "inserts", "deletes",
                                      "echecs", "cartes")):
                    total["classeurs"] += 1
                for k in ("updates", "inserts", "deletes", "echecs", "cartes"):
                    total[k] += s[k]
            return total

        def on_done(total):
            # run_async passe un seul argument ; total=None si work a échoué.
            if not total:
                self._lbl_maint.configure(
                    text="⚠ Erreur pendant la correction Yugipedia",
                    text_color=C["danger"],
                )
                return
            if total["cartes"] == 0:
                self._lbl_maint.configure(
                    text="✓ Aucune rareté à corriger",
                    text_color=C["success"],
                )
                return
            parts = [f"{total['cartes']} carte(s) sur {total['classeurs']} classeur(s)"]
            if total["updates"]:
                parts.append(f"{total['updates']} corrigée(s)")
            if total["inserts"]:
                parts.append(f"{total['inserts']} rareté(s) ajoutée(s)")
            if total["deletes"]:
                parts.append(f"{total['deletes']} parasite(s) retiré(s)")
            if total["echecs"]:
                parts.append(f"{total['echecs']} non résolue(s) (réseau ?)")
            self._lbl_maint.configure(
                text="✓ " + " — ".join(parts),
                text_color=(C["success"] if total["echecs"] == 0 else C["warning"]),
            )

        try:
            from module.gestion_img.async_image_loader import run_async
            run_async(work, self, on_done)
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur : {e}", text_color=C["danger"],
            )

    def _recuperer_covers_yugipedia(self):
        """Télécharge, pour les classeurs existants SANS cover locale, l'image
        de cover (YGOPRODeck si dispo, sinon fallback Yugipedia).

        Réseau → arrière-plan (0 freeze UI). N'écrase jamais une cover déjà
        présente. Les classeurs sans aucune source de cover sont comptés en
        échec et laissés tels quels (fallback carte conservé à l'affichage).
        """
        self._lbl_maint.configure(
            text="🖼 Récupération des covers en cours…",
            text_color=C["text2"],
        )

        def work():
            import os
            from module.centralisation_dossier import CLASSEUR_FOLDER
            from module.img_dl.booster_service import (
                find_local_booster, download_booster_if_needed,
            )
            res = {"recus": 0, "deja": 0, "echecs": 0, "classeurs": 0}
            if not os.path.isdir(CLASSEUR_FOLDER):
                return res
            for nom in sorted(os.listdir(CLASSEUR_FOLDER)):
                db = os.path.join(CLASSEUR_FOLDER, nom, f"{nom}.db")
                if not os.path.isfile(db):
                    continue
                res["classeurs"] += 1
                if find_local_booster(nom):
                    res["deja"] += 1
                    continue
                try:
                    if download_booster_if_needed(nom):
                        res["recus"] += 1
                    else:
                        res["echecs"] += 1
                except Exception as e:
                    from module.logger_app import log
                    log.warning(f"Cover {nom}: {e}")
                    res["echecs"] += 1
            return res

        def on_done(res):
            if not res:
                self._lbl_maint.configure(
                    text="⚠ Erreur pendant la récupération des covers",
                    text_color=C["danger"],
                )
                return
            if res["recus"] == 0 and res["echecs"] == 0:
                self._lbl_maint.configure(
                    text=f"✓ Toutes les covers sont déjà présentes "
                         f"({res['deja']} classeur(s))",
                    text_color=C["success"],
                )
                return
            parts = [f"{res['recus']} cover(s) récupérée(s)"]
            if res["deja"]:
                parts.append(f"{res['deja']} déjà présente(s)")
            if res["echecs"]:
                parts.append(f"{res['echecs']} introuvable(s)")
            self._lbl_maint.configure(
                text="✓ " + " — ".join(parts),
                text_color=(C["success"] if res["echecs"] == 0 else C["warning"]),
            )

        try:
            from module.gestion_img.async_image_loader import run_async
            run_async(work, self, on_done)
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur : {e}", text_color=C["danger"],
            )

    def _corriger_overframe(self):
        """Force la complétion des prints Overframe dans cardinfo.db existant.

        Contexte : l'enrichissement Overframe ne s'exécute normalement qu'au
        build de la base ou à la création d'un classeur, et il est idempotent
        (revid Yugipedia). Si cardinfo.db existe déjà sans avoir reçu les
        prints Overframe (ou seulement partiellement), ces variantes restent
        non déclarées. Ce bouton ré-applique la réconciliation pour tous les
        sets Overframe connus, en ignorant l'idempotence (force=True).

        Réseau (Yugipedia, 1 req/s) → exécuté en arrière-plan (0 freeze UI).
        N'affecte que cardinfo.db : les classeurs déjà créés doivent être
        recréés pour intégrer les nouveaux prints (un classeur fige sa table
        au moment de sa création).
        """
        self._lbl_maint.configure(
            text="🃏 Correction des Overframe en cours…",
            text_color=C["text2"],
        )

        def work():
            from module.donnees.overframe_enrichment import (
                corriger_overframe_cardinfo,
            )
            from module.creation_classeur.creation_classeur_service import (
                propager_overframe_classeurs_existants,
            )
            # 1) Corriger cardinfo.db (force, ignore l'idempotence revid).
            base = corriger_overframe_cardinfo(force=True)
            # 2) Propager aux classeurs déjà créés (additif, possession gardée).
            #    On propage même si cardinfo n'a "rien" changé : un classeur a
            #    pu être créé AVANT une correction antérieure de cardinfo.db.
            propag = {"classeurs": 0, "ajoutes": 0, "corriges": 0, "erreurs": 0}
            if base.get("status") in ("ok", "rien"):
                propag = propager_overframe_classeurs_existants()
            return {"base": base, "propag": propag}

        def on_done(res):
            if not res:
                self._lbl_maint.configure(
                    text="⚠ Erreur pendant la correction des Overframe",
                    text_color=C["danger"],
                )
                return
            base   = res.get("base", {}) or {}
            propag = res.get("propag", {}) or {}
            status = base.get("status")
            if status == "cardinfo_absente":
                self._lbl_maint.configure(
                    text="⚠ cardinfo.db introuvable — lancez « Initialiser la base »",
                    text_color=C["danger"],
                )
                return
            if status == "erreur":
                self._lbl_maint.configure(
                    text=f"⚠ Erreur : {base.get('erreur', 'inconnue')}",
                    text_color=C["danger"],
                )
                return

            parts = []
            # Côté base interne
            if base.get("ajoutes") or base.get("corriges"):
                seg = f"cardinfo : +{base.get('ajoutes', 0)} print(s)"
                if base.get("corriges"):
                    seg += f", {base['corriges']} cadre(s) corrigé(s)"
                parts.append(seg)
            # Côté classeurs
            if propag.get("classeurs"):
                seg = f"{propag['classeurs']} classeur(s)"
                bits = []
                if propag.get("ajoutes"):
                    bits.append(f"+{propag['ajoutes']} carte(s)")
                if propag.get("corriges"):
                    bits.append(f"{propag['corriges']} cadre(s) corrigé(s)")
                if bits:
                    seg += " (" + ", ".join(bits) + ")"
                parts.append(seg)

            if not parts:
                self._lbl_maint.configure(
                    text="✓ Overframe déjà à jour (aucune correction nécessaire)",
                    text_color=C["success"],
                )
                return

            warn = propag.get("erreurs", 0) > 0
            txt = "✓ " + " — ".join(parts)
            if propag.get("classeurs"):
                txt += "  · rouvrez le classeur pour voir les cartes"
            if warn:
                txt += f"  · {propag['erreurs']} classeur(s) en erreur"
            self._lbl_maint.configure(
                text=txt,
                text_color=(C["warning"] if warn else C["success"]),
            )

        try:
            from module.gestion_img.async_image_loader import run_async
            run_async(work, self, on_done)
        except Exception as e:
            self._lbl_maint.configure(
                text=f"⚠ Erreur : {e}", text_color=C["danger"],
            )

    def _demander_redemarrage(self):
        """Affiche le bandeau de redémarrage (dans son holder, sous la Navbar)
        s'il ne l'est pas déjà. Appelé par les réglages qui ne s'appliquent
        qu'au prochain démarrage (langue UI, taille de police)."""
        try:
            # Réactive l'ajustement à la hauteur du bandeau (sort du collapse).
            self._restart_holder.pack_propagate(True)
        except Exception as e:
            log.warning(f"_demander_redemarrage: {e}")

    def _masquer_redemarrage(self):
        try:
            # Re-collapse le holder à 0 px (aucun espace mort sous la navbar).
            self._restart_holder.pack_propagate(False)
            self._restart_holder.configure(height=0)
        except Exception:
            pass

    def _redemarrer(self):
        """Relance l'application et ferme l'instance courante."""
        from module.utilitaire.redemarrage import redemarrer
        ok = redemarrer(self.winfo_toplevel())
        if not ok:
            # Échec du lancement : informer sans fermer.
            try:
                self._masquer_redemarrage()
            except Exception:
                pass

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")

    def charger(self):
        """Appelé par NavigationController — rien à charger dynamiquement
        car tout est rempli depuis les préférences au __init__. Si la
        préférence a changé depuis un autre écran, on peut rebuild ici."""
        pass

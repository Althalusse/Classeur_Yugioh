"""
ecran_accueil.py — Écran 1 : liste des classeurs en grille.

Évolutions cycle 6 :
  1. Ratio correct : l'image de couverture est désormais affichée à 220×230
     (proche du ratio YGO 59:86) avec Image.thumbnail() qui PRÉSERVE le
     ratio au lieu de déformer. Plus de cartes aplaties.
  2. Image booster : si une image de booster officielle est trouvée dans
     cardinfo.db.set_locales.booster_image_url, elle est utilisée en
     priorité. Sinon fallback sur une carte random du classeur (ancien
     comportement). Téléchargement asynchrone en arrière-plan.
  3. Bouton ⚙ Options dans la navbar (remplace le bouton 🌐).
  4. Bouton « Grille N×N » sous la progress bar de chaque ClasseurCard,
     ouvre un mini-dialog pour override la grille spécifique à ce classeur
     (sauvegardée dans meta.colonnes/lignes via save_classeur_config).
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
from PIL import Image

from module.theme import C
from module.ui.composants import (
    gold_button, secondary_button, icon_button, danger_button,
    StatCard, Navbar, separator, progress_bar, CentreActiviteButton,
    search_entry,
)
from module.centralisation_dossier import CLASSEUR_FOLDER, IMAGES_SMALL_FOLDER
from module.creation_classeur.creation_classeur_service import (
    get_set_title, get_classeur_meta, get_classeur_meta_full, save_classeur_config,
    get_n_raretes_override, save_n_raretes_override, get_premiere_image_id,
    supprimer_classeur,
)
from module.statistique.statistique_collection_service import get_stats_collection
from module.config_langue import load_langue
from module.img_dl.booster_service import get_booster_image_path
from module.config import preferences
from module.gestion_img.async_image_loader import run_async
from module.i18n import t
from module.logger_app import log


# ─────────────────────────────────────────────────────────────────────────────
# Dialog d'override de grille par classeur
# ─────────────────────────────────────────────────────────────────────────────

class DialogGrilleOverride(ctk.CTkToplevel):
    """Dialog compact pour définir colonnes × lignes d'un classeur spécifique."""

    def __init__(self, parent, code: str, current_cols: int, current_rows: int,
                 on_save=None):
        super().__init__(parent)
        self.title(f"Grille du classeur {code}")
        self.geometry("380x220")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        self._code = code
        self._on_save = on_save

        ctk.CTkLabel(
            self, text=f"Grille pour « {code} »",
            font=("Georgia", 13, "bold"), text_color=C["text"],
        ).pack(pady=(18, 4))

        gmin, gmax = preferences.grille_bounds()
        ctk.CTkLabel(
            self,
            text=f"Valeurs entre {gmin} et {gmax}",
            font=("Outfit", 10), text_color=C["text3"],
        ).pack()

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=16)

        ctk.CTkLabel(
            row, text="Colonnes :",
            font=("Outfit", 11), text_color=C["text"],
        ).grid(row=0, column=0, padx=(0, 8), pady=4, sticky="e")
        self._cols_var = ctk.StringVar(value=str(current_cols))
        ctk.CTkEntry(
            row, textvariable=self._cols_var, width=60,
            font=("JetBrains Mono", 11), justify="center",
        ).grid(row=0, column=1, padx=(0, 16), pady=4)

        ctk.CTkLabel(
            row, text="Lignes :",
            font=("Outfit", 11), text_color=C["text"],
        ).grid(row=0, column=2, padx=(0, 8), pady=4, sticky="e")
        self._rows_var = ctk.StringVar(value=str(current_rows))
        ctk.CTkEntry(
            row, textvariable=self._rows_var, width=60,
            font=("JetBrains Mono", 11), justify="center",
        ).grid(row=0, column=3, pady=4)

        self._lbl_err = ctk.CTkLabel(
            self, text="", font=("Outfit", 10), text_color=C["danger"],
        )
        self._lbl_err.pack()

        btn_row = ctk.CTkFrame(self, fg_color="transparent")
        btn_row.pack(pady=(14, 16))
        secondary_button(
            btn_row, "Annuler", command=self.destroy, width=110,
        ).pack(side="left", padx=6)
        gold_button(
            btn_row, "Enregistrer", command=self._sauver, width=130,
        ).pack(side="left", padx=6)

        self.bind("<Escape>", lambda e: self.destroy())
        self.bind("<Return>", lambda e: self._sauver())

    def _sauver(self):
        gmin, gmax = preferences.grille_bounds()
        try:
            c = int(self._cols_var.get())
            r = int(self._rows_var.get())
        except ValueError:
            self._lbl_err.configure(text="⚠ Valeurs numériques requises")
            return
        if not (gmin <= c <= gmax) or not (gmin <= r <= gmax):
            self._lbl_err.configure(
                text=f"⚠ Valeurs doivent être entre {gmin} et {gmax}"
            )
            return

        db_path = os.path.join(CLASSEUR_FOLDER, self._code, f"{self._code}.db")
        try:
            save_classeur_config(db_path, c, r)
        except Exception as e:
            self._lbl_err.configure(text=f"⚠ Erreur : {e}")
            return

        if self._on_save:
            try:
                self._on_save(self._code, c, r)
            except Exception as e:
                log.warning(f"DialogGrilleOverride callback : {e}")
        self.destroy()


class DialogRaretesOverride(ctk.CTkToplevel):
    """Dialog compact pour override le nombre de raretés affichées par
    carte+artwork sur un classeur spécifique.

    Trois états possibles :
      - Vide / "(global)" : pas d'override → utilise la valeur globale
        de preferences.get_n_raretes_par_artwork().
      - 0 : override explicite "afficher toutes les raretés" (utile pour
        contourner un global ≥ 1 sur ce classeur précis).
      - N ≥ 1 : override explicite "N plus rares".
    """

    def __init__(self, parent, code: str, on_save=None):
        super().__init__(parent)
        self.title(f"Raretés du classeur {code}")
        self.geometry("440x260")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        self._code = code
        self._on_save = on_save

        ctk.CTkLabel(
            self, text=f"Raretés affichées pour « {code} »",
            font=("Georgia", 13, "bold"), text_color=C["text"],
        ).pack(pady=(18, 4))

        global_n = preferences.get_n_raretes_par_artwork()
        n_min, n_max = preferences.n_raretes_bounds()

        ctk.CTkLabel(
            self,
            text=(
                f"Vide = utilise la préférence globale (actuellement : "
                f"{global_n if global_n > 0 else 'toutes'})\n"
                f"0 = afficher toutes les raretés (override explicite)\n"
                f"N de 1 à {n_max} = afficher les N plus rares"
            ),
            font=("Outfit", 10), text_color=C["text3"],
            justify="center",
        ).pack(pady=(0, 8))

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=10)

        ctk.CTkLabel(
            row, text="N raretés :",
            font=("Outfit", 11), text_color=C["text"],
        ).grid(row=0, column=0, padx=(0, 8), pady=4, sticky="e")

        # Pré-remplit avec la valeur d'override existante, ou vide si pas d'override
        override_actuel = get_n_raretes_override(code)
        initial = "" if override_actuel is None else str(override_actuel)
        self._n_var = ctk.StringVar(value=initial)
        self._entry = ctk.CTkEntry(
            row, textvariable=self._n_var, width=80,
            font=("JetBrains Mono", 11), justify="center",
            placeholder_text="(global)",
            fg_color=C["bg3"], border_color=C["border"], border_width=1,
            text_color=C["text"],
        )
        self._entry.grid(row=0, column=1, pady=4)

        self._lbl_err = ctk.CTkLabel(
            self, text="", font=("Outfit", 10), text_color=C["danger"],
        )
        self._lbl_err.pack()

        btn_row = ctk.CTkFrame(self, fg_color="transparent")
        btn_row.pack(pady=(14, 16))
        secondary_button(
            btn_row, "Annuler", command=self.destroy, width=110,
        ).pack(side="left", padx=6)
        gold_button(
            btn_row, "Enregistrer", command=self._sauver, width=130,
        ).pack(side="left", padx=6)

        self.bind("<Escape>", lambda e: self.destroy())
        self.bind("<Return>", lambda e: self._sauver())

    def _sauver(self):
        raw = self._n_var.get().strip()
        n_min, n_max = preferences.n_raretes_bounds()

        if raw == "":
            # Vide → suppression de l'override → re-tombe sur le global
            try:
                save_n_raretes_override(self._code, None)
            except Exception as e:
                self._lbl_err.configure(text=f"⚠ Erreur : {e}")
                return
            n_save: int | None = None
        else:
            try:
                n = int(raw)
            except ValueError:
                self._lbl_err.configure(text="⚠ Valeur numérique requise (ou vide)")
                return
            if not (n_min <= n <= n_max):
                self._lbl_err.configure(
                    text=f"⚠ N doit être entre {n_min} et {n_max}"
                )
                return
            try:
                save_n_raretes_override(self._code, n)
            except Exception as e:
                self._lbl_err.configure(text=f"⚠ Erreur : {e}")
                return
            n_save = n

        if self._on_save:
            try:
                self._on_save(self._code, n_save)
            except Exception as e:
                log.warning(f"DialogRaretesOverride callback : {e}")
        self.destroy()


# ─────────────────────────────────────────────────────────────────────────────
# Carte classeur (nouveau ratio + menu actions)
# ─────────────────────────────────────────────────────────────────────────────

class ClasseurCard(ctk.CTkFrame):
    """Carte d'un classeur sur l'accueil.

    Dimensions redimensionnées pour respecter le ratio YGO 59:86 :
      - image cover : 220×230 (au lieu de 220×148 qui écrasait)
      - carte totale : 220×350 (image + nom + stats + progress)
    L'image est placée avec Image.thumbnail() qui préserve le ratio.

    Les actions "Définir la taille" et "Supprimer" sont regroupées dans
    un menu dropdown ouvert par le bouton ⋮ en haut-gauche de l'image.
    """
    CARD_W       = 220
    CARD_IMG_H   = 230   # zone image
    CARD_H       = 350   # hauteur totale — réduite car bouton grille du bas retiré

    def __init__(self, parent, data: dict, on_click=None, on_delete=None,
                 on_grille_change=None):
        super().__init__(
            parent,
            fg_color=C["bg_card"],
            border_color=C["border"],
            border_width=1,
            corner_radius=12,
            width=self.CARD_W,
            height=self.CARD_H,
            cursor="hand2",
        )
        self.pack_propagate(False)
        self._data             = data
        self._on_click         = on_click
        self._on_delete        = on_delete
        self._on_grille_change = on_grille_change
        self._img_ref          = None
        self._build()
        self._bind_hover()

    def _build(self):
        d        = self._data
        code     = d.get("code", "")
        nom      = d.get("nom",  code)
        total    = d.get("total", 0)
        poss     = d.get("possedees", 0)
        pct      = d.get("pourcentage", 0.0)
        img_path = d.get("cover_image")
        cols     = d.get("cols", 3)
        rows     = d.get("rows", 3)

        # ── Zone image (ratio préservé) ──────────────────────────────────
        img_frame = ctk.CTkFrame(
            self, fg_color=C["bg3"], corner_radius=8,
            height=self.CARD_IMG_H, width=self.CARD_W,
        )
        img_frame.pack(fill="x")
        img_frame.pack_propagate(False)

        self._render_cover(img_frame, img_path)

        # Badge code (coin haut-droit)
        self._code_badge = ctk.CTkLabel(
            img_frame, text=code,
            fg_color=C["bg"], text_color=C["gold"],
            font=("Consolas", 9),
            corner_radius=4, padx=6, pady=2,
        )
        self._code_badge.place(relx=1.0, rely=0.0, anchor="ne", x=-6, y=6)

        # Bouton menu actions (coin haut-gauche)
        # Remplace l'ancien bouton 🗑 : regroupe "Définir la grille" et
        # "Supprimer" dans un dropdown. Évite le bug de clics parasites
        # du bouton grille quand il était sous la progress bar
        # (les sous-widgets internes du CTkButton recevaient le binding
        # <Button-1> de "ouvrir classeur").
        self._menu_btn = ctk.CTkButton(
            img_frame, text="⋮", width=26, height=26,
            fg_color="transparent", hover_color=C["bg_hover"],
            text_color=C["text"], font=("Segoe UI", 16, "bold"),
            corner_radius=4, border_width=0,
            command=self._ouvrir_menu_actions,
        )
        self._menu_btn.place(x=6, y=6)
        # Conservé pour compatibilité avec les hover callbacks
        self._del_btn = self._menu_btn

        # ── Zone info (nom, stats, progress) ─────────────────────────────
        info = ctk.CTkFrame(self, fg_color="transparent")
        info.pack(fill="both", expand=True, padx=12, pady=(8, 10))

        ctk.CTkLabel(
            info,
            text=(nom[:36] + "…" if len(nom) > 36 else nom),
            font=("Outfit", 11, "bold"),
            text_color=C["text"],
            anchor="w", wraplength=self.CARD_W - 28, justify="left",
        ).pack(anchor="w")

        # Stats line : police agrandie (9 → 11), grille intégrée en préfixe.
        # Format : "3×3  ·  0 / 567 cartes  ·  0%"
        self._stats_lbl = ctk.CTkLabel(
            info,
            text=f"{cols}×{rows}  ·  {poss} / {total} cartes  ·  {pct:.0f}%",
            font=("Consolas", 11),
            text_color=C["text3"], anchor="w",
        )
        self._stats_lbl.pack(anchor="w", pady=(4, 8))

        pbar = progress_bar(info)
        pbar.pack(fill="x")
        pbar.set(min(1.0, pct / 100))
        # Stocke les valeurs pour pouvoir régénérer le label quand la
        # grille change sans refaire tout le build
        self._poss_cache  = poss
        self._total_cache = total
        self._pct_cache   = pct

    def _render_cover(self, img_frame, img_path):
        """Affiche l'image de couverture en préservant le ratio.

        Le decode + thumbnail + paste (coûteux) est déporté HORS DU THREAD UI :
        un placeholder est affiché immédiatement, puis l'image réelle est
        injectée dès qu'elle est prête. L'accueil ne se fige plus, quel que
        soit le nombre de classeurs.
        """
        # Placeholder immédiat (emoji discret), conservé pour le retirer ensuite.
        self._cover_ph = ctk.CTkLabel(
            img_frame, text="📂", font=("Segoe UI", 48),
            text_color=C["text3"],
        )
        self._cover_ph.place(relx=0.5, rely=0.5, anchor="center")

        if not (img_path and os.path.exists(img_path)):
            return  # pas d'image → on reste sur le placeholder emoji

        cw, ch = self.CARD_W, self.CARD_IMG_H

        def _work():
            # Thread worker : renvoie l'image composée (PIL), ou lève → None.
            pil = Image.open(img_path).convert("RGB")
            # thumbnail() : in-place, préserve le ratio, tient dans (cw, ch).
            pil.thumbnail((cw, ch), Image.LANCZOS)
            # Fond aux dimensions exactes du cadre + image centrée (évite les
            # bords étirés sur une vraie carte 59:86 dans un cadre 220×230).
            bg = Image.new("RGB", (cw, ch), C["bg3"])  # fond cohérent avec la carte
            ox = (cw - pil.size[0]) // 2
            oy = (ch - pil.size[1]) // 2
            bg.paste(pil, (ox, oy))
            return bg

        def _done(bg):
            # Thread UI : crée la CTkImage et l'affiche par-dessus le placeholder.
            try:
                if not img_frame.winfo_exists():
                    return
            except Exception:
                return
            if bg is None:
                return  # échec decode → on garde le placeholder emoji
            try:
                self._img_ref = ctk.CTkImage(bg, size=(cw, ch))
                lbl = ctk.CTkLabel(img_frame, image=self._img_ref, text="")
                lbl.place(x=0, y=0, relwidth=1, relheight=1)
                self._cover_ph.destroy()
                # L'image couvre tout le cadre : on REMONTE les overlays
                # (badge code + bouton ⋮) au-dessus d'elle pour qu'ils restent
                # visibles. NB : lbl.lower() masquerait l'image derrière le
                # canvas de fond du CTkFrame -> on lift les overlays plutôt.
                try:
                    self._code_badge.lift()
                except Exception:
                    pass
                try:
                    self._menu_btn.lift()
                except Exception:
                    pass
                # Le label image est créé APRÈS _bind_hover (chargement async) :
                # on re-câble hover + clic « ouvrir classeur » pour qu'un clic
                # sur l'image ouvre bien le classeur (et pas seulement le bas).
                self._bind_hover()
            except Exception as e:
                log.warning(f"ClasseurCard._render_cover done: {e}")

        run_async(_work, img_frame, _done)

    def _ouvrir_menu_actions(self, event=None):
        """Ouvre un mini-dropdown avec les actions : définir grille, supprimer.

        Le dropdown est un CTkToplevel sans bordures. Positionné au curseur
        quand il est déclenché par un clic droit (event fourni), sinon juste
        sous le bouton ⋮. Il se ferme automatiquement quand il perd le focus
        (clic ailleurs, Escape, sélection d'un item).
        """
        code = self._data.get("code", "")
        if event is not None:
            # Clic droit : on positionne le menu à l'endroit du clic.
            x, y = event.x_root, event.y_root
        else:
            try:
                x = self._menu_btn.winfo_rootx()
                y = self._menu_btn.winfo_rooty() + self._menu_btn.winfo_height() + 2
            except Exception:
                x, y = 100, 100

        menu = ctk.CTkToplevel(self.winfo_toplevel())
        menu.overrideredirect(True)   # retire la barre de titre
        menu.attributes("-topmost", True)
        menu.configure(fg_color=C["bg_card"])
        menu.geometry(f"+{x}+{y}")

        # Petite bordure custom (CTkToplevel ne permet pas border_width)
        border = ctk.CTkFrame(
            menu, fg_color=C["bg_card"],
            border_color=C["border2"], border_width=1,
            corner_radius=6,
        )
        border.pack(fill="both", expand=True)

        def close_menu():
            try:
                menu.destroy()
            except Exception:
                pass

        def action_grille():
            close_menu()
            self._ouvrir_dialog_grille()

        def action_raretes():
            close_menu()
            self._ouvrir_dialog_raretes()

        def action_delete():
            close_menu()
            if self._on_delete:
                self._on_delete(code)

        # Items du menu
        ctk.CTkButton(
            border, text="📐   Définir la taille du classeur",
            command=action_grille,
            fg_color="transparent", hover_color=C["bg_hover"],
            text_color=C["text"], anchor="w",
            font=("Outfit", 11),
            corner_radius=4, border_width=0,
            height=34, width=240,
        ).pack(fill="x", padx=4, pady=(4, 0))

        ctk.CTkButton(
            border, text="🃏   Raretés affichées par carte",
            command=action_raretes,
            fg_color="transparent", hover_color=C["bg_hover"],
            text_color=C["text"], anchor="w",
            font=("Outfit", 11),
            corner_radius=4, border_width=0,
            height=34, width=240,
        ).pack(fill="x", padx=4, pady=(0, 0))

        # Séparateur fin
        ctk.CTkFrame(
            border, height=1, fg_color=C["border"], corner_radius=0,
        ).pack(fill="x", padx=8, pady=4)

        ctk.CTkButton(
            border, text="🗑   Supprimer le classeur",
            command=action_delete,
            fg_color="transparent", hover_color=C["danger"],
            text_color=C["danger_text"], anchor="w",
            font=("Outfit", 11),
            corner_radius=4, border_width=0,
            height=34, width=240,
        ).pack(fill="x", padx=4, pady=(0, 4))

        # Auto-fermeture : Escape + perte de focus
        menu.bind("<Escape>", lambda e: close_menu())
        menu.bind("<FocusOut>", lambda e: close_menu())
        try:
            menu.focus_force()
        except Exception:
            pass

    def _ouvrir_dialog_grille(self):
        """Ouvre le dialog d'override de grille pour ce classeur."""
        code = self._data.get("code", "")
        cols = self._data.get("cols", 3)
        rows = self._data.get("rows", 3)

        def on_save(c, new_c, new_r):
            # Met à jour le data
            self._data["cols"] = new_c
            self._data["rows"] = new_r
            # Régénère le label stats avec la nouvelle grille
            try:
                self._stats_lbl.configure(
                    text=(f"{new_c}×{new_r}  ·  {self._poss_cache} / "
                          f"{self._total_cache} cartes  ·  "
                          f"{self._pct_cache:.0f}%"),
                )
            except Exception:
                pass
            # Remonte à l'accueil
            if self._on_grille_change:
                try:
                    self._on_grille_change(c, new_c, new_r)
                except Exception as e:
                    log.warning(f"on_grille_change : {e}")

        DialogGrilleOverride(
            self.winfo_toplevel(), code, cols, rows,
            on_save=on_save,
        )

    def _ouvrir_dialog_raretes(self):
        """Ouvre le dialog d'override du nombre de raretés affichées."""
        code = self._data.get("code", "")
        # Pas de callback de mise à jour visuelle nécessaire à l'accueil :
        # le filtre s'applique à l'ouverture du classeur, pas sur la
        # vignette accueil. On laisse on_save à None.
        DialogRaretesOverride(self.winfo_toplevel(), code)

    def _bind_hover(self):
        code = self._data.get("code", "")

        def enter(e):
            self.configure(border_color=C["gold_dim"])
            self._del_btn.configure(fg_color=C["bg"])
        def leave(e):
            self.configure(border_color=C["border"])
            self._del_btn.configure(fg_color="transparent")
        def click(e):
            if self._on_click:
                self._on_click(code)
        def menu(e):
            # Clic droit n'importe où sur la vignette -> menu d'actions au curseur.
            self._ouvrir_menu_actions(e)
            return "break"

        for w in _all_children(self):
            w.bind("<Enter>", enter)
            w.bind("<Leave>", leave)
            w.bind("<Button-3>", menu)
            # Pas de clic-pour-ouvrir sur les boutons (suppression, grille)
            if not isinstance(w, ctk.CTkButton):
                w.bind("<Button-1>", click)
        self.bind("<Enter>", enter)
        self.bind("<Leave>", leave)
        self.bind("<Button-3>", menu)
        self.bind("<Button-1>", click)


def _all_children(widget):
    """Parcourt la descendance mais ne descend PAS dans les CTkButton.

    Un CTkButton contient en interne un Canvas + un Label. Si on descend
    dedans pour binder <Button-1>, ces sous-widgets reçoivent aussi le
    binding 'ouvrir classeur' et interceptent les clics destinés au bouton
    lui-même. Résultat : le clic sur le bouton fait deux actions (ouvrir
    le classeur ET la command du bouton) ou pire, seul le clic carte
    s'exécute et la command du bouton est perdue.

    En s'arrêtant aux CTkButton (qui gèrent eux-mêmes leur command), les
    clics sur les boutons vont uniquement à leur handler natif.
    """
    yield widget
    if isinstance(widget, ctk.CTkButton):
        return
    for child in widget.winfo_children():
        yield from _all_children(child)


# ─────────────────────────────────────────────────────────────────────────────
# Dialog de suppression (inchangé)
# ─────────────────────────────────────────────────────────────────────────────

class DialogSuppression(ctk.CTkToplevel):
    def __init__(self, parent, code: str, nom: str, on_confirm):
        super().__init__(parent)
        self.title("Supprimer le classeur ?")
        self.geometry("400x200")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        ctk.CTkLabel(self, text="Supprimer le classeur ?",
                     font=("Georgia", 14, "bold"),
                     text_color=C["text"]).pack(pady=(20, 8))
        ctk.CTkLabel(self, text=f"« {nom or code} » sera supprimé définitivement.",
                     font=("Outfit", 11),
                     text_color=C["text2"]).pack()

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(pady=20)
        secondary_button(row, "Annuler",  command=self.destroy,    width=110).pack(side="left", padx=8)
        danger_button(row,   "Supprimer", command=lambda: (on_confirm(code), self.destroy()), width=110).pack(side="left", padx=8)
        self.bind("<Escape>", lambda e: self.destroy())


# ─────────────────────────────────────────────────────────────────────────────
# Écran Accueil
# ─────────────────────────────────────────────────────────────────────────────

class EcranAccueil(ctk.CTkFrame):

    # Disposition de la grille de display (classeurs)
    _GRID_GAP         = 16   # écart fixe entre display (= 2× le GAP des cartes)
    _GRID_MAX_COLS    = 12   # plafond colonnes (garde-fou écrans très larges)
    # Marge horizontale réservée = padx du scroll (2×16) + scrollbar verticale
    # + sécurité. Volontairement généreuse : on préfère sous-estimer le nombre
    # de colonnes (un peu de vide à droite) plutôt que déborder et déclencher
    # un défilement horizontal. Réduire cette valeur = plus de colonnes.
    _GRID_SIDE_MARGIN = 48

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._classeurs: list[dict] = []
        self._affiches: list[dict] = []   # sous-ensemble filtré par la recherche
        self._grid_cols = 3
        self._stat_classeurs = None
        self._stat_total     = None
        self._stat_poss      = None
        self._resize_after   = None   # id du after() de debounce du resize
        self._search_var     = ctk.StringVar()
        self._search_after   = None   # id du after() de debounce de la recherche
        self._build()

    def _build(self):
        # Navbar
        Navbar(
            self,
            title="Yu-Gi-Oh! Collection",
            right_factory=self._build_nav_right,
        ).pack(fill="x")

        # Stats row
        stats = ctk.CTkFrame(self, fg_color="transparent")
        stats.pack(fill="x", padx=24, pady=(20, 0))
        stats.columnconfigure((0, 1, 2), weight=1)

        self._stat_classeurs = StatCard(stats, "0", "Classeurs")
        self._stat_classeurs.grid(row=0, column=0, padx=8, sticky="ew")
        self._stat_total = StatCard(stats, "0", "Cartes total")
        self._stat_total.grid(row=0, column=1, padx=8, sticky="ew")
        self._stat_poss = StatCard(stats, "0", "Possédées")
        self._stat_poss.grid(row=0, column=2, padx=8, sticky="ew")

        # Barre de recherche (filtre la grille des classeurs par nom / code)
        search_row = ctk.CTkFrame(self, fg_color="transparent")
        search_row.pack(fill="x", padx=24, pady=(16, 0))
        search_entry(
            search_row, textvariable=self._search_var,
            placeholder=t("search.binder"),
        ).pack(fill="x")
        self._search_var.trace_add("write", self._on_recherche)

        separator(self).pack(fill="x", padx=24, pady=16)

        self._scroll = ctk.CTkScrollableFrame(
            self, fg_color="transparent", corner_radius=0
        )
        self._scroll.pack(fill="both", expand=True, padx=16, pady=(0, 16))
        # Redimensionnement en temps réel : on lie <Configure> sur self ET sur
        # la fenêtre. CustomTkinter peut rediriger .bind() vers un canvas
        # interne (l'event n'a alors pas event.widget == self) — on ne filtre
        # donc pas sur event.widget. Le binding sur le toplevel sert de filet
        # si celui sur self ne porte pas la bonne géométrie.
        self.bind("<Configure>", self._on_resize)
        try:
            self.winfo_toplevel().bind("<Configure>", self._on_resize, add="+")
        except Exception:
            pass

    def _build_nav_right(self, parent):
        """Crée les boutons droits de la navbar.

        Cycle 6 : le bouton 🌐 FR/EN est remplacé par ⚙ Options. Le toggle
        de langue reste accessible dans l'écran Options.

        Migration M2 (avr. 2026) : ajout du bouton "🔄 MAJ BDD" qui ouvre
        la fenêtre de contrôle de version distante (UpdateWindow CTk).

        Lot 3 import/export (avr. 2026) : ajout du bouton "📋 Import/Export"
        en 1ère position pour ouvrir la fenêtre Scanflip globale.
        Ordre : Import/Export → MAJ BDD → Options → Nouveau classeur.

        Centre d'activité (mai 2026) : ajout du bouton "📥 Activité" avec
        badge live qui affiche le nombre de tâches en attente/cours dans
        FileAttenteClasseur (création de classeurs + téléchargement
        d'images, y compris ceux déclenchés par import CSV).
        """
        # Ordre par nature : statut → consultation → données → méta → système → action.
        # 📥 Activité — badge live, première position pour visibilité
        CentreActiviteButton(parent).pack(side="left", padx=4)

        # Consultation collection
        icon_button(
            parent, "📊  Statistiques",
            command=self._ouvrir_statistiques,
        ).pack(side="left", padx=4)
        # Opérations sur les données
        icon_button(
            parent, "📋  Import/Export",
            command=self._ouvrir_import_export,
        ).pack(side="left", padx=4)
        icon_button(
            parent, "🔄  MAJ BDD",
            command=self._ouvrir_update_bdd,
        ).pack(side="left", padx=4)
        # Méta (crédits / soutien)
        icon_button(
            parent, "🙏  Merci",
            command=self._ouvrir_merci,
        ).pack(side="left", padx=4)
        icon_button(
            parent, "☕  Contribution",
            command=self._ouvrir_donation,
        ).pack(side="left", padx=4)
        # Système
        icon_button(
            parent, "⚙  Options",
            command=self._ouvrir_options,
        ).pack(side="left", padx=4)
        # Action principale (CTA)
        gold_button(
            parent, "+ Nouveau classeur",
            command=self._ouvrir_selecteur,
        ).pack(side="left", padx=4)

    def _ouvrir_options(self):
        if self._navigate_to:
            self._navigate_to("options")

    def _ouvrir_statistiques(self):
        if self._navigate_to:
            self._navigate_to("statistique")

    def _ouvrir_donation(self):
        if self._navigate_to:
            self._navigate_to("donation")

    def _ouvrir_merci(self):
        if self._navigate_to:
            self._navigate_to("merci")

    def _ouvrir_update_bdd(self):
        """Ouvre la fenêtre de mise à jour de la base de données distante.

        Délègue toute la logique à update_window.show_update_window() qui
        gère :
          - Vérification de la version distante
          - Confirmation utilisateur si MAJ disponible
          - Backup auto + run_init en thread + mise à jour last_update.txt
        """
        try:
            from module.ui.update_window import show_update_window
            # Le parent est self.winfo_toplevel() pour ancrer la modale
            # à la fenêtre principale de l'app.
            show_update_window(self.winfo_toplevel())
        except Exception as e:
            log.warning(f"EcranAccueil._ouvrir_update_bdd: {e}")

    def _ouvrir_import_export(self):
        """Ouvre la fenêtre Import/Export Scanflip (Lot 3).

        Sans pré-sélection de classeur (toute la collection par défaut).
        Onglet par défaut : Export. Callback de rafraîchissement appelle
        _load_data pour mettre à jour les chiffres de l'accueil après un
        import réussi.
        """
        try:
            from module.ui.dialog_import_export import show_import_export_dialog
            show_import_export_dialog(
                self.winfo_toplevel(),
                classeur_initial=None,
                onglet_initial="export",
                on_update=self._refresh_after_import,
            )
        except Exception as e:
            log.warning(f"EcranAccueil._ouvrir_import_export: {e}")

    def _refresh_after_import(self):
        """Recharge les données de l'accueil après un import réussi."""
        try:
            self._load_data()
        except Exception as e:
            log.warning(f"EcranAccueil._refresh_after_import: {e}")

    def _safe_after(self, *args, **kwargs):
        """Wrapper défensif autour de self.after() pour éviter les crashes
        'main thread is not in main loop' quand le widget n'est plus attaché
        à un mainloop actif (ex : app en train de se fermer, écran détruit,
        ou interaction avec show_init_window au premier lancement).

        Appelé depuis les threads worker — ignore silencieusement si le
        widget n'accepte plus de callbacks.
        """
        try:
            if not self.winfo_exists():
                return
        except Exception:
            return
        try:
            self.after(*args, **kwargs)
        except RuntimeError:
            # "main thread is not in main loop" — l'app est probablement
            # en train de se fermer, ou mainloop n'a pas encore démarré
            pass
        except Exception as e:
            log.warning(f"_safe_after : {e}")

    def rafraichir(self):
        threading.Thread(target=self._load_data, daemon=True).start()

    def _load_data(self):
        data         = []
        total_cartes = 0
        total_poss   = 0

        if not os.path.exists(CLASSEUR_FOLDER):
            self._safe_after(0, self._render, data, 0, 0)
            return

        use_fr    = load_langue() == "FR"
        stats_map = {}
        try:
            for s in get_stats_collection():
                stats_map[s["nom"].upper()] = s
        except Exception:
            pass

        for code in sorted(os.listdir(CLASSEUR_FOLDER)):
            path = os.path.join(CLASSEUR_FOLDER, code)
            if not os.path.isdir(path):
                continue
            db  = os.path.join(path, f"{code}.db")
            nom = get_set_title(code, use_fr)
            st  = stats_map.get(code.upper(), {})
            # Optimisation M6 : count + grille en UNE seule ouverture SQLite
            # (au lieu de 2 séparées via get_classeur_card_count +
            # get_classeur_config). Si la BDD n'existe pas, on retombe sur
            # les valeurs par défaut applicatives (même comportement qu'avant).
            # Optimisation : count + grille + image_id de couverture en UNE
            # seule ouverture SQLite (au lieu de get_classeur_meta +
            # get_premiere_image_id = 2 ouvertures par classeur).
            if os.path.exists(db):
                nb, cols, rows, image_id = get_classeur_meta_full(db)
            else:
                nb = 0
                cols, rows = preferences.get_grille_defaut()
                image_id = None
            data.append({
                "code":        code,
                "nom":         nom or code,
                "total":       nb,
                "possedees":   st.get("possedees", 0),
                "pourcentage": st.get("pourcentage", 0.0),
                "cover_image": self._find_cover(
                    code, db, image_id=image_id, image_id_known=True
                ),
                "cols":        cols,
                "rows":        rows,
            })
            total_cartes += nb
            total_poss   += st.get("possedees", 0)

        self._safe_after(0, self._render, data, total_cartes, total_poss)

        # Lance les téléchargements async des boosters manquants pour
        # améliorer la couverture au prochain rafraîchissement
        threading.Thread(
            target=self._ensure_booster_images,
            args=([d["code"] for d in data],),
            daemon=True,
        ).start()

    def _find_cover(self, code: str, db_path: str,
                    image_id: str | None = None,
                    image_id_known: bool = False) -> str | None:
        """Cherche une image de couverture pour le classeur.

        Priorité (cycle 6) :
          1. Image de booster officielle (img/boosters/<CODE>.ext)
             via booster_service — téléchargée depuis cardinfo.db si absente
             (mais de façon asynchrone pour ne pas bloquer le chargement).
          2. Carte random du classeur (ancien comportement) :
             img/small/{first_card_image_id}.jpg
          3. Fallback per-classeur (ancien dossier img/<CODE>/)
          4. None → placeholder 📂

        image_id / image_id_known : permet à l'appelant (_load_data) de fournir
        le card_image_id déjà lu via get_classeur_meta_full, évitant ainsi une
        2ᵉ ouverture SQLite (get_premiere_image_id). Si image_id_known est
        False, l'id est lu à la demande (comportement historique).
        """
        # 1. Booster local (sans DL, non bloquant)
        booster = get_booster_image_path(code, auto_download=False)
        if booster:
            return booster

        # 2. Carte partagée (img/small/{card_image_id}.jpg)
        if os.path.exists(IMAGES_SMALL_FOLDER):
            try:
                if not image_id_known and os.path.exists(db_path):
                    image_id = get_premiere_image_id(db_path)
                if image_id:
                    p = os.path.join(IMAGES_SMALL_FOLDER, f"{image_id}.jpg")
                    if os.path.exists(p):
                        return p
            except Exception:
                pass

        # 3. Fallback per-classeur (anciens YGOJSON)
        per = os.path.join(os.path.dirname(IMAGES_SMALL_FOLDER), code)
        if os.path.isdir(per):
            files = sorted(f for f in os.listdir(per) if f.endswith(".jpg"))
            if files:
                return os.path.join(per, files[0])

        return None

    def _ensure_booster_images(self, codes: list[str]):
        """Téléchargement asynchrone des images de boosters manquantes.

        Exécuté dans un thread daemon après _render : les DL arrivent en
        fond, et au prochain rafraîchissement (ou après un léger after())
        les images s'afficheront correctement.

        Pas de rate limit agressif ici : 1-2 téléchargements séquentiels
        d'images légères (~50-200 KB) — rate limité naturellement par
        requests dans booster_service.
        """
        touches = []
        for code in codes:
            try:
                if get_booster_image_path(code, auto_download=False):
                    continue   # déjà en cache local
                path = get_booster_image_path(code, auto_download=True)
                if path:
                    touches.append(code)
            except Exception as e:
                log.warning(f"_ensure_booster_images {code}: {e}")

        # Si au moins une image booster a été téléchargée, programme un
        # rafraîchissement sur le main thread pour les afficher
        if touches:
            self._safe_after(500, self._refresh_covers_only)

    def _refresh_covers_only(self):
        """Met à jour uniquement les cover_image des classeurs existants
        sans re-fetch des stats (plus rapide qu'un _load_data complet)."""
        if not self._classeurs:
            return
        for d in self._classeurs:
            code = d.get("code", "")
            db   = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
            d["cover_image"] = self._find_cover(code, db)
        # Re-render la grille (sous-ensemble filtré) avec les nouveaux covers
        for w in self._scroll.winfo_children():
            w.destroy()
        self._render_grid(self._affiches)

    def _render(self, data: list, total_cartes: int, total_poss: int):
        self._classeurs = data
        # Les StatCards reflètent TOUTE la collection (pas le filtre).
        if self._stat_classeurs:
            self._stat_classeurs.update_value(str(len(data)))
        if self._stat_total:
            self._stat_total.update_value(f"{total_cartes:,}")
        if self._stat_poss:
            self._stat_poss.update_value(f"{total_poss:,}")

        # Applique la recherche courante (vide => tout) et rend la grille.
        self._appliquer_recherche()

    def _filtrer_classeurs(self) -> list:
        """Sous-ensemble de self._classeurs correspondant à la recherche
        (sous-chaîne insensible à la casse sur le code ET le nom)."""
        q = self._search_var.get().strip().lower()
        if not q:
            return list(self._classeurs)
        return [
            d for d in self._classeurs
            if q in f"{d.get('code', '')} {d.get('nom', '')}".lower()
        ]

    def _appliquer_recherche(self):
        """Recalcule la liste affichée et reconstruit la grille."""
        self._affiches = self._filtrer_classeurs()

        for w in self._scroll.winfo_children():
            w.destroy()

        if not self._classeurs:
            self._render_empty()
        elif not self._affiches:
            self._render_aucun_resultat()
        else:
            self._grid_cols = self._compute_cols_from_width()
            self._render_grid(self._affiches)
            # Le premier calcul de colonnes peut se faire AVANT que le widget
            # ait sa largeur réelle (démarrage, fenêtre maximisée) : winfo_width
            # renvoie alors une valeur partielle → trop peu de colonnes, et il
            # fallait quitter/revenir pour corriger. On revérifie donc une fois
            # la géométrie traitée (after_idle), avec un filet de sécurité
            # légèrement différé pour le cas « maximisé au démarrage ».
            self.after_idle(self._reflow_grid)
            self._safe_after(150, self._reflow_grid)

    def _on_recherche(self, *_):
        """Anti-rebond : ne re-filtre/re-rend qu'après une courte pause de
        frappe (évite de reconstruire 500 vignettes à chaque caractère)."""
        if self._search_after:
            try:
                self.after_cancel(self._search_after)
            except Exception:
                pass
        self._search_after = self.after(250, self._appliquer_recherche)

    def _render_aucun_resultat(self):
        f = ctk.CTkFrame(self._scroll, fg_color="transparent")
        f.pack(expand=True, pady=60)
        ctk.CTkLabel(f, text="🔍", font=("Segoe UI", 48),
                     text_color=C["bg_hover"]).pack()
        ctk.CTkLabel(f, text=t("search.no_results"),
                     font=("Outfit", 12),
                     text_color=C["text3"]).pack(pady=8)

    def _compute_cols_from_width(self) -> int:
        """Nombre de colonnes tenant dans la largeur courante, sans débordement.

        Période d'une colonne = largeur d'un display + écart fixe. On soustrait
        une marge généreuse (padx + scrollbar verticale) et on plafonne, de sorte
        que la grille ne déborde jamais latéralement (donc aucun défilement
        horizontal) — quitte à laisser un peu de vide à droite.
        """
        try:
            w = self.winfo_width()
        except Exception:
            w = 1280
        if w <= 1:
            w = 1280
        period = ClasseurCard.CARD_W + self._GRID_GAP
        usable = max(0, w - self._GRID_SIDE_MARGIN)
        cols = max(1, usable // period)
        return min(int(cols), self._GRID_MAX_COLS)

    def _render_empty(self):
        f = ctk.CTkFrame(self._scroll, fg_color="transparent")
        f.pack(expand=True, pady=60)
        ctk.CTkLabel(f, text="📂", font=("Segoe UI", 64),
                     text_color=C["bg_hover"]).pack()
        ctk.CTkLabel(f, text="Aucun classeur",
                     font=("Georgia", 18, "bold"),
                     text_color=C["text2"]).pack(pady=8)
        ctk.CTkLabel(f, text="Créez votre premier classeur pour commencer.",
                     font=("Outfit", 11),
                     text_color=C["text3"]).pack()
        gold_button(f, "+ Créer un classeur",
                    command=self._ouvrir_selecteur).pack(pady=16)

    def _render_grid(self, data: list):
        cols = max(1, self._grid_cols)
        pad  = self._GRID_GAP // 2   # 8 px de chaque côté → 16 px entre 2 display

        # Réinitialise toutes les colonnes (y compris l'ancienne colonne tampon)
        # pour repartir d'une base propre quand le nombre de colonnes change.
        try:
            n_prev = self._scroll.grid_size()[0]
        except Exception:
            n_prev = cols
        for c in range(max(cols, n_prev) + 1):
            try:
                self._scroll.columnconfigure(c, weight=0, minsize=0)
            except Exception:
                pass

        for i, d in enumerate(data):
            row, col = divmod(i, cols)
            ClasseurCard(
                self._scroll, d,
                on_click=self._ouvrir_classeur,
                on_delete=self._confirmer_suppression,
                on_grille_change=self._on_grille_change,
            ).grid(row=row, column=col, padx=pad, pady=pad, sticky="nw")

        # Les colonnes de display gardent une largeur fixe (weight=0) : l'écart
        # reste donc constant quelle que soit la largeur de la fenêtre. Une
        # colonne « tampon » à droite absorbe tout l'espace restant, ce qui
        # aligne les display de gauche à droite et rejette le vide à droite.
        self._scroll.columnconfigure(cols, weight=1)

    def _on_resize(self, event=None):
        """Réagit au redimensionnement de la fenêtre, en temps réel.

        On NE filtre PAS sur event.widget : CustomTkinter redirige .bind() vers
        un canvas interne, donc event.widget n'est pas self et un ancien filtre
        « event.widget is self » bloquait tout (la grille ne se réorganisait
        qu'après changement de page). On recalcule la largeur réelle via
        winfo_width() et on débounce pour rester fluide pendant le glissement.
        """
        if not self._affiches:
            return
        # Le binding <Configure> est posé sur le toplevel (add="+") et survit à
        # la destruction de cet écran (navigation vers le classeur). On se
        # protège donc d'un appel sur un widget détruit : winfo_ismapped()
        # lèverait TclError ("bad window path name ...").
        try:
            if not self.winfo_exists() or not self.winfo_ismapped():
                return   # accueil non visible/détruit (autre écran) → rien à refaire
        except Exception:
            return
        cols = self._compute_cols_from_width()
        if cols == self._grid_cols:
            return
        self._grid_cols = cols
        if self._resize_after:
            try:
                self.after_cancel(self._resize_after)
            except Exception:
                pass
        # Petit debounce : un redimensionnement émet une rafale d'events ;
        # on ne reconstruit qu'une fois le geste calmé (fluide).
        self._resize_after = self.after(60, self._rebuild_grid)

    def _reflow_grid(self):
        """Recalcule le nombre de colonnes une fois la fenêtre stabilisée et
        re-render seulement si nécessaire. Corrige le cas où le premier rendu
        a eu lieu avant que le widget ait sa largeur réelle (démarrage /
        fenêtre maximisée). Idempotent : sans changement de largeur, ne fait
        rien — donc pas de boucle ni de rebuild superflu."""
        if not self._affiches:
            return
        cols = self._compute_cols_from_width()
        if cols != self._grid_cols:
            self._grid_cols = cols
            self._rebuild_grid()

    def _rebuild_grid(self):
        if not self._affiches:
            return
        try:
            for widget in self._scroll.winfo_children():
                widget.destroy()
            self._render_grid(self._affiches)
        except Exception:
            from module.logger_app import log
            log.exception("_rebuild_grid a échoué")

    def _ouvrir_selecteur(self):
        if self._navigate_to:
            self._navigate_to("selecteur_set")

    def _ouvrir_classeur(self, code: str):
        if self._navigate_to:
            self._navigate_to("classeur", code=code)

    def _confirmer_suppression(self, code: str):
        nom = next((d["nom"] for d in self._classeurs if d["code"] == code), code)
        DialogSuppression(self.winfo_toplevel(), code, nom, self._supprimer)

    def _supprimer(self, code: str):
        ok = supprimer_classeur(code)
        if not ok:
            try:
                from module.utilitaire.dialogs import afficher_warning
                afficher_warning(
                    f"Le classeur « {code} » n'a pas pu être entièrement supprimé "
                    f"(fichier encore utilisé ?). Fermez le classeur s'il est ouvert "
                    f"et réessayez. La re-création reste possible.",
                    "Suppression incomplète",
                )
            except Exception:
                log.warning(f"_supprimer({code}) : suppression incomplète")
        self.rafraichir()

    def _on_grille_change(self, code: str, cols: int, rows: int):
        """Callback appelé quand l'utilisateur a sauvé une grille override.

        Met à jour self._classeurs pour que l'affichage reflète la nouvelle
        grille. Pas besoin de rebuild complet — le bouton a déjà son label
        mis à jour via DialogGrilleOverride.
        """
        for d in self._classeurs:
            if d.get("code") == code:
                d["cols"] = cols
                d["rows"] = rows
                break

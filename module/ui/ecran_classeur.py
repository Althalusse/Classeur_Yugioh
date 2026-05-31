"""
ecran_classeur.py — Écran 3 : visualiseur de classeur double-page spread.

Évolutions majeures :
  1. Taille de carte ADAPTATIVE : card_w / card_h calculés selon la largeur
     de la fenêtre, respectant le ratio 59:86 (card YGO) et la config cols/rows.
  2. Première page SEULE à droite (comme un vrai classeur / album).
     Spread 0 : page 1 à droite uniquement.
     Spread s (s≥1) : page 2s (gauche) + page 2s+1 (droite).
  3. Cache PIL pour accélérer drastiquement la navigation entre pages.
  4. Bouton toggle 🌐 directement dans la navbar : bascule FR/EN et recharge
     les cartes depuis la DB avec les noms dans la nouvelle langue.
  5. Recalcul au redimensionnement avec debounce (200 ms) pour éviter la
     cascade de reconstructions pendant un drag de bordure.
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
import tkinter as tk
import customtkinter as ctk
from PIL import Image, ImageEnhance

from module.theme import C, scaled_font, get_font_scale
from module.ui.composants import (
    gold_button, secondary_button, icon_button,
    Navbar, separator, search_entry, progress_bar, styled_combobox,
    CentreActiviteButton,
)
from module.ui.dialog_carte import DialogCarte
from module.carte_posseder.affichage_carte_classeur import get_cartes_info
from module.carte_posseder.gestion_carte_posseder import update_quantite_by_rowid
from module.creation_classeur.creation_classeur_service import (
    get_set_title, get_classeur_config, save_classeur_config,
    get_n_raretes_override,
)
from module.gestion_img.gestion_image_classeur import get_image_path
from module.gestion_img.cache_images import get_or_load_pil_image
from module.gestion_img.async_image_loader import (
    request_ctk_image, prefetch_images, clear_image_cache,
)
from module.centralisation_dossier import CLASSEUR_FOLDER
from module.img_dl.file_attente_classeur import FileAttenteClasseur
from module.config_langue import load_langue, save_langue
from module.config import preferences
from module.gestion_rarete.tri_carte import filtrer_n_raretes_par_artwork
from module.statistique.panneau_rarete_classeur import PanneauRareteClasseur
from module.i18n import t
from module.logger_app import log

_file_attente = FileAttenteClasseur()

# Ratio carte YGO (référence constante, utilisée pour calcul dynamique)
CARD_RATIO_W = 59
CARD_RATIO_H = 86

# Bornes raisonnables pour card_w (px)
CARD_W_MIN = 80
CARD_W_MAX = 220

# Tailles par défaut / fallback avant toute mesure
CARD_W_DEFAULT = 140
CARD_H_DEFAULT = int(CARD_W_DEFAULT * CARD_RATIO_H / CARD_RATIO_W)

# Playset : nombre d'exemplaires d'une même rareté à partir duquel on
# considère la rareté « complète » (un playset Yu-Gi-Oh! = 3 copies).
# La quantité étant stockée par couple (numéro + rareté) dans la table cards
# du classeur, une carte du classeur représente déjà une rareté unique :
# quantite >= PLAYSET_SEUIL ⇒ playset atteint.
PLAYSET_SEUIL = 3


# ─────────────────────────────────────────────────────────────────────────────
# Chargement image avec effets (utilise le cache PIL)
# ─────────────────────────────────────────────────────────────────────────────

_CTK_IMAGE_CACHE: dict = {}
_CTK_IMAGE_CACHE_MAX = 400


def _clear_ctk_image_cache():
    """Vide le cache des CTkImage (à appeler si la taille des cartes change)."""
    _CTK_IMAGE_CACHE.clear()
    clear_image_cache()   # purge aussi le cache du moteur asynchrone


def _load_card_image(path: str, possessed: bool, card_w: int, card_h: int,
                     hover: bool = False):
    """Charge une image carte avec filtres selon l'état (possessed/hover).

    Deux niveaux de cache :
      - PIL brut (get_or_load_pil_image) : Image.open() + convert une seule
        fois par fichier.
      - CTkImage finale (ce cache-ci) : le resize LANCZOS + les filtres
        couleur/luminosité sont coûteux et étaient refaits à CHAQUE rendu
        de page (2× par carte × N cartes → flash/lag au changement de page).
        On mémorise donc le CTkImage résultant par (path, possessed, taille,
        hover). Gain direct sur la fluidité de navigation.
    """
    key = (path, bool(possessed), int(card_w), int(card_h), bool(hover))
    cached = _CTK_IMAGE_CACHE.get(key)
    if cached is not None:
        return cached
    try:
        pil = get_or_load_pil_image(path)
        if pil is None:
            pil_placeholder = Image.new("RGB", (card_w, card_h), "#1A1A2E")
            img = ctk.CTkImage(pil_placeholder, size=(card_w, card_h))
        else:
            pil = pil.resize((card_w, card_h), Image.LANCZOS)
            if not possessed:
                if hover:
                    pil = ImageEnhance.Color(pil).enhance(0.70)
                    pil = ImageEnhance.Brightness(pil).enhance(0.70)
                else:
                    pil = ImageEnhance.Color(pil).enhance(0.20)
                    pil = ImageEnhance.Brightness(pil).enhance(0.35)
            img = ctk.CTkImage(pil, size=(card_w, card_h))
    except Exception:
        pil = Image.new("RGB", (card_w, card_h), "#1A1A2E")
        img = ctk.CTkImage(pil, size=(card_w, card_h))

    if len(_CTK_IMAGE_CACHE) < _CTK_IMAGE_CACHE_MAX:
        _CTK_IMAGE_CACHE[key] = img
    return img


# ─────────────────────────────────────────────────────────────────────────────
# Widget carte individuelle
# ─────────────────────────────────────────────────────────────────────────────

class CarteWidget(ctk.CTkFrame):
    """
    Une carte dans la grille du classeur.
    - Image ratio 59:86 (taille variable via card_w/card_h)
    - Overlay hover : nom, code, rareté
    - Badge quantité (coin haut-droit si qty > 0)
    - Toggle ✓ (toujours visible si possédée, hover si non)
    """

    def __init__(self, parent, carte: dict, classeur_code: str,
                 card_w: int, card_h: int,
                 on_update=None, open_dialog=None, open_anomalies=None):
        super().__init__(
            parent,
            fg_color=C["bg3"],
            corner_radius=6,
            width=card_w,
            height=card_h,
            cursor="hand2",
        )
        self.pack_propagate(False)
        self.configure(width=card_w, height=card_h)

        self._carte   = carte
        self._code    = classeur_code
        self._db      = os.path.join(CLASSEUR_FOLDER, classeur_code,
                                      f"{classeur_code}.db")
        self._card_w  = card_w
        self._card_h  = card_h
        self._on_update     = on_update
        self._open_dialog   = open_dialog
        self._open_anomalies = open_anomalies  # callback(set_code) — clic droit
        self._img_norm    = None
        self._img_hover   = None
        self._img_ref     = None
        self._hovering    = False     # survol en cours (pour callbacks async)
        self._req_token   = 0         # invalide les callbacks d'images périmées
        self._has_real_image = False  # True dès qu'une vraie image (≠ placeholder) est dispo

        self._build()
        self._bind_events()

    def _build(self):
        carte = self._carte
        possessed = (carte.get("quantite", 0) or 0) > 0

        # Image principale : créée AVANT le chargement, avec un placeholder.
        # Le redimensionnement LANCZOS + les filtres sont déportés hors du
        # thread UI ; les images réelles arrivent via _request_card_images.
        self._img_lbl = ctk.CTkLabel(
            self, text="",
            corner_radius=4,
        )
        self._img_lbl.place(x=0, y=0, relwidth=1, relheight=1)

        self._request_card_images(possessed)

        qty = carte.get("quantite", 0) or 0
        if qty > 0:
            # Badge quantité : coin haut-droit
            self._qty_badge = ctk.CTkLabel(
                self, text=str(qty),
                fg_color=C["bg"],
                text_color=C["gold"],
                font=("JetBrains Mono", 10, "bold"),
                corner_radius=12,
                width=22, height=22,
            )
            self._qty_badge.place(relx=1.0, rely=0.0, anchor="ne", x=-3, y=3)
        else:
            self._qty_badge = None

        # Toggle ✓ (coin haut-gauche)
        self._toggle_btn = ctk.CTkButton(
            self, text="✓" if possessed else "+",
            width=22, height=22,
            fg_color=C["gold"] if possessed else C["bg"],
            hover_color=C["gold_hover"],
            text_color="#000" if possessed else C["text"],
            font=("Segoe UI", 10, "bold"),
            corner_radius=3,
            border_width=0,
            command=self._toggle_possessed,
        )
        self._toggle_btn.place(x=3, y=3)
        if not possessed:
            self._toggle_btn.configure(fg_color="transparent")

        # Badge Overframe (coin bas-droit, TOUJOURS visible) — distingue un
        # print "art étendu" de la version cadre normal, qui partagent la même
        # image. Couleur violette dédiée (C["overframe"]), distincte du doré.
        if (carte.get("extended_art", 0) or 0):
            self._of_badge = ctk.CTkLabel(
                self, text="OVERFRAME",
                fg_color=C["overframe"], text_color=C["overframe_text"],
                font=("Outfit", 8, "bold"),
                corner_radius=3, padx=5, pady=1,
            )
            self._of_badge.place(relx=1.0, rely=1.0, anchor="se", x=-4, y=-4)
        else:
            self._of_badge = None

        # Badge Playset (coin bas-GAUCHE) — signale qu'on possède au moins un
        # playset (PLAYSET_SEUIL = 3 exemplaires) de cette rareté. Même style
        # que le badge Overframe, mais à gauche de l'encadré de la carte.
        # Affiché quoi qu'il arrive dès que la quantité atteint le seuil.
        if qty >= PLAYSET_SEUIL:
            self._playset_badge = ctk.CTkLabel(
                self, text="PLAYSET",
                fg_color=C["playset"], text_color=C["playset_text"],
                font=("Outfit", 8, "bold"),
                corner_radius=3, padx=5, pady=1,
            )
            self._playset_badge.place(relx=0.0, rely=1.0, anchor="sw", x=4, y=-4)
        else:
            self._playset_badge = None

        # Overlay infos (survol) — barre sombre en bas de la carte.
        # Le nom passe sur 2 lignes (wrap) et la rareté est affichée EN ENTIER
        # (plus de troncature arbitraire). Hauteur dimensionnée pour contenir,
        # au pire, un nom sur 2 lignes + une rareté sur 2 lignes, à l'échelle
        # de police courante (max 1.50x).
        nom    = (carte.get("name", "") or "")
        sc     = (carte.get("set_code", "") or "").strip()
        rarete = (carte.get("rarity", "") or "").strip()
        is_of  = bool(carte.get("extended_art", 0) or 0)
        # Suffixe Overframe pour la ligne d'info (en plus du badge visuel).
        rarete_aff = (rarete + " · Overframe") if (rarete and is_of) else \
                     ("Overframe" if is_of else rarete)

        # Largeur de wrap = largeur de carte moins les marges latérales.
        wrap_w = max(40, self._card_w - 8)
        # Garde-fou : on borne le nom à ~2 lignes de caractères (ellipsis
        # au-delà) pour qu'il ne déborde jamais sur une 3ᵉ ligne.
        # ~card_w/8 caractères par ligne → ×2 pour deux lignes.
        max_chars = max(20, int(self._card_w / 4))
        nom_aff = nom if len(nom) <= max_chars else nom[:max_chars].rstrip() + "…"

        # Hauteur de l'overlay = 2 lignes nom (12pt) + 2 lignes rareté (11pt)
        # + marges, mises à l'échelle de la police.
        fs = get_font_scale()
        _line = lambda pt: round(pt * fs * 1.7)   # hauteur approx. d'une ligne
        overlay_h = 6 + 2 * _line(12) + 2 * _line(11) + 6

        self._overlay = ctk.CTkFrame(
            self, fg_color=C["bg"],
            corner_radius=0, height=overlay_h,
        )
        # Nom (2 lignes max, wrap) + "code · rareté" (rareté complète, wrap)
        # IMPORTANT : ces deux labels passent par scaled_font(...) pour que le
        # facteur d'échelle global (Options → "Taille de police") s'applique.
        ctk.CTkLabel(
            self._overlay,
            text=nom_aff,
            font=scaled_font("Outfit", 12, "bold"),
            text_color=C["text"],
            anchor="w", justify="left",
            wraplength=wrap_w,
        ).pack(anchor="w", padx=4, pady=(4, 0), fill="x")
        ctk.CTkLabel(
            self._overlay,
            text=(f"{sc} · {rarete_aff}" if rarete_aff else sc),
            font=scaled_font("JetBrains Mono", 11),
            # text2 (#A0A4B8) : meilleur contraste que text3 contre le fond
            # sombre. set_code et rareté sont des infos structurelles.
            text_color=C["text2"],
            anchor="w", justify="left",
            wraplength=wrap_w,
        ).pack(anchor="w", padx=4, fill="x")

        self._overlay.place_forget()

    # ── Chargement images (asynchrone, hors thread UI) ──────────────────────
    def _request_card_images(self, possessed: bool):
        """Charge norm + hover en arrière-plan et affiche l'image adaptée à
        l'état de survol courant.

        Règles d'affichage :
          - L'image visible suit toujours `self._hovering` (norm hors survol,
            hover pendant le survol) — c'est ce qui manquait : auparavant un
            toggle effectué EN SURVOL (cas normal du clic sur le ✓) ne
            rafraîchissait jamais le label, l'image restant assombrie jusqu'au
            survol suivant.
          - Lors d'un simple rafraîchissement (toggle) d'une carte qui affiche
            déjà une vraie image, on conserve l'image courante tant que la
            nouvelle n'est pas prête → pas de flash de placeholder.

        Les callbacks ne s'appliquent que si le widget existe encore ET si le
        jeton de requête n'a pas changé.
        """
        self._req_token += 1
        token = self._req_token
        img_path = get_image_path(
            self._code, self._carte.get("image_filename") or ""
        )
        w, h = self._card_w, self._card_h

        # Image normale (état hors survol)
        norm, norm_ready = request_ctk_image(
            img_path, possessed, w, h, hover=False,
            owner=self,
            on_ready=lambda im, t=token: self._on_norm_ready(im, t),
        )
        if norm_ready:
            self._img_norm = norm
            self._img_ref  = norm
            if possessed:
                self._img_hover = norm
            self._has_real_image = True
        elif not self._has_real_image:
            # Premier affichage : on montre le placeholder en attendant.
            self._img_norm = norm
            self._img_ref  = norm
            if possessed:
                self._img_hover = norm
        # sinon : refresh sans image prête → on garde l'ancienne vraie image
        # (ni _img_norm ni _img_hover ne sont écrasés par le placeholder).

        # Image survol : identique à norm si possédée (aucun filtre), sinon
        # variante assombrie — préchargée pour un survol instantané.
        if possessed:
            if norm_ready or not self._has_real_image:
                self._img_hover = norm
        else:
            hov, hov_ready = request_ctk_image(
                img_path, possessed, w, h, hover=True,
                owner=self,
                on_ready=lambda im, t=token: self._on_hover_ready(im, t),
            )
            if hov_ready or not self._has_real_image:
                self._img_hover = hov

        self._apply_current_image()

    def _apply_current_image(self):
        """Affiche l'image correspondant à l'état de survol courant."""
        img = self._img_hover if self._hovering else self._img_norm
        if img is not None:
            try:
                self._img_lbl.configure(image=img)
            except Exception:
                pass

    def _on_norm_ready(self, image, token: int):
        if token != self._req_token:
            return
        self._img_norm = image
        self._img_ref  = image
        if (self._carte.get("quantite", 0) or 0) > 0:
            # Possédée : hover == norm, on garde les deux synchronisés.
            self._img_hover = image
        self._has_real_image = True
        self._apply_current_image()

    def _on_hover_ready(self, image, token: int):
        if token != self._req_token:
            return
        self._img_hover = image
        self._apply_current_image()

    def _bind_events(self):
        # Helper : la souris est-elle encore dans la carte (ou un descendant) ?
        # Sans cette vérification, l'overlay (placé en bas de la carte)
        # déclenche un <Leave> sur la frame parent dès que la souris arrive
        # dessus → l'overlay disparaît → la souris est de nouveau sur la
        # carte seule → <Enter> → boucle infinie de clignotement.
        def _souris_dans_carte() -> bool:
            try:
                x, y = self.winfo_pointerxy()
                w_under = self.winfo_containing(x, y)
            except Exception:
                return False
            # Remonte la hiérarchie : si on trouve `self` en ancêtre, on est
            # toujours dans la carte (ou un de ses widgets enfants).
            while w_under is not None:
                if w_under is self:
                    return True
                try:
                    w_under = w_under.master
                except Exception:
                    break
            return False

        def enter(e):
            self._hovering = True
            self._apply_current_image()
            self._overlay.place(relx=0, rely=1.0, anchor="sw",
                                relwidth=1)
            # Survol : rendre le bouton toggle visible. Possédée → garder l'or
            # (sinon le ✓ noir devient invisible sur fond sombre) ; non
            # possédée → fond sombre pour faire apparaître le "+".
            possessed = (self._carte.get("quantite", 0) or 0) > 0
            self._toggle_btn.configure(
                fg_color=C["gold"] if possessed else C["bg"]
            )

        def leave(e):
            # Si la souris est encore quelque part sur la carte (overlay,
            # bouton toggle, image, label enfant…), on ne fait rien.
            # Le vrai <Leave> sera déclenché quand la souris quittera
            # vraiment la zone de la carte.
            if _souris_dans_carte():
                return
            self._hovering = False
            self._apply_current_image()
            self._overlay.place_forget()
            # Restaure la couleur du toggle selon l'état : or si possédée
            # (✓ visible), transparent sinon (+ masqué hors survol).
            possessed = (self._carte.get("quantite", 0) or 0) > 0
            self._toggle_btn.configure(
                fg_color=C["gold"] if possessed else "transparent"
            )

        # Bind sur tous les widgets potentiellement survolés. L'overlay et
        # ses labels enfants ont besoin du même traitement, sinon Tk émet
        # un <Leave> sur la carte parent dès qu'on entre sur eux.
        widgets_a_binder = [self, self._img_lbl, self._overlay]
        # Récupération des descendants de l'overlay (labels nom/code)
        try:
            widgets_a_binder.extend(self._overlay.winfo_children())
        except Exception:
            pass

        for w in widgets_a_binder:
            w.bind("<Enter>", enter)
            w.bind("<Leave>", leave)

        # Clic gauche = ouvrir dialog. On le bind seulement sur self et
        # l'image — pas sur l'overlay, qui pourra héberger ses propres
        # actions (clic droit pour modifier l'artwork, cf. _show_context_menu).
        for w in (self, self._img_lbl):
            w.bind("<Button-1>", lambda e: self._click())

        # Clic droit = menu contextuel (modifier l'artwork, etc.)
        # Bindé sur tous les widgets visibles de la carte pour que le clic
        # droit fonctionne où qu'on tape.
        for w in widgets_a_binder:
            w.bind("<Button-3>", self._show_context_menu)

    def _click(self):
        if self._open_dialog:
            self._open_dialog(self._carte)

    def _show_context_menu(self, event):
        """Affiche un menu contextuel au clic droit sur la carte.

        Pour l'instant une seule entrée : "Modifier l'artwork" qui ouvre
        le dialog d'anomalies pré-filtré sur cette carte. Si la carte n'a
        pas d'artwork alternatif connu, le dialog affichera un message
        explicite à l'utilisateur.

        On utilise tk.Menu (et non CTkMenu qui n'existe pas en CTk
        standard) — c'est le pattern recommandé pour les menus
        contextuels avec CustomTkinter.
        """
        import tkinter as tk
        menu = tk.Menu(
            self, tearoff=0,
            bg=C["bg2"],
            fg=C["text"],
            activebackground=C["gold"],
            activeforeground="#000000",
            bd=0,
            relief="flat",
            font=("Outfit", 10),
        )
        # Header non cliquable : nom de la carte pour qu'on sache sur
        # laquelle on agit (utile sur des grilles denses).
        nom = (self._carte.get("name") or "").strip()
        sc  = (self._carte.get("set_code") or "").strip()
        en_tete = nom if not sc else f"{nom}  ·  {sc}"
        if len(en_tete) > 50:
            en_tete = en_tete[:47] + "…"
        menu.add_command(label=en_tete, state="disabled")
        menu.add_separator()
        menu.add_command(
            label="🎨  Modifier l'artwork…",
            command=self._on_modifier_artwork,
        )
        menu.add_command(
            label="➕  Ajouter une carte au classeur…",
            command=self._on_ajouter_carte,
        )
        # Suppression : réservée aux cartes ajoutées manuellement (is_custom).
        if self._carte.get("is_custom"):
            menu.add_separator()
            menu.add_command(
                label="🗑  Supprimer cette carte (ajout manuel)…",
                command=self._on_supprimer_carte,
            )

        try:
            # tk_popup pose le menu à l'écran à la position du clic.
            # grab_release évite que le menu reste capturé en cas de
            # clic ailleurs.
            menu.tk_popup(event.x_root, event.y_root)
        finally:
            menu.grab_release()

    def _on_modifier_artwork(self):
        """Callback du menu contextuel — délègue à l'écran parent qui
        sait comment ouvrir le dialog d'anomalies."""
        if self._open_anomalies:
            self._open_anomalies(self._carte)

    def _on_ajouter_carte(self):
        """Callback menu contextuel — ouvre le dialogue d'ajout manuel.

        Autonome : la carte connaît son classeur (self._code) et le toplevel
        parent. Pour le rafraîchissement, on remonte le widget-tree jusqu'à
        l'EcranClasseur afin d'appeler `refresh_after_correction` (re-fetch
        complet + relance du téléchargement d'images), comme le fait le flux
        « Modifier l'artwork ». Fallback sur self._on_update sinon.
        """
        from module.carte_custom.ajout_carte_ui import ouvrir_dialog_ajout_carte

        # Remonte jusqu'à l'écran classeur (qui porte refresh_after_correction)
        ecran = self.master
        while ecran is not None and not hasattr(ecran, "refresh_after_correction"):
            ecran = getattr(ecran, "master", None)

        def _refresh():
            if ecran is not None:
                ecran.refresh_after_correction()
            elif self._on_update:
                self._on_update()

        try:
            ouvrir_dialog_ajout_carte(
                self.winfo_toplevel(),
                classeur_code=self._code,
                set_prefix=self._code,
                on_done=_refresh,
            )
        except Exception as e:
            log.error(f"_on_ajouter_carte : {e}")

    def _on_supprimer_carte(self):
        """Supprime une carte ajoutée manuellement, après confirmation.

        Réservé aux cartes is_custom (l'entrée de menu n'apparaît que pour
        elles). Remonte à l'EcranClasseur pour le refresh complet.
        """
        from tkinter import messagebox
        from module.carte_custom.ajout_carte_service import supprimer_carte_custom

        rowid = self._carte.get("rowid")
        nom   = (self._carte.get("name") or "cette carte").strip()
        sc    = (self._carte.get("set_code") or "").strip()
        if rowid is None:
            return

        if not messagebox.askyesno(
            "Supprimer la carte",
            f"Supprimer définitivement « {nom} »"
            f"{f' ({sc})' if sc else ''} du classeur ?\n\n"
            f"Cette carte a été ajoutée manuellement. "
            f"Cette action est irréversible.",
            parent=self.winfo_toplevel(),
        ):
            return

        ok, msg = supprimer_carte_custom(self._code, rowid)
        if not ok:
            messagebox.showwarning("Suppression", msg,
                                   parent=self.winfo_toplevel())
            return

        # Refresh complet (remonte à l'EcranClasseur)
        ecran = self.master
        while ecran is not None and not hasattr(ecran, "refresh_after_correction"):
            ecran = getattr(ecran, "master", None)
        if ecran is not None:
            ecran.refresh_after_correction()
        elif self._on_update:
            self._on_update()

    def _toggle_possessed(self):
        qty     = self._carte.get("quantite", 0) or 0
        new_qty = 0 if qty > 0 else 1
        rowid   = self._carte.get("rowid")
        if rowid:
            update_quantite_by_rowid(self._db, rowid, new_qty)
        self._carte["quantite"]  = new_qty
        self._carte["possessed"] = 1 if new_qty > 0 else 0
        self._refresh_visuals()
        if self._on_update:
            self._on_update()

    def _refresh_visuals(self):
        qty = self._carte.get("quantite", 0) or 0
        possessed = qty > 0
        # Recharge norm + hover hors thread UI selon le nouvel état possédé.
        # Le cache rend le swap instantané si l'image a déjà été calculée.
        self._request_card_images(possessed)

        if self._qty_badge:
            self._qty_badge.place_forget()
            self._qty_badge = None
        if qty > 0:
            self._qty_badge = ctk.CTkLabel(
                self, text=str(qty),
                fg_color=C["bg"], text_color=C["gold"],
                font=("JetBrains Mono", 10, "bold"),
                corner_radius=12, width=22, height=22,
            )
            self._qty_badge.place(relx=1.0, rely=0.0, anchor="ne", x=-3, y=3)

        # Badge Playset : (re)construit selon la quantité courante. Même règle
        # qu'au _build (qty >= PLAYSET_SEUIL), pour que le badge apparaisse ou
        # disparaisse aussi sur un simple rafraîchissement local (toggle).
        if getattr(self, "_playset_badge", None):
            self._playset_badge.place_forget()
            self._playset_badge = None
        if qty >= PLAYSET_SEUIL:
            self._playset_badge = ctk.CTkLabel(
                self, text="PLAYSET",
                fg_color=C["playset"], text_color=C["playset_text"],
                font=("Outfit", 8, "bold"),
                corner_radius=3, padx=5, pady=1,
            )
            self._playset_badge.place(relx=0.0, rely=1.0, anchor="sw", x=4, y=-4)

        self._toggle_btn.configure(
            text="✓" if possessed else "+",
            fg_color=C["gold"] if possessed else "transparent",
            text_color="#000" if possessed else C["text"],
        )


# ─────────────────────────────────────────────────────────────────────────────
# Page du classeur (grille N×N)
# ─────────────────────────────────────────────────────────────────────────────

class BinderPage(ctk.CTkFrame):
    """Une page du classeur (grille NxN de CarteWidget + slots vides)."""

    GAP = 8

    def __init__(self, parent, cartes: list, cols: int, rows: int,
                 classeur_code: str, card_w: int, card_h: int,
                 on_update=None, open_dialog=None, open_anomalies=None):
        super().__init__(
            parent,
            fg_color=C["bg3"],
            corner_radius=12,
            border_color=C["border"],
            border_width=1,
        )
        max_slots = cols * rows
        for i in range(max_slots):
            r, c = divmod(i, cols)
            if i < len(cartes):
                CarteWidget(
                    self, cartes[i], classeur_code,
                    card_w, card_h,
                    on_update=on_update, open_dialog=open_dialog,
                    open_anomalies=open_anomalies,
                ).grid(row=r, column=c, padx=self.GAP, pady=self.GAP)
            else:
                # Slot vide (même taille que les cartes)
                slot = ctk.CTkFrame(
                    self, width=card_w, height=card_h,
                    fg_color="transparent",
                    border_color=C["border"],
                    border_width=1,
                    corner_radius=4,
                )
                slot.grid(row=r, column=c, padx=self.GAP, pady=self.GAP)
                slot.pack_propagate(False)


# ─────────────────────────────────────────────────────────────────────────────
# Écran Classeur
# ─────────────────────────────────────────────────────────────────────────────

class EcranClasseur(ctk.CTkFrame):
    """Visualiseur double-page spread pour un classeur."""

    # Constantes pour le calcul de taille adaptative
    _OUTER_PAD     = 40   # padx/pady sur _spread_frame
    _SCROLLBAR_W   = 20   # réserve scrollbar verticale
    _SPREAD_GAP    = 24   # gap visuel entre les 2 pages du spread
    _PAGE_INNER_PAD = 16  # padding interne BinderPage (cornerradius + borduret)
    _RESIZE_DEBOUNCE_MS = 200

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._cartes: list[dict] = []
        self._filtered: list[dict] = []
        self._classeur_code  = ""
        self._page  = 0       # index spread (0 = première page à droite seule)
        self._cols  = 3
        self._rows  = 3
        self._card_w = CARD_W_DEFAULT
        self._card_h = CARD_H_DEFAULT
        self._search_var     = ctk.StringVar()
        self._rarity_var     = ctk.StringVar(value="Toutes")
        self._possessed_var  = ctk.StringVar(value="Toutes")
        self._search_after   = None
        self._resize_after   = None

        self._build()
        # Bind resize après construction complète
        self.bind("<Configure>", self._on_configure)

    # ── Construction UI ────────────────────────────────────────────────────

    def _build(self):
        # Navbar dynamique (reconstruite à chaque load)
        self._navbar_frame = ctk.CTkFrame(self, fg_color=C["bg2"],
                                           corner_radius=0, height=56)
        self._navbar_frame.pack(fill="x")
        self._navbar_frame.pack_propagate(False)
        self._build_navbar_content()

        # Barre progression DL
        self._dl_bar_frame = ctk.CTkFrame(
            self, fg_color=C["bg2"],
            corner_radius=0, height=40,
        )
        self._dl_lbl = ctk.CTkLabel(
            self._dl_bar_frame, text="",
            font=("Outfit", 10), text_color=C["text3"],
        )
        self._dl_lbl.pack(side="left", padx=16)
        self._dl_pbar = progress_bar(self._dl_bar_frame, width=300)
        self._dl_pbar.pack(side="left", padx=8)
        # Caché par défaut
        self._dl_bar_frame.pack_forget()

        # Barre recherche / filtres
        self._build_search_bar()

        # Panneau dépliable « Statistiques par rareté » (calcul en mémoire,
        # alimenté dans _on_cartes_loaded). Replié par défaut.
        self._panneau_rarete = PanneauRareteClasseur(self)
        self._panneau_rarete.pack(fill="x")

        separator(self).pack(fill="x", padx=0, pady=(0, 0))

        # Zone principale scrollable
        self._main_scroll = ctk.CTkScrollableFrame(
            self, fg_color=C["bg"], corner_radius=0,
        )
        self._main_scroll.pack(fill="both", expand=True)

        # Conteneur HÔTE stable du spread (ne bouge jamais) — sert d'ancre
        # pour le double-buffering : les frames de spread y sont créés puis
        # swappés sans détruire l'hôte (donc sans repli/flash du layout).
        self._spread_host = ctk.CTkFrame(
            self._main_scroll, fg_color="transparent"
        )
        self._spread_host.pack(pady=20, padx=20, anchor="center")

        # Buffer courant (vide au départ ; rempli par _afficher_spread).
        self._spread_frame = None

        # Navigation pages
        self._build_nav_bar()

    def _build_navbar_content(self):
        for w in self._navbar_frame.winfo_children():
            w.destroy()

        # Gauche
        left = ctk.CTkFrame(self._navbar_frame, fg_color="transparent")
        left.pack(side="left", padx=12, pady=8)
        ctk.CTkButton(
            left, text="←", command=self._retour,
            width=36, height=36,
            fg_color="transparent", hover_color=C["bg_hover"],
            text_color=C["text"], font=("Segoe UI", 16), corner_radius=4,
        ).pack(side="left", padx=(0, 8))

        title = self._classeur_code or "Classeur"
        use_fr = load_langue() == "FR"
        nom    = get_set_title(self._classeur_code, use_fr=use_fr) if self._classeur_code else ""
        ctk.CTkLabel(
            left,
            text=(nom[:35] + "…" if len(nom) > 35 else nom) or title,
            font=("Playfair Display", 13, "bold"),
            text_color=C["text"],
        ).pack(side="left")

        if self._classeur_code:
            ctk.CTkLabel(left, text=f"  {self._classeur_code}",
                         font=("JetBrains Mono", 10),
                         text_color=C["gold"]).pack(side="left")

        if self._cartes:
            total = len(set((c["set_code"], c["rarity"]) for c in self._cartes))
            poss  = sum(1 for c in self._cartes if (c.get("quantite") or 0) > 0)
            pct   = poss / len(self._cartes) * 100 if self._cartes else 0
            ctk.CTkLabel(
                left,
                text=f"  {poss}/{len(self._cartes)} ({pct:.0f}%)",
                font=("Outfit", 9),
                text_color=C["text3"],
            ).pack(side="left")

        # Droite
        right = ctk.CTkFrame(self._navbar_frame, fg_color="transparent")
        right.pack(side="right", padx=12, pady=8)
        # 📥 Activité — visible sur tous les écrans pour suivre les
        # téléchargements en arrière-plan déclenchés à l'ouverture du classeur.
        CentreActiviteButton(right).pack(side="left", padx=4)
        # Toggle langue (🌐 FR/EN)
        self._lang_btn = icon_button(
            right, self._lang_label(), command=self._toggle_langue,
        )
        self._lang_btn.pack(side="left", padx=4)
        icon_button(right, "📷 Anomalies",       command=self._ouvrir_anomalies).pack(side="left", padx=4)
        # Lot 3 import/export (avr. 2026) : un seul bouton qui ouvre la
        # fenêtre Scanflip globale, pré-configurée sur ce classeur.
        # Remplace les anciens boutons "⬆ Import" et "⬇ CSV" séparés.
        icon_button(right, "📋 Import/Export",   command=self._ouvrir_import_export).pack(side="left", padx=4)

        ctk.CTkFrame(self._navbar_frame, height=1,
                     fg_color=C["border"], corner_radius=0).pack(side="bottom", fill="x")

    def _lang_label(self) -> str:
        """Texte du bouton toggle langue (affiche la langue VERS laquelle basculer)."""
        return "🌐 EN" if load_langue() == "FR" else "🌐 FR"

    def _toggle_langue(self):
        """Bascule FR/EN et recharge les cartes pour afficher les noms traduits."""
        current = load_langue()
        save_langue("FR" if current == "EN" else "EN")
        # Recharge complet : get_cartes_info() relit la DB avec la bonne colonne
        if self._classeur_code:
            self.charger(self._classeur_code)

    def _build_search_bar(self):
        bar = ctk.CTkFrame(self, fg_color="transparent")
        bar.pack(fill="x", padx=16, pady=10)
        bar.columnconfigure(0, weight=1)

        entry = search_entry(
            bar, textvariable=self._search_var,
            placeholder="🔍  Rechercher une carte…",
        )
        entry.grid(row=0, column=0, sticky="ew", padx=(0, 8))
        self._search_var.trace_add("write", self._on_search_change)

        # Filtres (ligne suivante, masquée si filtre fermé)
        self._filter_row = ctk.CTkFrame(self, fg_color="transparent")
        self._filter_row.pack(fill="x", padx=16, pady=(0, 6))

        # Dropdown rareté
        self._rarity_cb = styled_combobox(
            self._filter_row,
            values=["Toutes"],
            variable=self._rarity_var,
            width=180,
        )
        self._rarity_cb.pack(side="left", padx=(0, 8))
        self._rarity_cb.configure(command=lambda v: self._appliquer_filtres())

        # Dropdown possédées
        possessed_cb = styled_combobox(
            self._filter_row,
            values=["Toutes", "Possédées", "Non possédées"],
            variable=self._possessed_var,
            width=160,
        )
        possessed_cb.pack(side="left")
        possessed_cb.configure(command=lambda v: self._appliquer_filtres())

    def _build_nav_bar(self):
        self._nav_frame = ctk.CTkFrame(
            self._main_scroll, fg_color="transparent"
        )
        self._nav_frame.pack(pady=16)

        self._btn_prev = secondary_button(
            self._nav_frame, "◀ Précédent",
            command=self._page_precedente, width=130,
        )
        self._btn_prev.pack(side="left", padx=8)

        # Entry de navigation directe (remplace le label statique)
        self._page_entry = ctk.CTkEntry(
            self._nav_frame,
            width=52, height=28,
            font=("JetBrains Mono", 10),
            fg_color=C["bg3"],
            border_color=C["border"],
            border_width=1,
            text_color=C["text"],
            justify="center",
        )
        self._page_entry.pack(side="left", padx=(8, 2))
        self._page_entry.bind("<Return>",   self._aller_page_depuis_entry)
        self._page_entry.bind("<KP_Enter>", self._aller_page_depuis_entry)
        self._page_entry.bind("<FocusOut>", self._aller_page_depuis_entry)

        ctk.CTkLabel(
            self._nav_frame, text=" / ",
            font=("JetBrains Mono", 10),
            text_color=C["text3"],
        ).pack(side="left")

        self._page_total_lbl = ctk.CTkLabel(
            self._nav_frame, text="—",
            font=("JetBrains Mono", 10),
            text_color=C["text3"],
        )
        self._page_total_lbl.pack(side="left", padx=(0, 8))

        self._btn_next = secondary_button(
            self._nav_frame, "Suivant ▶",
            command=self._page_suivante, width=130,
        )
        self._btn_next.pack(side="left", padx=8)

    # ── Chargement ─────────────────────────────────────────────────────────

    def charger(self, code: str):
        self._classeur_code = code
        self._page = 0
        self._cartes   = []
        self._filtered = []
        self._build_navbar_content()
        threading.Thread(target=self._load_cartes, daemon=True).start()
        self._poll_dl_progress()

    def _safe_after(self, *args, **kwargs):
        """Wrapper défensif autour de self.after() — voir EcranAccueil.
        Évite les crashes 'main thread is not in main loop' quand le widget
        n'est plus attaché à un mainloop actif (ex : écran détruit,
        interaction avec show_init_window au premier lancement, app en
        train de se fermer)."""
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
            log.warning(f"_safe_after : {e}")

    def _load_cartes(self):
        try:
            cartes = get_cartes_info(self._classeur_code)
            self._safe_after(0, self._on_cartes_loaded, cartes)

            # Déclenchement automatique du refresh des images.
            #
            # À chaque ouverture d'un classeur, on pousse une tâche dans la
            # file d'attente. Le worker :
            #   - Détecte que le classeur existe → skip la création.
            #   - Scanne les images locales manquantes OU identifiées comme
            #     placeholders notfound.jpg (cf. est_notfound_placeholder).
            #   - Retente chaque image avec fallback multi-sources
            #     (YGOPRODeck ↔ Yugipedia selon la source active).
            #   - Si les deux sources échouent, laisse un placeholder en place ;
            #     la carte sera retentée à la prochaine ouverture du classeur.
            #
            # Si aucune image n'est à (re)télécharger, la tâche se termine
            # quasi instantanément avec le message « Images déjà présentes ✓ »
            # et n'affiche rien de gênant dans l'UI.
            #
            # Le _poll_dl_progress déjà lancé par charger() détectera la tâche
            # et gère le re-render à chaque image qui tombe sur disque.
            try:
                _file_attente.ajouter(self._classeur_code)
            except Exception as e:
                # L'échec de l'ajout ne doit jamais bloquer l'affichage
                # du classeur — c'est un bonus, pas une nécessité.
                log.warning(f"EcranClasseur._load_cartes: "
                      f"ajout refresh file d'attente échoué : {e}")
        except Exception as e:
            log.warning(f"EcranClasseur._load_cartes: {e}")
            self._safe_after(0, self._on_cartes_loaded, [])

    def _on_cartes_loaded(self, cartes: list):
        self._cartes = cartes

        # Alimente le panneau « Statistiques par rareté » (mémoire)
        try:
            self._panneau_rarete.set_cartes(self._cartes)
        except Exception as e:
            log.warning(f"EcranClasseur: maj panneau rareté: {e}")

        # Charger config grille
        db = os.path.join(CLASSEUR_FOLDER, self._classeur_code,
                          f"{self._classeur_code}.db")
        self._cols, self._rows = get_classeur_config(db)

        # Raretés pour filtre
        raretes = sorted({c.get("rarity", "") for c in cartes if c.get("rarity")})
        self._rarity_cb.configure(values=["Toutes"] + raretes)
        self._rarity_var.set("Toutes")
        self._possessed_var.set("Toutes")
        self._search_var.set("")

        # Force le layout à se stabiliser AVANT de mesurer la largeur.
        # Sans ça, au tout premier load (après démarrage + navigation), le
        # frame n'est pas encore complètement mappé et winfo_width() retourne
        # une valeur partielle → fallback 1280px → cartes trop petites.
        try:
            self.update_idletasks()
        except Exception:
            pass
        self._card_w, self._card_h = self._compute_card_size()

        self._appliquer_filtres()
        self._build_navbar_content()

        # Filet de sécurité : au tout premier chargement d'un classeur après
        # démarrage de l'app, le layout peut prendre un tick supplémentaire
        # pour se stabiliser. On re-mesure 150 ms plus tard et on re-rend si
        # la taille de carte calculée a changé.
        self.after(150, self._recompute_if_needed)

    def _recompute_if_needed(self):
        """Recalcule la taille de carte et re-render si elle a changé."""
        if not self._cartes:
            return
        try:
            self.update_idletasks()
        except Exception:
            pass
        new_w, new_h = self._compute_card_size()
        if new_w != self._card_w or new_h != self._card_h:
            self._card_w, self._card_h = new_w, new_h
            # La taille de carte a changé : les CTkImage mises en cache (clé
            # incluant l'ancienne taille) deviennent obsolètes. On purge pour
            # éviter d'accumuler des rendus inutilisables en mémoire.
            _clear_ctk_image_cache()
            self._afficher_spread()

    # ── Taille de carte adaptative ────────────────────────────────────────

    def _compute_card_size(self) -> tuple[int, int]:
        """Calcule card_w et card_h selon la largeur disponible de la fenêtre.

        Raisonnement :
          - On vise à remplir la largeur de la fenêtre avec 2 pages côte à
            côte, chacune contenant une grille (cols × rows) de cartes.
          - On préfère maximiser la lisibilité des cartes dans les limites
            [CARD_W_MIN, CARD_W_MAX].
          - La hauteur n'est pas contraignante grâce au scroll vertical.
        """
        try:
            win_w = self.winfo_width()
        except Exception:
            win_w = 1280

        # Si la fenêtre n'est pas encore mappée, winfo_width() renvoie 1
        if win_w <= 1:
            win_w = 1280

        cols = max(1, self._cols)

        # Largeur disponible pour le spread (2 pages côte à côte)
        available = win_w - 2 * self._OUTER_PAD - self._SCROLLBAR_W
        # Par page
        page_w = (available - self._SPREAD_GAP) / 2
        # Par carte : enlève padding interne page + gaps entre cartes
        card_w = (page_w - 2 * self._PAGE_INNER_PAD
                  - (cols + 1) * BinderPage.GAP) / cols

        # Clamp
        card_w = int(max(CARD_W_MIN, min(CARD_W_MAX, card_w)))
        card_h = int(card_w * CARD_RATIO_H / CARD_RATIO_W)
        return card_w, card_h

    # ── Resize handling (debounce) ────────────────────────────────────────

    def _on_configure(self, event):
        # Ne réagir qu'aux événements émis par self (cadre principal)
        if event.widget is not self:
            return
        if self._resize_after is not None:
            try:
                self.after_cancel(self._resize_after)
            except Exception:
                pass
        self._resize_after = self.after(
            self._RESIZE_DEBOUNCE_MS, self._on_resize_debounced
        )

    def _on_resize_debounced(self):
        self._resize_after = None
        if not self._cartes:
            return
        new_w, new_h = self._compute_card_size()
        # Ne re-render que si la taille a vraiment changé (évite flicker)
        if new_w == self._card_w and new_h == self._card_h:
            return
        self._card_w, self._card_h = new_w, new_h
        self._afficher_spread()

    # ── Filtres ────────────────────────────────────────────────────────────

    def _on_search_change(self, *_):
        if self._search_after:
            self.after_cancel(self._search_after)
        self._search_after = self.after(300, self._appliquer_filtres)

    def _appliquer_filtres(self):
        terme    = self._search_var.get().strip().lower()
        rarity   = self._rarity_var.get()
        poss_f   = self._possessed_var.get()
        result   = []
        for c in self._cartes:
            if terme:
                if (terme not in (c.get("name") or "").lower() and
                        terme not in (c.get("set_code") or "").lower()):
                    continue
            if rarity != "Toutes" and c.get("rarity") != rarity:
                continue
            qty = (c.get("quantite") or 0) > 0
            if poss_f == "Possédées"     and not qty:
                continue
            if poss_f == "Non possédées" and qty:
                continue
            result.append(c)

        # Filtre "une rareté par artwork" (préférence utilisateur).
        # Appliqué APRÈS les autres filtres pour que la rareté gagnante
        # de chaque groupe soit choisie parmi les cartes restantes — utile
        # par exemple si l'utilisateur a filtré sur "Possédées" : on veut
        # alors la plus rare PARMI ses possessions, pas la plus rare en
        # absolu (qu'il pourrait ne pas avoir).
        #
        # Si le filtre rareté du dropdown est actif (≠ "Toutes"), le filtre
        # N-raretés n'a aucun effet (il ne reste qu'une rareté donnée).
        # Pas de cas pathologique à gérer.
        #
        # Priorité de résolution : override par-classeur > préférence globale.
        # Override = None → utilise le global. Override = N≥0 → écrase le global
        # (y compris N=0 qui force "toutes les raretés" même si global ≥ 1).
        try:
            n_override = get_n_raretes_override(self._classeur_code)
            if n_override is not None:
                n_eff = n_override
            else:
                n_eff = preferences.get_n_raretes_par_artwork()
            if n_eff > 0:
                result = filtrer_n_raretes_par_artwork(result, n_eff)
        except Exception as e:
            # Si la lecture des préférences échoue (fichier corrompu,
            # I/O), on retombe sur l'affichage complet — défensive,
            # ce filtre est non-essentiel.
            log.warning(f"_appliquer_filtres n-raretes : {e}")

        self._filtered = result
        self._page     = 0
        self._afficher_spread()

    # ── Affichage (première page seule à droite) ─────────────────────────

    def _nb_spreads(self) -> int:
        """Nombre total de spreads selon la logique première-page-seule.

        - 0 cartes → 1 spread vide
        - 1 page → 1 spread (page à droite seule)
        - N pages → 1 + ceil((N-1) / 2) spreads
        """
        cpp = self._cols * self._rows
        if not self._filtered:
            return 1
        nb_pages = -(-len(self._filtered) // cpp)  # ceil div
        if nb_pages <= 1:
            return 1
        return 1 + -(-(nb_pages - 1) // 2)

    def _spread_slices(self, spread_idx: int) -> tuple[list, list]:
        """Retourne (cartes_gauche, cartes_droite) pour un spread donné.

        Spread 0 : gauche vide, droite = page 0 (cartes 0..cpp-1).
        Spread s (s≥1) : gauche = page 2s-1, droite = page 2s.
        """
        cpp = self._cols * self._rows
        if spread_idx == 0:
            start_r = 0
            return [], self._filtered[start_r : start_r + cpp]
        # s >= 1
        start_l = (2 * spread_idx - 1) * cpp
        start_r = (2 * spread_idx) * cpp
        left  = self._filtered[start_l : start_l + cpp]
        right = self._filtered[start_r : start_r + cpp]
        return left, right

    def _afficher_spread(self):
        """Affiche le spread courant SANS flash (double-buffering).

        Au lieu de détruire les widgets existants AVANT de reconstruire
        (ce qui laisse un vide visible le temps du rebuild + resize images),
        on construit un NOUVEAU frame complet, masqué, puis on swap :
        l'ancien frame n'est détruit qu'une fois le nouveau prêt et placé.
        L'œil ne voit jamais d'état vide intermédiaire.
        """
        nb_spreads = self._nb_spreads()
        p = max(0, min(self._page, nb_spreads - 1))
        self._page = p

        cartes_g, cartes_d = self._spread_slices(p)

        # Nouveau conteneur de spread (pas encore placé → invisible).
        new_frame = ctk.CTkFrame(self._spread_host, fg_color="transparent")
        new_frame.grid_columnconfigure(0, weight=1, uniform="pages")
        new_frame.grid_columnconfigure(1, weight=1, uniform="pages")

        if p == 0:
            BinderPage(
                new_frame, cartes_d,
                self._cols, self._rows,
                self._classeur_code,
                self._card_w, self._card_h,
                on_update=self._on_card_updated,
                open_dialog=self._open_card_dialog,
                open_anomalies=self._open_anomalies_pour_carte,
            ).grid(row=0, column=1, padx=(self._SPREAD_GAP // 2, 0),
                   pady=0, sticky="w")
        else:
            if cartes_g:
                BinderPage(
                    new_frame, cartes_g,
                    self._cols, self._rows,
                    self._classeur_code,
                    self._card_w, self._card_h,
                    on_update=self._on_card_updated,
                    open_dialog=self._open_card_dialog,
                    open_anomalies=self._open_anomalies_pour_carte,
                ).grid(row=0, column=0, padx=(0, self._SPREAD_GAP // 2),
                       pady=0, sticky="e")
            if cartes_d:
                BinderPage(
                    new_frame, cartes_d,
                    self._cols, self._rows,
                    self._classeur_code,
                    self._card_w, self._card_h,
                    on_update=self._on_card_updated,
                    open_dialog=self._open_card_dialog,
                    open_anomalies=self._open_anomalies_pour_carte,
                ).grid(row=0, column=1, padx=(self._SPREAD_GAP // 2, 0),
                       pady=0, sticky="w")

        # Force le calcul de layout du nouveau frame AVANT de l'afficher,
        # pour qu'il apparaisse déjà complet (et non assemblé en direct).
        new_frame.update_idletasks()

        # SWAP : on place le nouveau, puis on retire l'ancien.
        old_frame = self._spread_frame
        new_frame.pack(anchor="center")
        self._spread_frame = new_frame
        if old_frame is not None:
            old_frame.destroy()

        # Synchronise l'entry et le label total avec l'état courant
        self._sync_page_entry(nb_spreads)

        # Boutons nav
        self._btn_prev.configure(
            state="normal" if p > 0 else "disabled",
            fg_color=C["bg_hover"] if p > 0 else C["bg3"],
        )
        self._btn_next.configure(
            state="normal" if p < nb_spreads - 1 else "disabled",
        )

        # Pré-charge en arrière-plan les images des spreads voisins (p±1) pour
        # une navigation instantanée : les workers remplissent le cache
        # CTkImage sans bloquer le thread UI ni afficher de placeholder ici.
        self._prefetch_neighbor_spreads(p, nb_spreads)

    def _prefetch_neighbor_spreads(self, current: int, nb_spreads: int):
        """Réchauffe le cache image des spreads voisins (current ± 1)."""
        specs = []
        w, h = self._card_w, self._card_h
        for idx in (current + 1, current - 1):
            if idx < 0 or idx >= nb_spreads:
                continue
            cg, cd = self._spread_slices(idx)
            for carte in (cg + cd):
                possessed = (carte.get("quantite", 0) or 0) > 0
                path = get_image_path(
                    self._classeur_code, carte.get("image_filename") or ""
                )
                specs.append((path, possessed, w, h, False))
        if specs:
            try:
                prefetch_images(specs)
            except Exception as e:
                log.warning(f"prefetch spreads: {e}")

    def _sync_page_entry(self, nb_spreads: int | None = None):
        """Synchronise l'Entry de page et le label total avec la page courante."""
        try:
            self._page_entry.delete(0, "end")
            self._page_entry.insert(0, str(self._page + 1))
        except Exception:
            pass
        try:
            if nb_spreads is None:
                nb_spreads = self._nb_spreads()
            self._page_total_lbl.configure(text=str(nb_spreads))
        except Exception:
            pass

    def _aller_page_depuis_entry(self, event=None):
        """Navigue vers la page saisie directement dans l'Entry."""
        try:
            val = int(self._page_entry.get().strip())
            nb = self._nb_spreads()
            new_page = max(0, min(nb - 1, val - 1))   # 1-based → 0-based
            if new_page != self._page:
                self._page = new_page
                self._afficher_spread()
            else:
                # Resynchro au cas où l'utilisateur a tapé une valeur
                # identique à la page courante (ex: tapé "1" sur page 1)
                self._sync_page_entry()
        except (ValueError, TypeError):
            # Saisie non numérique → on remet la valeur courante
            self._sync_page_entry()

    def _page_suivante(self):
        if self._page < self._nb_spreads() - 1:
            self._page += 1
            self._afficher_spread()

    def _page_precedente(self):
        self._page = max(0, self._page - 1)
        self._afficher_spread()

    def _on_card_updated(self):
        self._build_navbar_content()

    # ── Dialog carte ───────────────────────────────────────────────────────

    def _open_card_dialog(self, carte: dict):
        DialogCarte(
            self.winfo_toplevel(),
            carte, self._classeur_code,
            on_update=self._on_dialog_update,
        )

    def _on_dialog_update(self):
        self._afficher_spread()
        self._build_navbar_content()

    # ── Barre progression DL ───────────────────────────────────────────────

    def _poll_dl_progress(self):
        if not self._classeur_code:
            return
        taches = [tache for tache in _file_attente.taches
                  if tache.code == self._classeur_code]
        if taches:
            tache = taches[-1]
            prev = getattr(self, "_last_dl_status", None)
            if tache.statut.name == "EN_COURS":
                self._dl_bar_frame.pack(fill="x", after=self._navbar_frame)
                self._dl_lbl.configure(text=f"⏳ {tache.message}")
                self._dl_pbar.set(tache.progression / 100)

                # Re-render périodique du spread pendant le DL pour que les
                # images qui tombent progressivement sur disque apparaissent
                # immédiatement, sans attendre la fin du DL.
                # On ne le fait pas à chaque poll (trop agressif) : on compte
                # les ticks et on re-render toutes les ~3 secondes.
                self._dl_tick_count = getattr(self, "_dl_tick_count", 0) + 1
                if self._dl_tick_count >= 5:   # 5 × 600 ms ≈ 3 s
                    self._dl_tick_count = 0
                    try:
                        from module.gestion_img.cache_images import clear_cache
                        clear_cache()
                    except Exception:
                        pass
                    # Re-render avec les cartes actuelles (self._filtered reste
                    # valide, seules les images sur disque ont changé)
                    try:
                        self._afficher_spread()
                    except Exception as e:
                        log.warning(f"re-render spread during DL: {e}")
            elif tache.statut.name == "TERMINE":
                self._dl_bar_frame.pack_forget()
                self._dl_tick_count = 0
                # Si on vient de finir un DL (transition EN_COURS → TERMINE),
                # des nouvelles images sont maintenant sur disque. Il faut
                # invalider le cache PIL (au cas où des entrées pointent vers
                # d'anciens fichiers remplacés) et re-render le spread pour
                # afficher les artworks fraîchement téléchargés.
                if prev == "EN_COURS":
                    try:
                        from module.gestion_img.cache_images import clear_cache
                        clear_cache()
                    except Exception:
                        pass
                    # Recharge les cartes depuis la DB (car la correction
                    # d'anomalie a inséré de nouvelles lignes) puis re-render.
                    # On ne recall pas charger() complet pour garder la page
                    # courante et l'état des filtres.
                    threading.Thread(
                        target=self._reload_cartes_silent, daemon=True
                    ).start()
            self._last_dl_status = tache.statut.name
            self.after(600, self._poll_dl_progress)
        else:
            self._dl_bar_frame.pack_forget()
            self._last_dl_status = None
            self._dl_tick_count = 0

    def _reload_cartes_silent(self):
        """Recharge les cartes depuis la DB sans reset de la page ni des filtres.

        Utilisé quand un téléchargement d'images vient de se terminer (ex:
        après correction d'anomalie) pour refléter les nouvelles entrées.
        """
        try:
            cartes = get_cartes_info(self._classeur_code)
        except Exception as e:
            log.warning(f"EcranClasseur._reload_cartes_silent: {e}")
            return
        self._safe_after(0, self._on_cartes_silently_reloaded, cartes)

    def _on_cartes_silently_reloaded(self, cartes: list):
        """Met à jour self._cartes et re-render en conservant la page courante."""
        self._cartes = cartes
        # Rafraîchit la liste des raretés (de nouvelles peuvent apparaître)
        raretes = sorted({c.get("rarity", "") for c in cartes if c.get("rarity")})
        current_rarity = self._rarity_var.get()
        self._rarity_cb.configure(values=["Toutes"] + raretes)
        if current_rarity not in ["Toutes"] + raretes:
            self._rarity_var.set("Toutes")
        # Réapplique les filtres actuels (conserve search, rarity, possessed)
        # mais tente de rester sur la même page si possible
        saved_page = self._page
        self._appliquer_filtres()  # remet _page=0
        # Revenir à la page sauvegardée si elle existe encore
        nb = self._nb_spreads()
        self._page = min(saved_page, nb - 1) if nb > 0 else 0
        self._afficher_spread()
        self._build_navbar_content()

    def refresh_after_correction(self):
        """Appelé par DialogAnomalies après correction(s) d'anomalie(s).

        Contrairement à charger(), cette méthode :
          - conserve la page courante et les filtres actifs
          - relance _poll_dl_progress (qui peut s'être arrêté si la file
            était vide), de façon à afficher la progress bar pour les
            nouveaux téléchargements déclenchés
          - recharge la liste des cartes depuis la DB (les corrections ont
            inséré de nouvelles lignes qui doivent apparaître, initialement
            en placeholder tant que l'image n'est pas DL)
        """
        # Relance le polling (il peut s'être arrêté si aucune tâche n'était
        # active avant l'ajout par DialogAnomalies._post_correction).
        self._poll_dl_progress()
        # Reload silencieux des cartes
        threading.Thread(target=self._reload_cartes_silent, daemon=True).start()

    # ── Actions navbar ─────────────────────────────────────────────────────

    def _ouvrir_anomalies(self):
        from module.ui.dialog_anomalies import DialogAnomalies
        DialogAnomalies(
            self.winfo_toplevel(),
            self._classeur_code,
            on_update=self.refresh_after_correction,
        )

    def _open_anomalies_pour_carte(self, carte: dict):
        """Ouvre le dialog "Modifier l'artwork" pour une carte précise.

        Appelé par le menu contextuel "Modifier l'artwork" du clic droit
        sur une vignette du visualiseur.

        Réutilise la feature artwork-alt de l'import CSV (cf.
        module.import_csv.artwork_alt_resolver/artwork_alt_ui) pour LISTER
        les artworks alternatifs connus, mais l'application diffère : ici on
        REMPLACE EN PLACE l'artwork de la carte cliquée (même rowid) via
        `remplacer_artwork_carte`, en conservant possession / quantité /
        qualité. Aucune nouvelle ligne n'est créée (contrairement au flux
        d'import CSV qui, lui, utilise `appliquer_choix_artworks`).

        Le nom de la méthode reste `_open_anomalies_pour_carte` pour
        préserver le contrat d'API avec ClasseurCard (`open_anomalies`
        callback). Le module anomalie reste intouché par cette feature.
        """
        set_code   = (carte.get("set_code") or "").strip()
        rarity     = (carte.get("rarity") or "").strip()
        nom_carte  = (carte.get("name") or "").strip()
        if not set_code:
            return

        # ── Étape 1 : construire la proposition ──────────────────────────
        try:
            from module.import_csv.artwork_alt_resolver import (
                lister_propositions_pour_carte,
            )
            groupe = lister_propositions_pour_carte(
                classeur=self._classeur_code,
                set_code=set_code,
                rarete_full=rarity,
                name=nom_carte,
            )
        except Exception as e:
            log.warning(f"_open_anomalies_pour_carte lookup : {e}")
            groupe = None

        # ── Étape 2 : aucun artwork alternatif disponible ────────────────
        if groupe is None:
            # On distingue 2 cas pour informer correctement l'utilisateur :
            #   (a) cardinfo.db ne connaît AUCUN artwork pour cette carte
            #   (b) tous les artworks connus sont déjà dans le classeur
            # On regarde combien de lignes le classeur a déjà pour ce
            # set_code afin d'afficher le bon message.
            try:
                from module.import_csv.artwork_alt_resolver import (
                    _lister_artworks_classeur,
                )
                nb_existants = len(_lister_artworks_classeur(
                    self._classeur_code, set_code,
                ))
            except Exception:
                nb_existants = 0

            try:
                import tkinter.messagebox as mb
                if nb_existants >= 2:
                    msg = (
                        f"Tous les artworks alternatifs connus pour cette "
                        f"carte ({set_code}) sont déjà présents dans ce "
                        f"classeur ({nb_existants} artwork(s)).\n\n"
                        f"Si une nouvelle entrée apparaît dans cardinfo.db "
                        f"lors d'une mise à jour, elle sera détectée."
                    )
                else:
                    msg = (
                        f"Aucun artwork alternatif connu pour cette carte "
                        f"({set_code}) qui ne soit déjà présent dans le classeur.\n\n"
                        f"Si une nouvelle entrée apparaît dans cardinfo.db lors "
                        f"d'une mise à jour, elle sera détectée."
                    )
                mb.showinfo(
                    "Aucun artwork alternatif",
                    msg,
                    parent=self.winfo_toplevel(),
                )
            except Exception:
                pass
            return

        # ── Étape 3 : callback de validation ─────────────────────────────
        def on_validate(decisions: list[dict]):
            # Remplacement EN PLACE : on change l'artwork de la carte cliquée
            # (même rowid) sans créer de nouvelle ligne, en conservant
            # possession / quantité / qualité. Le dialogue autorise le
            # multi-choix mais un remplacement est 1→1 : on applique le
            # PREMIER artwork coché et on ignore les suivants.
            if not decisions:
                return
            rowid = carte.get("rowid")
            if rowid is None:
                log.warning("Modifier l'artwork : rowid de la carte absent — abandon.")
                return
            choix = decisions[0]
            try:
                from module.import_csv.artwork_alt_resolver import (
                    remplacer_artwork_carte,
                )
                ok = remplacer_artwork_carte(
                    classeur=self._classeur_code,
                    rowid=rowid,
                    new_image_uuid=choix.get("card_image_uuid") or "",
                    new_image_id=choix.get("card_image_id"),
                    new_image_url=choix.get("card_image_url") or "",
                    new_image_small=choix.get("card_image_small") or "",
                )
                # Refresh classeur (déclenche re-fetch + relance polling DL
                # pour télécharger l'image du nouvel artwork).
                self.refresh_after_correction()
                if ok:
                    log.info("Modifier l'artwork : artwork remplacé en place.")
                    if len(decisions) > 1:
                        log.info(
                            f"Modifier l'artwork : {len(decisions) - 1} autre(s) "
                            f"choix ignoré(s) (remplacement = 1 seul artwork)."
                        )
                else:
                    log.warning("Modifier l'artwork : remplacement non appliqué.")
            except Exception as e:
                log.error(f"_open_anomalies_pour_carte on_validate : {e}")

        # ── Étape 4 : ouverture du dialog avec libellés single-card ──────
        try:
            from module.import_csv.artwork_alt_ui import (
                afficher_dialog_artwork_alt,
            )
            nb_propositions = len(groupe["propositions"])
            display_name = nom_carte or groupe.get("name") or set_code
            afficher_dialog_artwork_alt(
                self.winfo_toplevel(),
                propositions=[groupe],
                on_validate=on_validate,
                title=f"Modifier l'artwork — {display_name}",
                subtitle=(
                    f"{nb_propositions} artwork(s) alternatif(s) connu(s) "
                    f"pour {set_code}. Sélectionnez celui que vous voulez : "
                    f"il REMPLACERA l'artwork actuel de cette carte "
                    f"(la quantité et la possession sont conservées). "
                    f"Si vous en cochez plusieurs, seul le premier est utilisé."
                ),
            )
        except Exception as e:
            log.error(f"_open_anomalies_pour_carte open dialog : {e}")

    def _ouvrir_import_export(self):
        """Ouvre la fenêtre Import/Export Scanflip pré-configurée sur ce classeur (Lot 3).

        L'onglet par défaut est Export (cas d'usage le plus courant depuis
        un classeur ouvert : "j'ai modifié possessed/quantité, j'exporte").
        Le dropdown est pré-rempli avec le classeur actuel, mais l'utilisateur
        peut basculer sur "toute la collection" ou un autre classeur.

        Après un import réussi, on rafraîchit le classeur courant pour
        afficher les nouveaux possédés/quantités.
        """
        from module.ui.dialog_import_export import show_import_export_dialog
        show_import_export_dialog(
            self.winfo_toplevel(),
            classeur_initial=self._classeur_code,
            onglet_initial="export",
            on_update=lambda: self.charger(self._classeur_code),
        )

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")

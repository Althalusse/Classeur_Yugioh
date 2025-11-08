import sqlite3
import tkinter as tk
import json
import time
import shutil
import io
import os
import urllib.request
from tkinter import ttk, messagebox
from tkinter import simpledialog
import module.ajout_carte.ajout_carte_service as ajout_carte_service
from module.ajout_carte.ajout_carte_service import completer_infos_depuis_cardinfo, ajouter_carte
from module.centralisation_dossier import CLASSEUR_FOLDER, CARDINFO_DB, CONFIG_FILE
from module.utilitaire.actualisation_UI import creer_callback_rafraichir, refresh_classeurs
from module.gestion_img.gestion_image_classeur import copier_image_personnalisee
from main import afficher_erreur, afficher_info, afficher_warning
from PIL import Image, ImageTk


# Correspond a l'interface graphique pour ajouter une carte dans un classeur

# Ce fichier contient uniquement la logique UI :
# - Création et gestion des widgets Tkinter
# - Gestion des événements utilisateur (bind, callback, etc.)
# - Appels aux fonctions du service (ajout_carte_service) pour toute logique métier
# - Aucun accès direct à la base de données ou traitement métier ici

class AjoutCarteFrame:
    def __init__(self, parent, refresh_callback=None):
        self.parent = parent
        self.refresh_callback = refresh_callback
        self.suffixes = ["/*Alternative art*/"]
        self.rarity_priorities = ajout_carte_service.load_rarity_priorities(CONFIG_FILE)
        self.image_preview = None
        self.image_label = None
        self.image_preview_after_id = None
        self.drag_start = None  # Ajouté pour éviter l'AttributeError
        self.last_drag = None   # Ajouté pour éviter l'AttributeError
        self.setup_ui()

    def setup_ui(self):
        # PanedWindow horizontal pour redimensionnement dynamique
        paned = ttk.PanedWindow(self.parent, orient="horizontal")
        paned.pack(fill="both", expand=True)

        # --- Partie gauche : Recherche + Ajout ---
        left_frame = ttk.Frame(paned)
        paned.add(left_frame, weight=2)

        # Recherche de carte
        search_labelframe = ttk.LabelFrame(left_frame, text="Recherche de carte")
        search_labelframe.pack(fill="both", expand=True, padx=4, pady=(4, 2))
        search_labelframe.columnconfigure(0, weight=1)

        search_bar = ttk.Frame(search_labelframe)
        search_bar.pack(fill="x", pady=(5, 2))
        search_bar.columnconfigure(1, weight=1)
        ttk.Label(search_bar, text="Nom ou code :").grid(row=0, column=0, sticky="w", padx=2)
        self.search_entry = ttk.Entry(search_bar)
        self.search_entry.grid(row=0, column=1, sticky="ew", padx=2)
        ttk.Button(search_bar, text="Rechercher", command=self.search_card).grid(row=0, column=2, padx=2)

        self.result_list = ttk.Treeview(
            search_labelframe, columns=("name", "code", "rarity"),
            show="headings", height=10, selectmode="extended"
        )
        self.result_list.pack(fill="both", expand=True, padx=2, pady=(0, 5))
        self.result_list.heading("name", text="Nom")
        self.result_list.heading("code", text="Code")
        self.result_list.heading("rarity", text="Rareté")
        self.result_list.bind("<Control-a>", self.select_all_results)
        self.result_list.bind("<Button-1>", self.on_click)
        self.result_list.bind("<B1-Motion>", self.on_drag)
        self.result_list.bind("<ButtonRelease-1>", self.on_release)
        self.result_list.bind("<<TreeviewSelect>>", self.on_select_card)
        self.result_list.bind("<Button-3>", self.on_right_click_result)
        self.result_list.configure(selectmode="extended")

        # Ajout d'une séparation visuelle
        ttk.Separator(left_frame, orient="horizontal").pack(fill="x", padx=4, pady=2)

        # Ajouter une carte avec Frame contenant une hauteur contrôlable
        form_container = ttk.Frame(left_frame)
        form_container.pack(fill="x", padx=4, pady=(2, 4))
        # Pour une hauteur fixe :
        # form_container.pack_propagate(False)
        # form_container.config(height=260)
        # Pour une hauteur minimale (mais extensible si la fenêtre grandit) :
        form_container.update_idletasks()
        form_container.after(0, lambda: form_container.winfo_toplevel().update_idletasks())
        form_container.after(0, lambda: form_container.config(height=max(260, form_container.winfo_height())))
        form_container.pack_propagate(False)

        form_labelframe = ttk.LabelFrame(form_container, text="Ajouter une carte")
        form_labelframe.pack(fill="both", expand=True)
        for i in range(2):
            form_labelframe.columnconfigure(i, weight=1 if i == 1 else 0)

        self.entries = {}
        ttk.Label(form_labelframe, text="Classeur :").grid(row=0, column=0, sticky="e", padx=2, pady=2)
        self.combo_classeur = ttk.Combobox(form_labelframe, values=ajout_carte_service.get_classeurs(CLASSEUR_FOLDER))
        self.combo_classeur.grid(row=0, column=1, sticky="ew", padx=2, pady=2)
        ttk.Button(form_labelframe, text="↻", width=3, command=lambda: refresh_classeurs(self)).grid(row=0, column=2, padx=2, pady=2)

        ttk.Label(form_labelframe, text="Nom de la carte :").grid(row=1, column=0, sticky="e", padx=2, pady=2)
        self.entries["name"] = ttk.Entry(form_labelframe)
        self.entries["name"].grid(row=1, column=1, sticky="ew", padx=2, pady=2)
        self.suffix_var = tk.StringVar()
        self.suffix_combo = ttk.Combobox(form_labelframe, values=[""] + self.suffixes, width=16, textvariable=self.suffix_var, state="readonly")
        self.suffix_combo.grid(row=1, column=2, padx=2, pady=2)
        self.suffix_combo.set("")

        ttk.Label(form_labelframe, text="Code :").grid(row=2, column=0, sticky="e", padx=2, pady=2)
        self.entries["card_sets_set_code"] = ttk.Entry(form_labelframe)
        self.entries["card_sets_set_code"].grid(row=2, column=1, columnspan=2, sticky="ew", padx=2, pady=2)

        ttk.Label(form_labelframe, text="Rareté :").grid(row=3, column=0, sticky="e", padx=2, pady=2)
        self.entries["card_sets_set_rarity"] = ttk.Combobox(form_labelframe, values=[""] + list(self.rarity_priorities.keys()))
        self.entries["card_sets_set_rarity"].grid(row=3, column=1, columnspan=2, sticky="ew", padx=2, pady=2)
        self.entries["card_sets_set_rarity"].bind('<KeyRelease>', self.filter_rarity)

        ttk.Label(form_labelframe, text="Description :").grid(row=4, column=0, sticky="e", padx=2, pady=2)
        self.entries["description"] = ttk.Entry(form_labelframe)
        self.entries["description"].grid(row=4, column=1, columnspan=2, sticky="ew", padx=2, pady=2)

        ttk.Label(form_labelframe, text="URL Image :").grid(row=5, column=0, sticky="e", padx=2, pady=2)
        self.entries["card_images_image_url"] = ttk.Entry(form_labelframe)
        self.entries["card_images_image_url"].grid(row=5, column=1, sticky="ew", padx=2, pady=2)
        ttk.Button(form_labelframe, text="Image personnalisée", command=self.choisir_image_personnalisee).grid(row=5, column=2, padx=2, pady=2)
        self.entries["card_images_image_url"].bind('<KeyRelease>', self.schedule_image_preview)
        self.entries["card_images_image_url"].bind('<<ComboboxSelected>>', self.schedule_image_preview)

        self.flags = {}
        self.flags["possessed"] = tk.BooleanVar(value=False)
        ttk.Checkbutton(form_labelframe, text="Possédée", variable=self.flags["possessed"]).grid(row=6, column=0, sticky="w", padx=2, pady=2)

        actions_frame = ttk.Frame(form_labelframe)
        actions_frame.grid(row=7, column=0, columnspan=3, sticky="ew", padx=2, pady=(8, 2))
        ttk.Button(actions_frame, text="Ajouter la carte", command=self.ajouter_carte).pack(side="right", padx=6)

        # Aperçu image à droite du formulaire
        # Réduire la largeur du label d'aperçu image
        self.image_label = ttk.Label(form_labelframe, anchor="center", relief="groove", width=50)
        self.image_label.grid(row=0, column=3, rowspan=8, sticky="nsew", padx=(8, 2), pady=2)

        # --- Partie droite : Cartes personnalisées ---
        right_frame = ttk.Frame(paned)
        paned.add(right_frame, weight=3)

        custom_labelframe = ttk.LabelFrame(right_frame, text="Cartes personnalisées")
        custom_labelframe.pack(fill="both", expand=True, padx=4, pady=4)
        custom_labelframe.columnconfigure(0, weight=1)
        custom_labelframe.rowconfigure(1, weight=1)

        filter_frame = ttk.Frame(custom_labelframe)
        filter_frame.grid(row=0, column=0, sticky="ew", padx=5, pady=(5, 0))
        ttk.Label(filter_frame, text="Filtrer par classeur :").pack(side="left")
        self.custom_filter = ttk.Combobox(filter_frame, values=["Tous"] + ajout_carte_service.get_classeurs(CLASSEUR_FOLDER))
        self.custom_filter.pack(side="left", fill="x", expand=True, padx=5)
        self.custom_filter.set("Tous")
        self.custom_filter.bind('<<ComboboxSelected>>', lambda e: self.refresh_custom_cards())

        self.custom_list = ttk.Treeview(
            custom_labelframe, columns=("name", "code", "rarity", "classeur"),
            show="headings", height=25, selectmode="extended"
        )
        self.custom_list.heading("name", text="Nom")
        self.custom_list.heading("code", text="Code")
        self.custom_list.heading("rarity", text="Rareté")
        self.custom_list.heading("classeur", text="Classeur")
        self.custom_list.grid(row=1, column=0, sticky="nsew", padx=5, pady=5)
        # Ajout : multiple selection via Ctrl+A et drag mouse
        self.custom_list.bind("<Control-a>", self.select_all_custom)
        self.custom_list.bind("<Button-1>", self.on_click_custom)
        self.custom_list.bind("<B1-Motion>", self.on_drag_custom)
        self.custom_list.bind("<ButtonRelease-1>", self.on_release_custom)
        self.custom_list.configure(selectmode="extended")

        custom_buttons_frame = ttk.Frame(custom_labelframe)
        custom_buttons_frame.grid(row=2, column=0, sticky="ew", pady=5)
        ttk.Button(custom_buttons_frame, text="Supprimer la carte sélectionnée", command=self.supprimer_carte_selectionnee).pack(side="right", padx=5)
        ttk.Button(custom_buttons_frame, text="↻ Rafraîchir la liste", command=self.refresh_custom_cards).pack(side="right", padx=5)

    def get_classeurs(self):
        return ajout_carte_service.get_classeurs(CLASSEUR_FOLDER)

    def ajouter_carte(self):
        classeur = self.combo_classeur.get()
        if not classeur:
            afficher_erreur("Veuillez sélectionner un classeur")
            return

        selected = self.result_list.selection()
        if not selected:
            data = {field: entry.get().strip() for field, entry in self.entries.items()}
            suffix = self.suffix_var.get()
            if suffix:
                data["name"] = f"{data['name']}{suffix}"
            if not all([data["name"], data["card_sets_set_code"]]):
                afficher_erreur("Le nom et le code sont obligatoires")
                return
            try:
                db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
                cardinfo_db = CARDINFO_DB  # Utiliser la base de données cardinfo
                # Compléter les informations depuis cardinfo
                data = completer_infos_depuis_cardinfo(cardinfo_db, data)
                # Ajouter la carte avec les données complétées
                ajouter_carte(db_path, data, self.flags["possessed"].get())
                afficher_info("Carte ajoutée avec succès")
                for entry in self.entries.values():
                    entry.delete(0, "end")
                self.refresh_custom_cards()
                if self.refresh_callback:
                    self.refresh_callback()
            except Exception as e:
                afficher_erreur(f"Erreur lors de l'ajout : {str(e)}")
        else:
            try:
                db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
                cards = []
                for item in selected:
                    card_dict = json.loads(self.result_list.item(item)["tags"][0])
                    suffix = self.suffix_var.get()
                    nom_form = self.entries["name"].get().strip()
                    if nom_form:
                        card_dict["name"] = nom_form
                    if suffix:
                        card_dict["name"] += suffix
                    code_form = self.entries["card_sets_set_code"].get().strip()
                    if code_form:
                        card_dict["card_sets_set_code"] = code_form
                    rarete_form = self.entries["card_sets_set_rarity"].get().strip()
                    if rarete_form:
                        card_dict["card_sets_set_rarity"] = rarete_form
                    desc_form = self.entries["description"].get().strip()
                    if desc_form:
                        card_dict["description"] = desc_form
                    img_form = self.entries["card_images_image_url"].get().strip()
                    if img_form:
                        card_dict["card_images_image_url"] = img_form
                    cards.append(card_dict)
                added_count = ajout_carte_service.ajouter_cartes_selectionnees(db_path, cards, self.flags["possessed"].get())
                afficher_info(f"{added_count} carte(s) ajoutée(s) avec succès")
                self.result_list.selection_remove(selected)
                for entry in self.entries.values():
                    entry.delete(0, "end")
                self.suffix_combo.set("")
                self.refresh_custom_cards()
                if self.refresh_callback:
                    self.refresh_callback()
            except Exception as e:
                afficher_erreur(f"Erreur lors de l'ajout : {str(e)}")

    def search_card(self):
        search_term = self.search_entry.get().strip()
        if not search_term:
            return
        try:
            results = ajout_carte_service.search_card(CARDINFO_DB, search_term, self.rarity_priorities)
            for item in self.result_list.get_children():
                self.result_list.delete(item)
            for card_dict in results:
                self.result_list.insert("", "end",
                    values=(card_dict["name"], card_dict["card_sets_set_code"], card_dict["card_sets_set_rarity"]),
                    tags=(json.dumps(card_dict),))
        except Exception as e:
            afficher_erreur(f"Erreur lors de la recherche : {str(e)}")

    def refresh_custom_cards(self):
        for item in self.custom_list.get_children():
            self.custom_list.delete(item)
        try:
            selected_classeur = self.custom_filter.get()
            classeurs = [selected_classeur] if selected_classeur != "Tous" else self.get_classeurs()
            rows = ajout_carte_service.get_custom_cards(CLASSEUR_FOLDER, classeurs)
            for row in rows:
                self.custom_list.insert("", "end", values=row)
        except Exception as e:
            afficher_erreur(f"Erreur lors du rafraîchissement : {str(e)}")

    def supprimer_carte(self):
        classeur = self.combo_classeur.get()
        if not classeur:
            afficher_erreur("Veuillez sélectionner un classeur")
            return
        nom = self.entries["name"].get().strip()
        code = self.entries["card_sets_set_code"].get().strip()
        rarete = self.entries["card_sets_set_rarity"].get().strip()
        if not all([nom, code, rarete]):
            afficher_erreur("Veuillez remplir au moins le nom, le code et la rareté de la carte")
            return
        suffix = self.suffix_var.get()
        if suffix:
            nom = f"{nom}{suffix}"
        if not messagebox.askyesno("Confirmation", 
                                  f"Voulez-vous vraiment supprimer la carte\n{nom} ({code}) - {rarete}\ndu classeur {classeur} ?"):
            return
        try:
            db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
            count = ajout_carte_service.supprimer_carte(db_path, nom, code, rarete)
            if count == 0:
                afficher_warning("Aucune carte correspondante trouvée dans le classeur")
            else:
                afficher_info(f"{count} carte(s) supprimée(s) avec succès")
                for entry in self.entries.values():
                    entry.delete(0, "end")
                self.suffix_combo.set("")
                if self.refresh_callback:
                    self.refresh_callback()
        except Exception as e:
            afficher_erreur(f"Erreur lors de la suppression : {str(e)}")

    def supprimer_carte_selectionnee(self):
        selected = self.custom_list.selection()
        if not selected:
            afficher_warning("Veuillez sélectionner au moins une carte à supprimer")
            return
        cards_to_delete = []
        for item in selected:
            values = self.custom_list.item(item)["values"]
            cards_to_delete.append(values)
        cards_str = "\n".join([f"{nom} ({code}) - {rarete} ({classeur})" 
                          for nom, code, rarete, classeur in cards_to_delete])
        if not messagebox.askyesno("Confirmation", 
                              f"Voulez-vous vraiment supprimer ces cartes ?\n\n{cards_str}"):
            return
        try:
            by_classeur = {}
            for nom, code, rarete, classeur in cards_to_delete:
                if classeur not in by_classeur:
                    by_classeur[classeur] = []
                by_classeur[classeur].append((nom, code, rarete))
            deleted_count = ajout_carte_service.supprimer_cartes_selectionnees(CLASSEUR_FOLDER, by_classeur)
            self.refresh_custom_cards()
            if self.refresh_callback:
                self.refresh_callback()
            afficher_info(f"{deleted_count} carte(s) supprimée(s) avec succès")
        except Exception as e:
            afficher_erreur(f"Erreur lors de la suppression : {str(e)}")

    def select_all_results(self, event):
        """Sélectionne tous les éléments de la liste"""
        self.result_list.selection_set(self.result_list.get_children())
        return "break"  # Empêche la propagation de l'événement

    def on_click(self, event):
        """Enregistre le point de départ du glissement"""
        # Correction : autoriser la sélection multiple par glissement
        row = self.result_list.identify_row(event.y)
        if not row:
            return
        if event.state & 0x0004:  # Ctrl
            if row in self.result_list.selection():
                self.result_list.selection_remove(row)
            else:
                self.result_list.selection_add(row)
        elif event.state & 0x0001:  # Shift
            selection = self.result_list.selection()
            items = self.result_list.get_children()
            if selection:
                first = items.index(selection[0])
                last = items.index(row)
                rng = items[min(first, last):max(first, last)+1]
                self.result_list.selection_set(rng)
            else:
                self.result_list.selection_set(row)
        else:
            self.result_list.selection_set(row)
        self.drag_start = row
        self.last_drag = row

    def on_drag(self, event):
        """Gère la sélection pendant le glissement"""
        # Permet la sélection même si le curseur est en dehors d'un item
        if not self.drag_start:
            return
        current = self.result_list.identify_row(event.y)
        items = self.result_list.get_children()
        if not current:
            # Si le curseur est au-dessus ou en dessous de la liste, sélectionne tout jusqu'au début ou la fin
            y = event.y
            if y < 0:
                current = items[0]
            else:
                current = items[-1]
        if current not in items:
            return
        start_idx = items.index(self.drag_start)
        current_idx = items.index(current)
        if start_idx < current_idx:
            to_select = items[start_idx:current_idx+1]
        else:
            to_select = items[current_idx:start_idx+1]
        self.result_list.selection_set(to_select)
        self.last_drag = current

    def on_release(self, event):
        """Réinitialise les variables de glissement"""
        self.drag_start = None
        self.last_drag = None

    def on_select_card(self, event):
        selected_items = self.result_list.selection()
        if not selected_items:
            return
        # Toujours remplir les champs pour la première sélection
        card_dict = json.loads(self.result_list.item(selected_items[0])["tags"][0])
        self.suffix_combo.set("")
        self.entries["name"].delete(0, "end")
        self.entries["name"].insert(0, card_dict.get("name", ""))
        self.entries["card_sets_set_code"].delete(0, "end")
        self.entries["card_sets_set_code"].insert(0, card_dict.get("card_sets_set_code", ""))
        # Si une seule carte sélectionnée, remplir la rareté, sinon laisser la rareté inchangée
        if len(selected_items) == 1:
            self.entries["card_sets_set_rarity"].delete(0, "end")
            self.entries["card_sets_set_rarity"].insert(0, card_dict.get("card_sets_set_rarity", ""))
        # Toujours remplir les autres champs
        self.entries["description"].delete(0, "end")
        self.entries["description"].insert(0, card_dict.get("description", ""))
        self.entries["card_images_image_url"].delete(0, "end")
        if card_dict.get("card_images_image_url"):
            self.entries["card_images_image_url"].insert(0, card_dict["card_images_image_url"])
        self.flags["possessed"].set(bool(card_dict.get("possessed", False)))

    def filter_rarity(self, event):
        value = event.widget.get().lower()
        all_rarities = list(self.rarity_priorities.keys())
        if value:
            filtered_rarities = [rarity for rarity in all_rarities if rarity.lower().startswith(value)]
        else:
            filtered_rarities = all_rarities
        event.widget['values'] = filtered_rarities
        if len(filtered_rarities) == 1 and value in filtered_rarities[0].lower():
            event.widget.set(filtered_rarities[0])
        if filtered_rarities:
            event.widget.event_generate('<Down>')

    def choisir_image_personnalisee(self):
        chemin_image = copier_image_personnalisee(self.entries["name"].get().strip())
        if chemin_image:
            self.entries["card_images_image_url"].delete(0, "end")
            self.entries["card_images_image_url"].insert(0, chemin_image)
            # Affiche l'aperçu immédiatement pour une image locale
            self.update_image_preview()

    def schedule_image_preview(self, event=None):
        # Annule le précédent appel différé s'il existe
        if self.image_preview_after_id is not None:
            self.parent.after_cancel(self.image_preview_after_id)
        # Planifie la mise à jour de l'aperçu dans 2 secondes (2000 ms)
        self.image_preview_after_id = self.parent.after(2000, self.update_image_preview)

    def update_image_preview(self, event=None):
        self.image_preview_after_id = None  # Reset l'ID car l'action est exécutée
        url = self.entries["card_images_image_url"].get().strip()
        if not url:
            if self.image_label is not None:
                self.image_label.config(image='', text='Aucun aperçu')
            self.image_preview = None
            return
        try:
            im = ajout_carte_service.charger_image_depuis_url_ou_fichier(url)
            self.image_preview = ImageTk.PhotoImage(im)
            if self.image_label is not None:
                self.image_label.config(image=self.image_preview, text='')
        except Exception:
            if self.image_label is not None:
                self.image_label.config(image='', text='Image non trouvée')
            self.image_preview = None

    def select_all_custom(self, event):
        """Sélectionne tous les éléments de la liste personnalisée"""
        self.custom_list.selection_set(self.custom_list.get_children())
        return "break"

    def on_click_custom(self, event):
        """Débute la sélection multiple par glissement sur custom_list"""
        row = self.custom_list.identify_row(event.y)
        items = self.custom_list.get_children()
        if not row:
            self.custom_list.selection_remove(self.custom_list.selection())
            self.custom_drag_start = None
            self.custom_last_drag = None
            return
        if not hasattr(self, "custom_drag_start"):
            self.custom_drag_start = None
            self.custom_last_drag = None
        if event.state & 0x0004:  # Ctrl
            if row in self.custom_list.selection():
                self.custom_list.selection_remove(row)
            else:
                self.custom_list.selection_add(row)
        elif event.state & 0x0001:  # Shift
            selection = self.custom_list.selection()
            if selection:
                first = items.index(selection[0])
                last = items.index(row)
                rng = items[min(first, last):max(first, last)+1]
                self.custom_list.selection_set(rng)
            else:
                self.custom_list.selection_set(row)
        else:
            self.custom_list.selection_set(row)
        self.custom_drag_start = row
        self.custom_last_drag = row

    def on_drag_custom(self, event):
        """Gère la sélection pendant le glissement sur custom_list"""
        if not hasattr(self, "custom_drag_start") or not self.custom_drag_start:
            return
        current = self.custom_list.identify_row(event.y)
        items = self.custom_list.get_children()
        if not current:
            return
        if current not in items:
            return
        start_idx = items.index(self.custom_drag_start)
        current_idx = items.index(current)
        if start_idx < current_idx:
            to_select = items[start_idx:current_idx+1]
        else:
            to_select = items[current_idx:start_idx+1]
        self.custom_list.selection_set(to_select)
        self.custom_last_drag = current

    def on_release_custom(self, event):
        """Réinitialise les variables de glissement pour custom_list"""
        self.custom_drag_start = None
        self.custom_last_drag = None

    def on_right_click_result(self, event):
        row_id = self.result_list.identify_row(event.y)
        if not row_id:
            return
        self.result_list.selection_set(row_id)
        card_dict = json.loads(self.result_list.item(row_id)["tags"][0])
        classeurs = self.get_classeurs()
        if not classeurs:
            afficher_info("Aucun classeur disponible.")
            return

        menu = tk.Menu(self.result_list, tearoff=0)
        sous_menu = tk.Menu(menu, tearoff=0)
        for c in classeurs:
            sous_menu.add_command(
                label=c,
                command=lambda choix=c: self.ajouter_carte_dans_classeur(card_dict, choix)
            )
        menu.add_cascade(label="Ajouter cette carte dans un classeur", menu=sous_menu)
        menu.tk_popup(event.x_root, event.y_root)

    def ajouter_carte_dans_classeur(self, card_dict, classeur):
        db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
        try:
            ajouter_carte(db_path, card_dict, possessed=1, is_custom=False)
            afficher_info(f"Carte ajoutée à {classeur}.")
        except Exception as e:
            afficher_erreur(f"Erreur lors de l'ajout : {str(e)}")

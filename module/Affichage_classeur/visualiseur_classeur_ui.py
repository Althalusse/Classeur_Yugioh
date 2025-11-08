import os
import tkinter as tk
import sqlite3
from tkinter import ttk, messagebox
from module.centralisation_dossier import AFFICHER_CARTE, DEFAULT_IMAGE_PATH
from module.carte_posseder.gestion_carte_posseder import set_all_possessed
from module.carte_posseder.affichage_carte_classeur import get_cartes_info
from module.utilitaire.actualisation_UI import creer_callback_rafraichir
from module.recherche_carte import recherche_carte
from module.Affichage_classeur.gestionnaire_classeur import obtenir_liste_classeurs
from PIL import Image, ImageTk


def afficher_cartes_interface(cadre_parent=None, callback_rafraichir_stats=None):
    viewer = VisualiseurClasseur(cadre_parent, callback_rafraichir_stats)
    return viewer.rafraichir_classeurs


class VisualiseurClasseur:
    def __init__(self, cadre_parent=None, callback_rafraichir_stats=None):
        self.callback_rafraichir_stats = callback_rafraichir_stats
        self.racine = None
        if cadre_parent is None:
            self.racine = tk.Tk()
            self.racine.title("Affichage des cartes")
            self.racine.geometry("1280x1024")
            cadre_parent = ttk.Frame(self.racine)
            cadre_parent.pack(fill=tk.BOTH, expand=True)
        self.cadre_parent = cadre_parent
        self.classeur_selectionne = tk.StringVar()
        self.cartes = []
        self.images_cache = []
        self.pages_max = 1
        self.page_var = tk.IntVar(value=1)
        self.titre_classeur = None  # Ajout de l'attribut pour le titre
        self._construire_ui()
        if self.racine is not None:
            self.racine.mainloop()

    def _construire_ui(self):
        cadre_principal = ttk.Frame(self.cadre_parent)
        cadre_principal.pack(fill=tk.BOTH, expand=True)

        # Ajout du titre du classeur en haut
        self.titre_classeur = tk.Label(
            cadre_principal,
            text="Aucun classeur sélectionné",
            font=("Arial", 16, "bold"),
            pady=10,
        )
        self.titre_classeur.pack(fill="x")

        cadre_sidebar = ttk.Frame(cadre_principal, width=220)
        cadre_sidebar.pack(side="left", fill="y", padx=10, pady=10)
        cadre_sidebar.pack_propagate(False)
        cadre_contenu = ttk.Frame(cadre_principal)
        cadre_contenu.pack(side="left", fill=tk.BOTH, expand=True)
        tk.Label(
            cadre_sidebar, text="Sélectionner un classeur", font=("Arial", 11, "bold")
        ).pack(pady=(5, 2))
        cadre_charger = ttk.Frame(cadre_sidebar)
        cadre_charger.pack(pady=(0, 5))
        self.combo_classeurs = ttk.Combobox(
            cadre_charger,
            values=obtenir_liste_classeurs(),
            textvariable=self.classeur_selectionne,
            state="readonly",
        )
        self.combo_classeurs.pack(side="left", padx=5)
        self.bouton_charger = ttk.Button(
            cadre_charger, text="Charger", command=self.charger_les_cartes
        )
        self.bouton_charger.pack(side="left", padx=5)
        self.classeur_selectionne.set("")
        ttk.Separator(cadre_sidebar, orient="horizontal").pack(fill="x", pady=8)
        cadre_options = ttk.Frame(cadre_sidebar)
        cadre_options.pack(pady=0)
        ttk.Label(cadre_options, text="Colonnes :").grid(
            row=0, column=0, padx=5, sticky="w"
        )
        self.colonnes_var = tk.IntVar(value=3)
        spin_colonnes = ttk.Spinbox(
            cadre_options, from_=1, to=10, width=5, textvariable=self.colonnes_var
        )
        spin_colonnes.grid(row=0, column=1, padx=5)
        ttk.Label(cadre_options, text="Lignes :").grid(
            row=1, column=0, padx=5, sticky="w"
        )
        self.lignes_var = tk.IntVar(value=3)
        spin_lignes = ttk.Spinbox(
            cadre_options, from_=1, to=10, width=5, textvariable=self.lignes_var
        )
        spin_lignes.grid(row=1, column=1, padx=5)
        ttk.Separator(cadre_sidebar, orient="horizontal").pack(fill="x", pady=8)
        cadre_full_unset = ttk.Frame(cadre_sidebar)
        cadre_full_unset.pack(pady=0)
        self.bouton_full_set = ttk.Button(
            cadre_full_unset,
            text="Tout possédé",
            state="disabled",
            command=lambda: self.tout_marquer_comme_possede_ui(1),
        )
        self.bouton_full_set.pack(side="top", fill="x", pady=(0, 5))
        self.bouton_unset = ttk.Button(
            cadre_full_unset,
            text="Tout non possédé",
            state="disabled",
            command=lambda: self.tout_marquer_comme_possede_ui(0),
        )
        self.bouton_unset.pack(side="top", fill="x")
        ttk.Separator(cadre_sidebar, orient="horizontal").pack(fill="x", pady=8)
        cadre_recherche = ttk.LabelFrame(cadre_sidebar, text="Recherche Carte")
        cadre_recherche.pack(fill="x", padx=2, pady=8)
        ttk.Label(cadre_recherche, text="Nom de la carte:").pack(
            anchor="w", padx=5, pady=(5, 0)
        )
        self.entree_nom = ttk.Entry(cadre_recherche)
        self.entree_nom.pack(fill="x", padx=5)
        ttk.Label(cadre_recherche, text="Code set:").pack(
            anchor="w", padx=5, pady=(5, 0)
        )
        self.entree_code = ttk.Entry(cadre_recherche)
        self.entree_code.pack(fill="x", padx=5)
        self.cadre_resultats = ttk.Frame(cadre_recherche)
        self.cadre_resultats.pack(fill="x", padx=5, pady=5)
        btn_search = ttk.Button(
            cadre_recherche, text="Rechercher", command=self.executer_recherche
        )
        btn_search.pack(pady=5)
        cadre_cartes = ttk.Frame(cadre_contenu)
        cadre_cartes.pack(fill=tk.BOTH, expand=True)
        self.canvas = tk.Canvas(cadre_cartes)
        self.scrollbar = ttk.Scrollbar(
            cadre_cartes, orient="vertical", command=self.canvas.yview
        )
        self.scrollable_frame = ttk.Frame(self.canvas)
        self.scrollable_frame.bind(
            "<Configure>",
            lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")),
        )
        self.canvas.create_window((0, 0), window=self.scrollable_frame, anchor="nw")
        self.canvas.configure(yscrollcommand=self.scrollbar.set)
        self.canvas.pack(side="left", fill="both", expand=True)
        self.scrollbar.pack(side="right", fill="y")
        self.lbl_page = ttk.Label(cadre_contenu, text="Page 1")
        self.lbl_page.pack(pady=5)
        navigation_frame = ttk.Frame(cadre_contenu)
        navigation_frame.pack(pady=5)
        bouton_precedent = ttk.Button(
            navigation_frame, text="<< Page précédente", command=self.page_precedente
        )
        bouton_precedent.pack(side="left", padx=5)
        bouton_suivant = ttk.Button(
            navigation_frame, text="Page suivante >>", command=self.page_suivante
        )
        bouton_suivant.pack(side="left", padx=5)
        cadre_goto = ttk.Frame(cadre_contenu)
        cadre_goto.pack(pady=5)
        ttk.Label(cadre_goto, text="Aller à la page :").pack(side="left", padx=(0, 5))
        self.entree_page = ttk.Entry(cadre_goto, width=5)
        self.entree_page.pack(side="left")
        btn_goto = ttk.Button(cadre_goto, text="Go", command=self.aller_a_la_page)
        btn_goto.pack(side="left", padx=5)

    def afficher_page(self, page_num):
        from .gestionnaire_classeur import afficher_cartes_sur_cadre

        colonnes = self.colonnes_var.get()
        lignes = self.lignes_var.get()
        cartes_par_page = colonnes * lignes
        nb_total = len(self.cartes)
        self.pages_max = (nb_total + cartes_par_page - 1) // cartes_par_page
        for widget in self.scrollable_frame.winfo_children():
            widget.destroy()
        self.images_cache.clear()
        if page_num < 1:
            page_num = 1
        if page_num > self.pages_max:
            page_num = self.pages_max
        self.page_var.set(page_num)
        if page_num == 1 or self.pages_max == 1:
            cartes_page = self.cartes[0:cartes_par_page]
            afficher_cartes_sur_cadre(
                self.scrollable_frame,
                cartes_page,
                colonnes,
                self.images_cache,
                self.classeur_selectionne,
                self.callback_rafraichir_stats,
            )
            self.lbl_page.config(text=f"Page 1 sur {self.pages_max}")
        else:
            cadre_gauche = ttk.Frame(self.scrollable_frame)
            cadre_gauche.grid(row=0, column=0, sticky="nsew", padx=10)
            cadre_droite = ttk.Frame(self.scrollable_frame)
            cadre_droite.grid(row=0, column=1, sticky="nsew", padx=10)
            start1 = (page_num - 1) * cartes_par_page
            end1 = start1 + cartes_par_page
            cartes_page1 = self.cartes[start1:end1]
            start2 = start1 + cartes_par_page
            end2 = start2 + cartes_par_page
            cartes_page2 = self.cartes[start2:end2] if start2 < nb_total else []
            afficher_cartes_sur_cadre(
                cadre_gauche,
                cartes_page1,
                colonnes,
                self.images_cache,
                self.classeur_selectionne,
                self.callback_rafraichir_stats,
            )
            if cartes_page2:
                afficher_cartes_sur_cadre(
                    cadre_droite,
                    cartes_page2,
                    colonnes,
                    self.images_cache,
                    self.classeur_selectionne,
                    self.callback_rafraichir_stats,
                )
            self.lbl_page.config(
                text=f"Pages {page_num} et {page_num + 1} sur {self.pages_max}"
            )

    def charger_les_cartes(self):
        from .gestionnaire_classeur import verifier_et_telecharger_images_manquantes

        if not self.classeur_selectionne.get():
            messagebox.showerror("Erreur", "Veuillez sélectionner un classeur.")
            return
        try:
            self.cartes = get_cartes_info(self.classeur_selectionne.get())
            # Mise à jour du titre avec le nom du set depuis la base de données
            db_path = os.path.join(
                AFFICHER_CARTE,
                self.classeur_selectionne.get(),
                f"{self.classeur_selectionne.get()}.db",
            )
            if os.path.exists(db_path):
                conn = sqlite3.connect(db_path)
                cursor = conn.cursor()
                cursor.execute("SELECT card_sets_set_name FROM cards LIMIT 1")
                result = cursor.fetchone()
                conn.close()

                if self.titre_classeur and self.titre_classeur.winfo_exists():
                    if result and result[0]:
                        self.titre_classeur.configure(
                            text=f"Classeur : {result[0]} ({self.classeur_selectionne.get()})",
                            anchor="center",
                            justify="center",
                        )
                    else:
                        self.titre_classeur.configure(
                            text=f"Classeur : {self.classeur_selectionne.get()}",
                            anchor="center",
                            justify="center",
                        )

            verifier_et_telecharger_images_manquantes(
                self.classeur_selectionne.get(), self.cartes
            )
            if not self.cartes:
                messagebox.showinfo("Info", "Aucune carte à afficher pour ce classeur.")
            self.afficher_page(1)
            if self.bouton_full_set and self.bouton_full_set.winfo_exists():
                self.bouton_full_set.configure(state="normal")
            if self.bouton_unset and self.bouton_unset.winfo_exists():
                self.bouton_unset.configure(state="normal")

        except FileNotFoundError as e:
            messagebox.showerror("Erreur", str(e))
            if self.bouton_full_set and self.bouton_full_set.winfo_exists():
                self.bouton_full_set.configure(state="disabled")
            if self.bouton_unset and self.bouton_unset.winfo_exists():
                self.bouton_unset.configure(state="disabled")
        except Exception as e:
            messagebox.showerror(
                "Erreur", f"Erreur lors du chargement des cartes :\n{e}"
            )
            if self.bouton_full_set and self.bouton_full_set.winfo_exists():
                self.bouton_full_set.configure(state="disabled")
            if self.bouton_unset and self.bouton_unset.winfo_exists():
                self.bouton_unset.configure(state="disabled")

    def tout_marquer_comme_possede_ui(self, valeur):
        try:
            set_all_possessed(self.classeur_selectionne.get(), valeur)
            for carte in self.cartes:
                carte["possessed"] = valeur
            self.afficher_page(self.page_var.get())
            if self.callback_rafraichir_stats:
                self.callback_rafraichir_stats()
        except Exception as e:
            messagebox.showerror(
                "Erreur BDD", f"Impossible de mettre à jour toutes les cartes:\n{e}"
            )

    def page_suivante(self):
        if self.page_var.get() == 1:
            self.afficher_page(2)
        else:
            self.afficher_page(self.page_var.get() + 2)

    def page_precedente(self):
        if self.page_var.get() <= 3:
            self.afficher_page(1)
        else:
            self.afficher_page(self.page_var.get() - 2)

    def aller_a_la_page(self):
        try:
            page = int(self.entree_page.get())
            if 1 <= page <= self.pages_max:
                self.afficher_page(page)
            else:
                messagebox.showwarning(
                    "Page invalide", f"Entrez un numéro entre 1 et {self.pages_max}"
                )
        except ValueError:
            messagebox.showwarning(
                "Entrée invalide", "Veuillez entrer un numéro de page valide."
            )

    def executer_recherche(self):
        for widget in self.cadre_resultats.winfo_children():
            widget.destroy()
        nom = self.entree_nom.get().strip()
        code = self.entree_code.get().strip()

        resultats = recherche_carte.rechercher_carte(
            nom_carte=nom if nom else None, code_set=code if code else None
        )
        if not resultats:
            ttk.Label(self.cadre_resultats, text="Aucun résultat.").pack()
        else:
            for _, _, _, _, page in resultats:
                ttk.Label(self.cadre_resultats, text=f"Page : {page}").pack(anchor="w")

    def rafraichir_classeurs(self):
        from .gestionnaire_classeur import obtenir_liste_classeurs

        """Actualise la liste des classeurs et met à jour l'interface."""
        classeurs = obtenir_liste_classeurs()
        self.combo_classeurs["values"] = classeurs
        current = self.classeur_selectionne.get()

        # Force le rafraîchissement de la combobox
        self.combo_classeurs.set("")
        self.combo_classeurs.update()

        if not classeurs:
            self._reset_interface()
            return

        # Si le classeur actuel existe toujours, le recharger
        if current in classeurs:
            self.classeur_selectionne.set(current)
            self.charger_les_cartes()
        else:
            self._reset_interface()

    def _reset_interface(self):
        """Réinitialise l'interface quand aucun classeur n'est sélectionné."""
        self.classeur_selectionne.set("")
        if self.titre_classeur and self.titre_classeur.winfo_exists():
            self.titre_classeur.configure(text="Aucun classeur sélectionné")
        for widget in self.scrollable_frame.winfo_children():
            widget.destroy()
        self.images_cache.clear()
        if self.lbl_page and self.lbl_page.winfo_exists():
            self.lbl_page.configure(text="Aucun classeur sélectionné")
        if self.bouton_full_set and self.bouton_full_set.winfo_exists():
            self.bouton_full_set.configure(state="disabled")
        if self.bouton_unset and self.bouton_unset.winfo_exists():
            self.bouton_unset.configure(state="disabled")


# Déplace la liste de référence d'image en dehors de la fonction pour éviter l'avertissement Pylance
_full_image_refs = []

def open_full_image(path, nom):
    """Affiche une image en grand dans une nouvelle fenêtre Tkinter."""

    if not os.path.exists(path):
        if os.path.exists(DEFAULT_IMAGE_PATH):
            path = DEFAULT_IMAGE_PATH
        else:
            messagebox.showerror("Erreur", "Image introuvable et image de remplacement manquante.")
            return

    top = tk.Toplevel()
    top.title(nom)
    pil_full = Image.open(path)
    img_full = ImageTk.PhotoImage(pil_full)
    label_full = tk.Label(top, image=img_full)
    label_full.pack()
    # Empêche le garbage collector de supprimer l'image
    _full_image_refs.append(img_full)


# Ajouter la fonction à __all__ pour l'exporter explicitement
__all__ = ["VisualiseurClasseur", "afficher_cartes_interface", "open_full_image"]
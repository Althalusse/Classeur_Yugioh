import os
import sqlite3
import json
import shutil
import pandas as pd
import stat
import tkinter as tk
from module.BDD_creation import DB_FILE
from module.centralisation_dossier import CLASSEUR_FOLDER, BDD_FOLDER, CARDINFO_DB 
from tkinter import simpledialog, messagebox, ttk
from module.img_dl import gui_telechargement
from module.img_dl.telechargement_service import TelechargementService
from module.creation_classeur.creation_classeur_service import create_classeur, remove_readonly

class FenetreSuppressionClasseur(tk.Toplevel):
    def __init__(self, master=None):
        super().__init__(master)
        self.title("Suppression de classeurs")
        self.geometry("400x400")
        self.configure(bg="#f0f0f0")

        # Liste des fichiers .db
        self.listbox = tk.Listbox(self, selectmode=tk.EXTENDED, exportselection=False, height=15)
        self.listbox.pack(padx=10, pady=10, fill=tk.BOTH, expand=True)

        # Ajout du raccourci clavier Ctrl+A
        self.listbox.bind('<Control-a>', self.select_all)

        self.remplir_listbox()

        # Bouton de suppression
        btn_supprimer = tk.Button(self, text="Supprimer", command=self.supprimer_classeur, bg="#ff6666", fg="white")
        btn_supprimer.pack(pady=10)

    def remplir_listbox(self):
        self.listbox.delete(0, tk.END)
        for fichier in os.listdir(CLASSEUR_FOLDER):
            if fichier.endswith(".db"):
                nom_classeur = fichier.replace(".db", "")
                self.listbox.insert(tk.END, nom_classeur)

    def supprimer_classeur(self):
        selection = self.listbox.curselection()
        if not selection:
            messagebox.showwarning("Aucune sélection", "Veuillez sélectionner au moins un classeur à supprimer.")
            return

        confirm = messagebox.askyesno("Confirmation", "Êtes-vous sûr de vouloir supprimer le(s) classeur(s) sélectionné(s) ?")
        if not confirm:
            return

        for index in reversed(selection):
            nom_classeur = self.listbox.get(index)
            chemin_classeur = os.path.join(CLASSEUR_FOLDER, f"{nom_classeur}.db")
            if os.path.exists(chemin_classeur):
                os.remove(chemin_classeur)
            self.listbox.delete(index)

    def select_all(self, event=None):
        self.listbox.select_set(0, tk.END)
        return "break"

# Fonction pour créer un nouveau classeur
# Supprimée car déplacée vers le service

# Fonction pour supprimer un classeur
# Supprimée car déplacée vers le service

def main_menu(parent, refresh_callback=None):
    def creer():
        # Récupère les codes de set uniques depuis la BDD principale
        def get_unique_set_codes_and_names(db_path):
            conn = sqlite3.connect(db_path)
            cursor = conn.cursor()
            cursor.execute("""
                SELECT card_sets_set_code, card_sets_set_name
                FROM cards
                WHERE card_sets_set_code IS NOT NULL
            """)
            codes = {}
            for code, name in cursor.fetchall():
                if "-" in code:
                    prefix = code.split("-")[0]
                    if prefix not in codes:
                        codes[prefix] = {"name": name, "count": 1}
                    else:
                        codes[prefix]["count"] += 1
            conn.close()
            return [
                (prefix, f"{prefix} ({codes[prefix]['name']}) [{codes[prefix]['count']}]")
                for prefix in sorted(codes)
            ]

        set_codes = get_unique_set_codes_and_names(DB_FILE)
        options = [display for code, display in set_codes]
        prefixes = [code for code, display in set_codes]

        window = tk.Toplevel()
        window.title("Création d'un classeur")
        window.geometry("800x600")  # Fenêtre plus grande
        window.resizable(False, False)  # Taille fixe

        # Frame principal avec padding
        main_frame = ttk.Frame(window, padding="20")
        main_frame.pack(fill=tk.BOTH, expand=True)

        # Titre avec icône
        title_frame = ttk.Frame(main_frame)
        title_frame.pack(fill="x", pady=(0, 20))
        ttk.Label(
            title_frame,
            text="✨ Création d'un nouveau classeur",
            font=("Helvetica", 16, "bold"),
            style="Title.TLabel"
        ).pack()

        # Description
        ttk.Label(
            main_frame,
            text="Recherchez ou sélectionnez un code set dans la liste ci-dessous :",
            font=("Helvetica", 10),
            wraplength=550
        ).pack(pady=(0, 10))

        # Frame pour la recherche
        search_frame = ttk.LabelFrame(main_frame, text="Recherche", padding="10")
        search_frame.pack(fill="x", pady=(0, 20))

        # Combobox améliorée
        selected_value = tk.StringVar()
        combobox = ttk.Combobox(
            search_frame, 
            textvariable=selected_value,
            values=options,
            font=("Helvetica", 12),
            width=50
        )
        combobox.pack(pady=10)

        # Liste des résultats avec scrollbar
        result_frame = ttk.LabelFrame(main_frame, text="Résultats", padding="10")
        result_frame.pack(fill="both", expand=True)
        
        tree = ttk.Treeview(
            result_frame,
            columns=("code", "name", "count"),
            show="headings",
            height=8,
            selectmode="browse"  # Mode de sélection unique
        )
        tree.heading("code", text="Code")
        tree.heading("name", text="Nom du set")
        tree.heading("count", text="Cartes")
        
        tree.column("code", width=100)
        tree.column("name", width=300)
        tree.column("count", width=100)
        scrollbar = ttk.Scrollbar(result_frame, orient="vertical", command=tree.yview)
        tree.configure(yscrollcommand=scrollbar.set)
        
        tree.pack(side="left", fill="both", expand=True)
        scrollbar.pack(side="right", fill="y")

        # Frame pour la progression
        progress_frame = ttk.LabelFrame(main_frame, text="Progression", padding="10")
        progress_frame.pack(fill="x", pady=(10, 0))
        
        progress_label = ttk.Label(progress_frame, text="En attente...", font=("Helvetica", 9))
        progress_label.pack(fill="x")
        
        progress_bar = ttk.Progressbar(
            progress_frame,
            orient="horizontal",
            mode="determinate"
        )
        progress_bar.pack(fill="x", pady=(5, 0))

        # Cacher initialement la frame de progression
        progress_frame.pack_forget()

        def update_progress(current, total, message=""):
            progress_frame.pack(fill="x", pady=(10, 0))
            progress_bar["value"] = (current / total) * 100
            progress_label.config(text=message)
            window.update_idletasks()

        def update_results(event=None):
            tree.delete(*tree.get_children())
            search_term = combobox.get().upper()
            for code, display in set_codes:
                if search_term in code:
                    try:
                        # Extraire le nom et le nombre de cartes du display
                        name = display.split(" (")[1].split(") [")[0]
                        count = display.split("[")[1].split("]")[0]
                        tree.insert("", "end", values=(code, name, count))
                    except IndexError:
                        # Ignorer les entrées mal formatées
                        continue

        # Associer update_results à la combobox
        combobox.bind('<KeyRelease>', update_results)

        def on_double_click(event):
            selection = tree.selection()
            if selection:
                item = tree.item(selection[0])
                code = item['values'][0]  # Code est dans la première colonne
                creer_classeur(code)

        def creer_classeur(code):
            try:
                code = str(code)
                download_complete = False
                
                progress_frame.pack(fill="x", pady=(10, 0))
                progress_label.config(text="Création du classeur...")
                progress_bar["value"] = 0
                window.update_idletasks()

                if create_classeur(code):
                    # Rafraîchir immédiatement après la création du classeur
                    update_count()
                    refresh_classeur_list()
                    if refresh_callback:
                        refresh_callback()
                        
                    progress_bar["value"] = 33
                    progress_label.config(text="Initialisation du téléchargement...")
                    window.update_idletasks()

                    # Variable pour suivre si on doit annuler le processus complet
                    cancel_creation = False

                    def on_window_close():
                        nonlocal cancel_creation
                        if not download_complete:
                            if messagebox.askyesno("Confirmation", 
                                "Le téléchargement des images est en cours.\n"
                                "Voulez-vous annuler la création complète du classeur ?"):
                                cancel_creation = True
                                # Supprimer le classeur créé
                                try:
                                    classeur_path = os.path.join(CLASSEUR_FOLDER, code)
                                    if os.path.exists(classeur_path):
                                        shutil.rmtree(classeur_path, onerror=remove_readonly)
                                except Exception:
                                    pass
                                window.destroy()
                            return
                        window.destroy()

                    # Lier l'événement de fermeture de la fenêtre
                    window.protocol("WM_DELETE_WINDOW", on_window_close)

                    def on_download_complete():
                        if not cancel_creation:
                            nonlocal download_complete
                            download_complete = True
                            progress_bar["value"] = 100
                            progress_label.config(text="✅ Téléchargement terminé!")
                            window.update_idletasks()
                            messagebox.showinfo("Succès", f"Le classeur '{code}' a été créé avec succès.")

                    def download_images():
                        try:
                            progress_bar["value"] = 40
                            progress_label.config(text="Téléchargement des images en cours...")
                            window.update_idletasks()
                            
                            def progress_update(current, total, message=""):
                                try:
                                    # Vérifier si la fenêtre existe toujours
                                    if progress_bar.winfo_exists():
                                        if total == 0:
                                            progress = 100
                                        else:
                                            progress = 40 + (current / max(total, 1)) * 50
                                        progress_bar["value"] = progress
                                        progress_label.config(text=message if message else f"Téléchargement : {current}/{total}")
                                        window.update_idletasks()
                                except Exception:
                                    pass

                            # Correction ici : instanciation de TelechargementGUI
                            service = TelechargementService()
                            manager = None  # Si vous avez un manager, passez-le ici, sinon None
                            tele_gui = gui_telechargement.TelechargementGUI(service, manager)
                            tele_gui.telecharger_images_gui(
                                code,
                                progress_callback=progress_update,
                                on_complete=on_download_complete
                            )
                        except Exception as e:
                            messagebox.showerror("Erreur", f"Erreur lors du téléchargement : {e}")
                            progress_frame.pack_forget()
                            progress_bar["value"] = 0
                            progress_label.config(text="En attente...")

                    window.after(100, download_images)
                else:
                    # Ne pas lever d'exception, afficher juste le message d'erreur
                    messagebox.showerror("Erreur", "Ce classeur existe déjà.")
                    # Réinitialiser l'interface
                    progress_frame.pack_forget()
                    progress_bar["value"] = 0
                    progress_label.config(text="En attente...")
                    return

            except Exception as e:
                messagebox.showerror("Erreur", f"Une erreur s'est produite : {e}")
                # Ne pas fermer la fenêtre en cas d'erreur
                progress_frame.pack_forget()
                progress_bar["value"] = 0
                progress_label.config(text="En attente...")

        # Bind le double-clic
        tree.bind('<Double-1>', on_double_click)

        # Frame pour les boutons
        button_frame = ttk.Frame(main_frame)
        button_frame.pack(fill="x", pady=(20, 0))

        # Boutons
        ttk.Button(
            button_frame,
            text="Annuler",
            command=window.destroy,
            style="Secondary.TButton",
            width=15
        ).pack(side="right", padx=5)

        def valider():
            selection = tree.selection()
            if selection:
                item = tree.item(selection[0])
                code = item['values'][0]
                creer_classeur(code)
            else:
                messagebox.showwarning("Attention", "Veuillez sélectionner un set dans la liste")

        ttk.Button(
            button_frame,
            text="✨ Valider",
            command=valider,
            style="Primary.TButton",
            width=15
        ).pack(side="right", padx=5)

        def creer():
            code_set = combobox.get().strip()
            if not code_set:
                messagebox.showwarning("Entrée invalide", "Veuillez choisir ou saisir un code set.")
                return
            # Si l'utilisateur a choisi dans la liste, extraire le code
            if " " in code_set:
                code_set = code_set.split(" ")[0]
            try:
                create_classeur(code_set)
                messagebox.showinfo("Succès", f"Le classeur '{code_set}' a été créé avec succès.")
                window.destroy()
                if refresh_callback:
                    refresh_callback()  # <-- AJOUTE CETTE LIGNE
            except Exception as e:
                messagebox.showerror("Erreur", f"Une erreur s'est produite : {e}")

        # Mise à jour initiale des résultats
        update_results()
        
        # Centre la fenêtre
        window.transient(parent)
        window.grab_set()
        window.focus_set()
        
    def supprimer():
        selections = tree.selection()
        if not selections:
            messagebox.showwarning("Avertissement", "Veuillez sélectionner un ou plusieurs classeurs à supprimer.")
            return
            
        nb_classeurs = len(selections)
        classeurs = []
        for item in selections:
            code_set = str(tree.item(item)['values'][0])
            classeurs.append(code_set)
        
        # Message de confirmation adapté
        if nb_classeurs == 1:
            message = f"Voulez-vous vraiment supprimer le classeur '{classeurs[0]}' ?"
        else:
            message = f"Voulez-vous vraiment supprimer ces {nb_classeurs} classeurs ?\n\n"
            message += "\n".join(f"• {code}" for code in classeurs[:5])
            if nb_classeurs > 5:
                message += f"\n... et {nb_classeurs - 5} autres"
        
        confirmation = messagebox.askyesno("Confirmation", message)
        if not confirmation:
            return

        # Suppression des classeurs sélectionnés
        erreurs = []
        for code_set in classeurs:
            try:
                classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)
                shutil.rmtree(classeur_path, onerror=remove_readonly)
            except Exception as e:
                erreurs.append(f"Erreur lors de la suppression de '{code_set}': {e}")

        # Affichage des erreurs s'il y en a
        if erreurs:
            messagebox.showerror("Erreur", "\n".join(erreurs))
        else:
            if nb_classeurs == 1:
                messagebox.showinfo("Succès", "Le classeur a été supprimé.")
            else:
                messagebox.showinfo("Succès", f"Les {nb_classeurs} classeurs ont été supprimés.")

        # Mise à jour de l'interface
        update_count()
        refresh_classeur_list()
        if refresh_callback:
            refresh_callback()

    # Clear le contenu du parent
    for widget in parent.winfo_children():
        widget.destroy()

    # Layout principal avec sidebar
    main_panel = ttk.PanedWindow(parent, orient=tk.HORIZONTAL)
    main_panel.pack(fill=tk.BOTH, expand=True)

    # Sidebar pour les actions
    sidebar = ttk.Frame(main_panel, width=220)
    main_panel.add(sidebar, weight=0)
    sidebar.pack_propagate(False)

    # Zone principale pour la liste des classeurs
    content = ttk.Frame(main_panel)
    main_panel.add(content, weight=1)

    # Titre de la sidebar
    ttk.Label(sidebar, text="Actions disponibles", font=("Arial", 11, "bold")).pack(pady=(10, 5))
    
    # Zone des actions
    action_frame = ttk.LabelFrame(sidebar, text="Gestion des classeurs")
    action_frame.pack(fill="x", padx=10, pady=5, ipady=5)

    # Boutons d'action
    create_btn = ttk.Button(
        action_frame,
        text="✨ Créer un nouveau classeur",
        command=creer,
        style='Action.TButton',
        width=25
    )
    create_btn.pack(pady=5, padx=5)

    # Nouveau bouton pour classeur personnalisé
    def creer_personnalise():
        nom = simpledialog.askstring("Nom du classeur", "Entrez le nom du nouveau classeur personnalisé :")
        if not nom:
            return
        nom = nom.strip()
        if not nom:
            messagebox.showwarning("Nom invalide", "Le nom ne peut pas être vide.")
            return
        # Dossier du classeur
        dossier_classeur = os.path.join(CLASSEUR_FOLDER, nom)
        db_path = os.path.join(dossier_classeur, f"{nom}.db")
        if os.path.exists(db_path):
            messagebox.showerror("Erreur", f"Un classeur nommé '{nom}' existe déjà.")
            return
        try:
            os.makedirs(dossier_classeur, exist_ok=True)
            conn = sqlite3.connect(db_path)
            # Table des cartes avec la structure demandée
            conn.execute("""
                CREATE TABLE IF NOT EXISTS cards (
                    "id" INTEGER,
                    "name" TEXT,
                    "type" TEXT,
                    "humanReadableCardType" TEXT,
                    "frameType" TEXT,
                    "description" TEXT,
                    "race" TEXT,
                    "ygoprodeck_url" TEXT,
                    "typeline" TEXT,
                    "atk" TEXT,
                    "def" TEXT,
                    "level" TEXT,
                    "attribute" TEXT,
                    "pend_desc" TEXT,
                    "monster_desc" TEXT,
                    "archetype" TEXT,
                    "scale" TEXT,
                    "linkval" TEXT,
                    "linkmarkers" TEXT,
                    "card_sets_set_name" TEXT,
                    "card_sets_set_code" TEXT,
                    "card_sets_set_rarity" TEXT,
                    "card_sets_set_rarity_code" TEXT,
                    "card_sets_set_price" TEXT,
                    "card_images_id" INTEGER,
                    "card_images_image_url" TEXT,
                    "card_images_image_url_small" TEXT,
                    "card_images_image_url_cropped" TEXT,
                    "card_prices_cardmarket_price" TEXT,
                    "card_prices_tcgplayer_price" TEXT,
                    "card_prices_ebay_price" TEXT,
                    "card_prices_amazon_price" TEXT,
                    "card_prices_coolstuffinc_price" TEXT,
                    "banlist_info_ban_tcg" TEXT,
                    "banlist_info_ban_ocg" TEXT,
                    "banlist_info_ban_goat" TEXT,
                    "set_code_prefix" TEXT,
                    "cardmarket_url" TEXT,     
                    "quantite" TEXT,
                    "qualite" TEXT,
                    "is_custom" TEXT,
                    "possessed" TEXT
                )
            """)
            # Table meta pour indiquer que c'est un classeur personnalisé
            conn.execute("""
                CREATE TABLE IF NOT EXISTS meta (
                    key TEXT PRIMARY KEY,
                    value TEXT
                )
            """)
            conn.execute("""
                INSERT INTO meta (key, value) VALUES ('classeur_personnaliser', '1')
            """)
            conn.commit()
            conn.close()
            messagebox.showinfo("Succès", f"Le classeur '{nom}' a été créé avec succès.")
            refresh_classeur_list()
            if refresh_callback:
                refresh_callback()
        except Exception as e:
            messagebox.showerror("Erreur", f"Impossible de créer le classeur : {e}")

    personalize_btn = ttk.Button(
        action_frame,
        text="📝 Créer un classeur personnalisé",
        command=creer_personnalise,
        style='Action.TButton',
        width=25
    )
    personalize_btn.pack(pady=5, padx=5)

    delete_btn = ttk.Button(
        action_frame,
        text="🗑️ Supprimer un classeur",
        command=supprimer,
        style='Delete.TButton',
        width=25
    )
    delete_btn.pack(pady=5, padx=5)

    # Zone d'information dans la sidebar
    info_frame = ttk.LabelFrame(sidebar, text="Informations")
    info_frame.pack(fill="x", padx=10, pady=5)
    
    classeur_count = len([d for d in os.listdir(CLASSEUR_FOLDER) if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))])
    ttk.Label(
        info_frame,
        text=f"Nombre de classeurs : {classeur_count}",
        font=('Helvetica', 10)
    ).pack(pady=5, padx=5)

    def update_count():
        classeur_count = len([d for d in os.listdir(CLASSEUR_FOLDER) if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))])
        for widget in info_frame.winfo_children():
            widget.destroy()
        ttk.Label(
            info_frame,
            text=f"Nombre de classeurs : {classeur_count}",
            font=('Helvetica', 10)
        ).pack(pady=5, padx=5)

    # Treeview dans la zone principale avec sélection activée
    ttk.Label(content, text="Liste des classeurs", font=("Arial", 12, "bold")).pack(pady=10)
    
    tree = ttk.Treeview(content, columns=("name", "cards", "date"), show="headings", selectmode="extended")
    tree.heading("name", text="Nom du classeur")
    tree.heading("cards", text="Nombre de cartes")
    tree.heading("date", text="Date de création")
    tree.bind('<Control-a>', lambda event: tree.selection_set(tree.get_children()))
    tree.pack(fill=tk.BOTH, expand=True, padx=10)

    # Ajouter une scrollbar verticale
    vsb = ttk.Scrollbar(content, orient="vertical", command=tree.yview)
    vsb.pack(side='right', fill='y')
    tree.configure(yscrollcommand=vsb.set)

    def refresh_classeur_list():
        tree.delete(*tree.get_children())
        # Mise à jour du compteur de classeurs
        classeur_count = len([d for d in os.listdir(CLASSEUR_FOLDER) if os.path.isdir(os.path.join(CLASSEUR_FOLDER, d))])
        for widget in info_frame.winfo_children():
            widget.destroy()
        ttk.Label(
            info_frame,
            text=f"Nombre de classeurs : {classeur_count}",
            font=('Helvetica', 10)
        ).pack(pady=5, padx=5)
        
        # Mise à jour de la liste des classeurs
        for classeur in os.listdir(CLASSEUR_FOLDER):
            if os.path.isdir(os.path.join(CLASSEUR_FOLDER, classeur)):
                # Compter les cartes dans la base de données
                db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
                card_count = 0
                if os.path.exists(db_path):
                    try:
                        conn = sqlite3.connect(db_path)
                        cursor = conn.cursor()
                        cursor.execute("SELECT COUNT(*) FROM cards")
                        card_count = cursor.fetchone()[0]
                        conn.close()
                    except:
                        pass
                # Date de création
                date_creation = os.path.getctime(os.path.join(CLASSEUR_FOLDER, classeur))
                date_str = pd.Timestamp(date_creation, unit='s').strftime('%Y-%m-%d %H:%M')
                tree.insert("", "end", values=(classeur, card_count, date_str))

    refresh_classeur_list()
    
    return main_panel


# ...rest of existing code...

import tkinter as tk
from tkinter import ttk, messagebox
import sys
import os
import json

from datetime import datetime
sys.path.append(os.path.abspath(os.path.dirname(__file__)))

# Ajoute le dossier 'module' à sys.path si gelé (exécutable)
if getattr(sys, 'frozen', False):
    # Utilise le dossier où se trouve le .exe, pas le dossier temporaire
    exe_dir = os.path.abspath(os.path.dirname(sys.argv[0]))
    module_path = os.path.join(exe_dir, 'module')
else:
    module_path = os.path.join(os.path.dirname(__file__), 'module')
if module_path not in sys.path:
    sys.path.insert(0, module_path)

from module.Affichage_classeur.visualiseur_classeur_ui import afficher_cartes_interface

from module import BDD_creation
from module.version import controle_version_database_api, controle_version_database_ui

#module qui permet de gérer le telechargement des images des cartes dans les classeurs créer
from module.img_dl import gui_telechargement 

#permet la sauvegarde et la restauration des cartes personnalisées
from module.sauvegarde_carte_custom.Savedata_Backup import create_backup, restore_backup, get_available_backups,delete_backup

#Module qui gere la partie statistique de la collectio
from module.statistique import statistique_collection
from module.statistique.statistique_collection import afficher_stats_interface

#permet la création et la suppression de classeurs
from module.creation_classeur import creation_suppression_classeur

#Gestion rareté
from module.gestion_rarete import gestion_rarete

#ajout_carte.py
from module.ajout_carte import ajout_carte  # Nouvel import

#inventaire_carte.py
from module.inventaire import inventaire_carte
from module.inventaire.inventaire_carte_UI import creer_interface_inventaire


#rafraichissement.py
from module.utilitaire.actualisation_UI import creer_callback_rafraichir, rafraichir_si_onglet, ajouter_bouton_rafraichir, rafraichissement_periodique, setup_rafraichissement_inventaire

#Configurations
from module.centralisation_dossier import FIRST_RUN_FILE
from module.gui_style import ApplicationStyle


def afficher_inventaire_carte(frame):
    """
    Affiche l'interface d'inventaire des cartes.
    """
    # Efface le contenu précédent
    for widget in frame.winfo_children():
        widget.destroy()
    # Crée l'interface d'inventaire via la fonction du module UI
    creer_interface_inventaire(frame)
    # Pas de retour ni de gestion de refresh ici

def creer_onglet_inventaire(notebook):
    """Crée et ajoute l'onglet Inventaire Carte au notebook principal."""
    tab_inventaire_carte = ttk.Frame(notebook)
    notebook.add(tab_inventaire_carte, text="Inventaire Carte")
    afficher_inventaire_carte(tab_inventaire_carte)
    return tab_inventaire_carte


def first_run_script():
    # Initialiser les dossiers avant la création de la BDD
    # Lance la création de la base de données principale
    BDD_creation.main()

def check_first_run():
    if not os.path.exists(FIRST_RUN_FILE):
        first_run_script()
        with open(FIRST_RUN_FILE, "w") as f:
            f.write("initialized")

def afficher_statistiques(cadre):
    # Efface le contenu précédent
    for widget in cadre.winfo_children():
        widget.destroy()
    # Appel à l'interface graphique de statistiques centralisée dans le module statistique_collection
    afficher_stats_interface(cadre)

def afficher_erreur(message, titre="Erreur"):
    messagebox.showerror(titre, message)

def afficher_info(message, titre="Info"):
    messagebox.showinfo(titre, message)

def afficher_warning(message, titre="Attention"):
    messagebox.showwarning(titre, message)

def creer_cadre_sauvegarde_restauration(notebook):
    """Crée l'onglet de sauvegarde et restauration"""
    cadre = ttk.Frame(notebook)
    cadre_sauvegarde = ttk.LabelFrame(cadre, text="Sauvegarde")
    cadre_sauvegarde.pack(fill="x", padx=10, pady=5)
    def faire_sauvegarde():
        try:
            fichier_sauvegarde = create_backup()
            afficher_info(f"Sauvegarde créée avec succès!\nFichier: {os.path.basename(fichier_sauvegarde)}", "Succès")
        except Exception as e:
            afficher_erreur(str(e))
    ttk.Button(cadre_sauvegarde, text="Créer une sauvegarde", 
               command=faire_sauvegarde).pack(padx=10, pady=5)
    cadre_restauration = ttk.LabelFrame(cadre, text="Restauration")
    cadre_restauration.pack(fill="x", padx=10, pady=5)
    def mettre_a_jour_liste_sauvegardes():
        sauvegardes = get_available_backups()
        listebox_sauvegardes.delete(0, tk.END)
        for sauvegarde in sauvegardes:
            date_str = datetime.strptime(sauvegarde["timestamp"], 
                "%Y%m%d_%H%M%S").strftime("%d/%m/%Y %H:%M:%S")
            listebox_sauvegardes.insert(tk.END, date_str)
    listebox_sauvegardes = tk.Listbox(cadre_restauration, height=5)
    listebox_sauvegardes.pack(fill="x", padx=10, pady=5)
    def faire_restauration():
        selection = listebox_sauvegardes.curselection()
        if not selection:
            afficher_warning("Veuillez sélectionner une sauvegarde à restaurer")
            return
        sauvegardes = get_available_backups()
        sauvegarde_selectionnee = sauvegardes[selection[0]]
        if messagebox.askyesno("Confirmation", 
            "Êtes-vous sûr de vouloir restaurer cette sauvegarde ?\n"
            "Les cartes personnalisées actuelles seront remplacées."):
            try:
                restore_backup(sauvegarde_selectionnee["path"])
                afficher_info("Restauration effectuée avec succès!", "Succès")
            except Exception as e:
                afficher_erreur(str(e))
    def faire_suppression():
        selection = listebox_sauvegardes.curselection()
        if not selection:
            afficher_warning("Veuillez sélectionner une sauvegarde à supprimer")
            return
        sauvegardes = get_available_backups()
        sauvegarde_selectionnee = sauvegardes[selection[0]]
        if messagebox.askyesno("Confirmation", "Voulez-vous vraiment supprimer cette sauvegarde ? Cette action est irréversible."):
            try:
                delete_backup(sauvegarde_selectionnee["path"])
                afficher_info("Sauvegarde supprimée avec succès.", "Succès")
                mettre_a_jour_liste_sauvegardes()
            except Exception as e:
                afficher_erreur(str(e))
    cadre_boutons = ttk.Frame(cadre_restauration)
    cadre_boutons.pack(fill="x", padx=10, pady=5)
    ttk.Button(cadre_boutons, text="Rafraîchir la liste", 
               command=mettre_a_jour_liste_sauvegardes).pack(side="left", padx=5)
    ttk.Button(cadre_boutons, text="Restaurer la sauvegarde", 
               command=faire_restauration).pack(side="left", padx=5)
    ttk.Button(cadre_boutons, text="Supprimer la sauvegarde", 
               command=faire_suppression).pack(side="left", padx=5)
    def voir_classeurs():
        selection = listebox_sauvegardes.curselection()
        if not selection:
            afficher_warning("Veuillez sélectionner une sauvegarde")
            return
        sauvegardes = get_available_backups()
        sauvegarde_selectionnee = sauvegardes[selection[0]]
        try:
            with open(sauvegarde_selectionnee["path"], "r", encoding="utf-8") as f:
                data = json.load(f)
            classeurs = data.get("classeurs", [])
            if not classeurs:
                afficher_info("Aucun classeur présent dans cette sauvegarde.", "Classeurs")
            else:
                afficher_info("\n".join(classeurs), "Classeurs dans la sauvegarde")
        except Exception as e:
            afficher_erreur(str(e), "Erreur")
    ttk.Button(cadre_boutons, text="Voir les classeurs", command=voir_classeurs).pack(side="left", padx=5)
    mettre_a_jour_liste_sauvegardes()
    return cadre

def creer_onglet_statistiques(notebook):
    """Crée et ajoute l'onglet Statistiques au notebook principal."""
    tab_stats = ttk.Frame(notebook)
    notebook.add(tab_stats, text="Statistiques")
    afficher_statistiques(tab_stats)
    return tab_stats


def creer_onglet_simulateur_classeur(notebook, afficher_statistiques_callback):
    """Crée et ajoute l'onglet Simulateur Classeur avec ses sous-onglets."""
    tab_simulateur = ttk.Frame(notebook)
    notebook.add(tab_simulateur, text="Simulateur Classeur")
    sous_notebook = ttk.Notebook(tab_simulateur)
    sous_notebook.pack(expand=True, fill="both")

    # Onglet Gestion Classeur
    tab_gestion = ttk.Frame(sous_notebook)
    sous_notebook.add(tab_gestion, text="Gestion Classeur")

    # Sous onglet Afficher les Classeurs
    tab_afficher = ttk.Frame(sous_notebook)
    sous_notebook.add(tab_afficher, text="Afficher Classeurs")
    menu_frame = ttk.Frame(tab_afficher)
    menu_frame.pack(side="left", fill="y", padx=5, pady=5)
    ttk.Separator(menu_frame, orient="horizontal").pack(fill="x", padx=5, pady=5)
    main_frame = ttk.Frame(tab_afficher)
    main_frame.pack(side="right", fill="both", expand=True, padx=5, pady=5)
    rafraichir_classeurs = afficher_cartes_interface(main_frame, callback_rafraichir_stats=afficher_statistiques_callback)
    def rafraichir_classeurs_wrapper():
        rafraichir_classeurs()
        afficher_statistiques_callback()
    creation_suppression_classeur.main_menu(tab_gestion, refresh_callback=rafraichir_classeurs_wrapper)

    # Sous-onglet Ajouter carte
    tab_ajouter_carte = ttk.Frame(sous_notebook)
    sous_notebook.add(tab_ajouter_carte, text="Ajouter carte")
    ajout_frame = ajout_carte.AjoutCarteFrame(tab_ajouter_carte, refresh_callback=lambda: rafraichir_classeurs())

    # Sous-onglet Sauvegarde/Restoration
    tab_backup = ttk.Frame(sous_notebook)
    sous_notebook.add(tab_backup, text="Sauvegarde/Restoration")
    frame_backup = creer_cadre_sauvegarde_restauration(tab_backup)
    frame_backup.pack(fill="both", expand=True)

    # Retourne les frames utiles pour les callbacks
    return {
        "tab_simulateur": tab_simulateur,
        "sous_notebook": sous_notebook,
        "tab_afficher": tab_afficher,
        "rafraichir_classeurs": rafraichir_classeurs,
        "ajout_frame": ajout_frame
    }

def creer_onglet_options(notebook):
    """Crée et ajoute l'onglet Options au notebook principal."""
    tab_options = ttk.Frame(notebook)
    notebook.add(tab_options, text="Options")
    rarity_frame = gestion_rarete.RarityPriorityFrame(tab_options)
    rarity_frame.pack(fill="both", expand=True, padx=10, pady=10)
    return tab_options

def creer_onglet_version(notebook):
    """Crée et ajoute l'onglet Version au notebook principal."""
    tab_version = ttk.Frame(notebook)
    notebook.add(tab_version, text="Version")

    frame = ttk.Frame(tab_version)
    frame.pack(fill="both", expand=True, padx=20, pady=20)

    # Affichage des infos de version
    label_info = ttk.Label(frame, text="Vérification de la version de la base de données...", font=("Arial", 12))
    label_info.pack(pady=10)

    def afficher_infos_version():
        update_available, infos = controle_version_database_api.check_for_updates()
        if 'error' in infos:
            label_info.config(text=f"Erreur : {infos['error']}", foreground="red")
        else:
            txt = f"Version locale : {infos['local']}\nVersion distante : {infos['remote']}"
            if update_available:
                txt += "\n\nUne mise à jour est disponible."
                label_info.config(foreground="orange")
            else:
                txt += "\n\nVotre base de données est à jour."
                label_info.config(foreground="green")
            label_info.config(text=txt)

    btn_verifier = ttk.Button(frame, text="Vérifier la version", command=afficher_infos_version)
    btn_verifier.pack(pady=5)

    def lancer_mise_a_jour():
        # Utilise la fonction UI déplacée
        res = controle_version_database_ui.check_updates_ui()
        afficher_infos_version()
        if res:
            messagebox.showinfo("Mise à jour", "Base de données mise à jour avec succès.")

    btn_maj = ttk.Button(frame, text="Mettre à jour la base de données", command=lancer_mise_a_jour)
    btn_maj.pack(pady=5)

    afficher_infos_version()
    return tab_version


def main():
    # Initialiser les dossiers au démarrage
    check_first_run()
    root = tk.Tk()
    root.title("Gestion Yu-Gi-Oh! - Application")
    root.geometry("1590x1370")
    
    # Appliquer le thème
    ApplicationStyle.apply_theme(root)
    
    # Création d'un cadre principal avec padding
    main_frame = ttk.Frame(root, padding="10", style="Card.TFrame")
    main_frame.pack(fill=tk.BOTH, expand=True, padx=10, pady=10)
    
    # Titre de l'application
    title_frame = ttk.Frame(main_frame)
    title_frame.pack(fill="x", pady=(0, 10))
    ttk.Label(
        title_frame,
        text="Gestionnaire de Collection Yu-Gi-Oh!",
        style="Title.TLabel"
    ).pack()
    
    notebook = ttk.Notebook(main_frame)
    notebook.pack(expand=True, fill="both")

    # Onglet Statistiques
    tab_stats = creer_onglet_statistiques(notebook)

    # Onglet Simulateur Classeur avec sous-onglets
    onglets_simulateur = creer_onglet_simulateur_classeur(
        notebook, afficher_statistiques_callback=lambda: afficher_statistiques(tab_stats))
    rafraichir_classeurs = onglets_simulateur["rafraichir_classeurs"]
    ajout_frame = onglets_simulateur["ajout_frame"]

   # Onglet Inventaire Carte
    creer_onglet_inventaire(notebook)

    # Onglet Options
    creer_onglet_options(notebook)

    # Onglet Version (NOUVEAU)
    creer_onglet_version(notebook)

    # Utilisation du module rafraichissement pour le changement d'onglet
    mapping = {
        "Afficher Classeurs": rafraichir_classeurs,
        # Correction : AjoutCarteFrame n'a pas de méthode refresh_classeurs, utiliser refresh_custom_cards à la place
        "Ajouter carte": ajout_frame.refresh_custom_cards
    }
    notebook.bind("<<NotebookTabChanged>>", lambda event: rafraichir_si_onglet(event, mapping))

    root.protocol("WM_DELETE_WINDOW", lambda: on_close_app(root))
    root.mainloop()

def on_close_app(root):
    # Exemple : appeler une fonction de sauvegarde si besoin
    try:
        # Si tu as des objets/fonctions à appeler pour sauvegarder l'état, fais-le ici
        # Par exemple : gestion_rarete.save_config_if_needed()
        pass
    except Exception as e:
        print(f"Erreur lors de la sauvegarde : {e}")

    # Fermer toutes les connexions SQLite ouvertes (sécurité)
    try:
        import sqlite3
        sqlite3.Connection.close  # Cette ligne ne ferme rien, c'est juste pour rappel
        # Si tu utilises des connexions globales, ferme-les ici
    except Exception:
        pass

    # Quitter proprement l'application
    root.destroy()
    sys.exit(0)

if __name__ == "__main__":
    main()

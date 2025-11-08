import requests
import pandas as pd
import sqlite3
import json
import os
from datetime import datetime
from module.centralisation_dossier import get_paths
import threading
import tkinter as tk
from tkinter.scrolledtext import ScrolledText
from tkinter import ttk

# Récupérer les chemins
paths = get_paths()
BDD_FOLDER = paths["bdd"]
DB_FILE = os.path.join(BDD_FOLDER, "cardinfo.db")
LAST_UPDATE_FILE = os.path.join(BDD_FOLDER, "last_update.txt")


def create_bdd_folder():
    if not os.path.exists(BDD_FOLDER):
        os.makedirs(BDD_FOLDER)
        # print(f"Le dossier de base de données '{BDD_FOLDER}' a été créé avec succès.")
    else:
        # print(f"Le dossier de base de données '{BDD_FOLDER}' existe déjà.")
        pass


def fetch_card_data(language="en"):
    url = f"https://db.ygoprodeck.com/api/v7/cardinfo.php?includeAliased"
    if language and language != "en":
        url = f"https://db.ygoprodeck.com/api/v7/cardinfo.php?language={language}&includeAliased"
    response = requests.get(url)
    if response.status_code == 200:
        return response.json()
    else:
        raise Exception("Échec de récupération des données depuis l'API")


def transform_data(data):
    df = pd.DataFrame(data)

    # Expansion des colonnes complexes
    if "card_sets" in df.columns:
        df = df.explode("card_sets").reset_index(drop=True)
        sets_df = pd.json_normalize(df["card_sets"].tolist()).add_prefix("card_sets.")
        df = pd.concat([df.drop(columns=["card_sets"]), sets_df], axis=1)

    if "card_images" in df.columns:
        df = df.explode("card_images").reset_index(drop=True)
        img_df = pd.json_normalize(df["card_images"].tolist()).add_prefix("card_images.")
        df = pd.concat([df.drop(columns=["card_images"]), img_df], axis=1)

    if "card_prices" in df.columns:
        df = df.explode("card_prices").reset_index(drop=True)
        price_df = pd.json_normalize(df["card_prices"].tolist()).add_prefix("card_prices.")
        df = pd.concat([df.drop(columns=["card_prices"]), price_df], axis=1)

    if "banlist_info" in df.columns:
        banlist_df = pd.json_normalize(df["banlist_info"].tolist()).add_prefix("banlist_info.")
        df = pd.concat([df.drop(columns=["banlist_info"]), banlist_df], axis=1)

    # Renommer 'desc' pour éviter conflit SQL
    if "desc" in df.columns:
        df.rename(columns={"desc": "description"}, inplace=True)

    return df


def create_dynamic_table(df):
    conn = sqlite3.connect(DB_FILE)
    cursor = conn.cursor()

    cursor.execute("DROP TABLE IF EXISTS cards")

    # Nettoyage des noms de colonnes
    df.columns = [col.replace(".", "_") for col in df.columns]

    types = []
    for col in df.columns:
        if df[col].dtype == "int64":
            types.append(f'"{col}" INTEGER')
        else:
            types.append(f'"{col}" TEXT')

    create_stmt = f'CREATE TABLE cards ({", ".join(types)})'
    cursor.execute(create_stmt)
    conn.commit()
    conn.close()
    # print("Table 'cards' créée dynamiquement avec toutes les colonnes.")


def insert_data_to_db(df):
    conn = sqlite3.connect(DB_FILE)
    cursor = conn.cursor()

    placeholders = ", ".join(["?" for _ in df.columns])
    insert_stmt = f"INSERT INTO cards ({', '.join(df.columns)}) VALUES ({placeholders})"

    for _, row in df.iterrows():
        values = [json.dumps(v) if isinstance(v, (dict, list)) else v for v in row]
        cursor.execute(insert_stmt, values)

    conn.commit()
    conn.close()
    # print("Données insérées avec succès.")


def afficher_fenetre_etapes(etapes):
    """
    Affiche une fenêtre Tkinter listant les étapes en temps réel.
    """
    root = tk.Tk()
    root.title("Initialisation de la base de données")
    root.geometry("600x400")
    
    # Centrer la fenêtre
    root.withdraw()  # Cache temporairement la fenêtre
    root.update_idletasks()  # Met à jour la géométrie
    x = (root.winfo_screenwidth() - 600) // 2
    y = (root.winfo_screenheight() - 400) // 2
    root.geometry(f"+{x}+{y}")
    root.deiconify()  # Réaffiche la fenêtre
    
    # Force la fenêtre à rester au premier plan
    root.attributes('-topmost', True)
    
    # Titre stylisé
    title_label = ttk.Label(
        root,
        text="Initialisation de la base de données",
        font=("Helvetica", 14, "bold")
    )
    title_label.pack(pady=10)

    text_area = ScrolledText(root, state='normal', font=("Consolas", 11), height=15)
    text_area.pack(expand=True, fill='both', padx=20, pady=(0, 20))

    for texte, couleur in etapes:
        text_area.insert(tk.END, f"➔ {texte}\n", couleur)
        text_area.tag_config(couleur, foreground=couleur)
        text_area.see(tk.END)
        root.update()
        root.lift()  # Garde la fenêtre au premier plan

    text_area.config(state='disabled')

    # Remplacer la fermeture automatique par une vérification
    def verifier_et_fermer():
        try:
            if verify_database():
                # Ajouter un message final de succès
                text_area.config(state='normal')
                text_area.insert(tk.END, "\n✅ Initialisation complète - Fermeture dans 2 secondes...\n", "green")
                text_area.config(state='disabled')
                text_area.see(tk.END)
                root.after(2000, root.destroy)  # Fermeture après 2 secondes
            else:
                # Si la vérification échoue, ajouter un message d'erreur
                text_area.config(state='normal')
                text_area.insert(tk.END, "\n❌ Échec de l'initialisation\n", "red")
                text_area.config(state='disabled')
                text_area.see(tk.END)
                root.after(3000, root.destroy)  # Fermeture après 3 secondes en cas d'erreur
        except Exception as e:
            text_area.config(state='normal')
            text_area.insert(tk.END, f"\n❌ Erreur : {str(e)}\n", "red")
            text_area.config(state='disabled')
            text_area.see(tk.END)
            root.after(3000, root.destroy)

    # Remplacer root.after(5500, root.destroy) par :
    root.after(1000, verifier_et_fermer)  # Vérification après 1 seconde
    root.mainloop()


class DatabaseInitializationError(Exception):
    pass


def verify_database():
    """Vérifie si la base de données existe et est correctement initialisée"""
    if not os.path.exists(DB_FILE):
        raise DatabaseInitializationError("La base de données n'existe pas")

    try:
        conn = sqlite3.connect(DB_FILE)
        cursor = conn.cursor()

        # Vérifie si la table cards existe
        cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='cards'")
        if not cursor.fetchone():
            raise DatabaseInitializationError("La table 'cards' n'existe pas")

        # Vérifie si la table contient des données
        cursor.execute("SELECT COUNT(*) FROM cards")
        count = cursor.fetchone()[0]
        if count == 0:
            raise DatabaseInitializationError("La base de données est vide")

        conn.close()
        return True

    except sqlite3.Error as e:
        raise DatabaseInitializationError(f"Erreur lors de la vérification de la BDD: {e}")


def save_last_update():
    """Sauvegarde les informations de version de la dernière mise à jour"""
    try:
        # Récupérer les informations de version depuis l'API
        try:
            response = requests.get("https://db.ygoprodeck.com/api/v7/checkDBVer.php", timeout=10)
            version_info = response.json()
        except:
            # Si l'API n'est pas accessible, sauvegarder juste la date
            version_info = {"database_version": "unknown", "last_update": datetime.now().strftime('%Y-%m-%d %H:%M:%S')}

        # Sauvegarder dans le fichier
        with open(LAST_UPDATE_FILE, 'w', encoding='utf-8') as f:
            json.dump(version_info, f, indent=2, ensure_ascii=False)
        return True
    except Exception as e:
        print(f"Erreur lors de la sauvegarde de la date de mise à jour : {e}")
        return False


def main():
    etapes = []
    try:
        # S'assurer que le dossier BDD existe
        create_bdd_folder()
        etapes.append(("Création du dossier de base de données...", "blue"))
        
        # Sauvegarder la version initiale avant tout
        if save_last_update():
            etapes.append(("✅ Fichier de version créé.", "green"))
        
        etapes.append(("Téléchargement des données...", "blue"))
        card_data = fetch_card_data()
        etapes.append(("Transformation des données...", "blue"))
        df = transform_data(card_data["data"])
        etapes.append(("Création de la table SQL...", "blue"))
        create_dynamic_table(df)
        etapes.append(("Insertion des données dans la base...", "blue"))
        insert_data_to_db(df)
        
        # Vérification finale complète
        if verify_database():
            # Sauvegarder la date de mise à jour
            if save_last_update():
                etapes.append(("✅ Base de données créée et vérifiée avec succès.", "green"))
                etapes.append(("✅ Date de mise à jour sauvegardée.", "green"))
            else:
                etapes.append(("⚠️ Base de données créée mais erreur lors de la sauvegarde de la date.", "orange"))
            return True
        else:
            raise DatabaseInitializationError("La vérification finale a échoué")
            
    except Exception as e:
        etapes.append((f"❌ Une erreur s'est produite : {e}", "red"))
        return False
    finally:
        afficher_fenetre_etapes(etapes)
    return True


# Fonction de vérification publique
def is_database_ready():
    try:
        verify_database()
        return True
    except DatabaseInitializationError:
        return False

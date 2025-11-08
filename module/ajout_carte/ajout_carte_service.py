import os
import sqlite3
import json
from PIL import Image
import urllib.request
import io

def get_classeurs(classeur_folder):
    return [d for d in os.listdir(classeur_folder) if os.path.isdir(os.path.join(classeur_folder, d))]

def load_rarity_priorities(config_file, all_rarities=None):
    if os.path.exists(config_file):
        try:
            with open(config_file, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}
    if all_rarities:
        return {r: 1 for r in all_rarities}
    return {}

def sort_by_rarity(results, rarity_priorities):
    def get_rarity_priority(rarity):
        return rarity_priorities.get(rarity, 9999)
    return sorted(results, key=lambda x: (get_rarity_priority(x[2]), x[0]))

def search_card(cardinfo_db, search_term, rarity_priorities):
    conn = sqlite3.connect(cardinfo_db)
    cursor = conn.cursor()
    cursor.execute("PRAGMA table_info(cards)")
    all_columns = [col[1] for col in cursor.fetchall()]
    cursor.execute("""
        SELECT * FROM cards 
        WHERE name LIKE ? OR card_sets_set_code LIKE ?
    """, (f"%{search_term}%", f"%{search_term}%"))
    results = cursor.fetchall()
    conn.close()
    idx_name = all_columns.index("name")
    idx_rarity = all_columns.index("card_sets_set_rarity")
    sorted_results = sorted(results, key=lambda x: (rarity_priorities.get(x[idx_rarity], 9999), x[idx_name]))
    sorted_results = sorted_results[:10]
    return [dict(zip(all_columns, row)) for row in sorted_results]

def ajouter_carte(db_path, data, possessed, is_custom=True):
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    cursor.execute("PRAGMA table_info(cards)")
    columns = [col[1] for col in cursor.fetchall()]
    if "is_custom" not in columns:
        cursor.execute("ALTER TABLE cards ADD COLUMN is_custom INTEGER DEFAULT 0")
    if "set_code_prefix" not in columns:
        cursor.execute("ALTER TABLE cards ADD COLUMN set_code_prefix TEXT")
    # Ajout du set_code_prefix si absent
    if "set_code_prefix" not in data or not data["set_code_prefix"]:
        code = data.get("card_sets_set_code", "")
        data["set_code_prefix"] = code.split("-")[0] if "-" in code else code
    cursor.execute("""
        INSERT INTO cards (name, card_sets_set_code, card_sets_set_rarity, 
                         description, is_custom, possessed, card_images_image_url, set_code_prefix)
        VALUES (?, ?, ?, ?, 1, ?, ?, ?)
    """, (
        data["name"], 
        data["card_sets_set_code"], 
        data["card_sets_set_rarity"], 
        data["description"],
        possessed,
        data.get("card_images_image_url", ""),
        data["set_code_prefix"]
    ))
    conn.commit()
    conn.close()

def ajouter_cartes_selectionnees(db_path, cards, flags_possessed):
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    cursor.execute("PRAGMA table_info(cards)")
    columns = [col[1] for col in cursor.fetchall()]
    if "is_custom" not in columns:
        cursor.execute("ALTER TABLE cards ADD COLUMN is_custom INTEGER DEFAULT 0")
    cursor.execute("PRAGMA table_info(cards)")
    target_columns = [col[1] for col in cursor.fetchall()]
    added_count = 0
    for card_dict in cards:
        card_dict["possessed"] = flags_possessed
        card_dict["is_custom"] = 1
        insert_cols = [col for col in target_columns if col in card_dict]
        insert_vals = [card_dict[col] for col in insert_cols]
        placeholders = ",".join(["?" for _ in insert_cols])
        sql = f"INSERT INTO cards ({','.join(insert_cols)}) VALUES ({placeholders})"
        cursor.execute(sql, insert_vals)
        added_count += 1
    conn.commit()
    conn.close()
    return added_count

def supprimer_carte(db_path, nom, code, rarete):
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    cursor.execute("""
        DELETE FROM cards 
        WHERE name = ? AND card_sets_set_code = ? AND card_sets_set_rarity = ?
    """, (nom, code, rarete))
    count = cursor.rowcount
    conn.commit()
    conn.close()
    return count

def get_custom_cards(classeur_folder, classeurs):
    result = []
    for classeur in classeurs:
        db_path = os.path.join(classeur_folder, classeur, f"{classeur}.db")
        if not os.path.exists(db_path):
            continue
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        cursor.execute("PRAGMA table_info(cards)")
        columns = [col[1] for col in cursor.fetchall()]
        if "is_custom" not in columns:
            cursor.execute("ALTER TABLE cards ADD COLUMN is_custom INTEGER DEFAULT 0")
            conn.commit()
        cursor.execute("""
            SELECT name, card_sets_set_code, card_sets_set_rarity
            FROM cards
            WHERE is_custom = 1
            ORDER BY name
        """)
        for row in cursor.fetchall():
            result.append((*row, classeur))
        conn.close()
    return result

def supprimer_cartes_selectionnees(classeur_folder, by_classeur):
    deleted_count = 0
    for classeur, cards in by_classeur.items():
        db_path = os.path.join(classeur_folder, classeur, f"{classeur}.db")
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        for nom, code, rarete in cards:
            cursor.execute("""
                DELETE FROM cards 
                WHERE name = ? AND card_sets_set_code = ? 
                AND card_sets_set_rarity = ? AND is_custom = 1
            """, (nom, code, rarete))
            deleted_count += cursor.rowcount
        conn.commit()
        conn.close()
    return deleted_count

def charger_image_depuis_url_ou_fichier(url, size=(200, 150)):
    """
    Charge une image depuis une URL http(s) ou un chemin local, et la redimensionne.
    Retourne un objet PIL.Image ou lève une exception si erreur.
    Le paramètre size correspond à (largeur, hauteur) = (width, height).
    """
    if not url:
        raise ValueError("URL vide")
    try:
        if url.startswith("http"):
            with urllib.request.urlopen(url, timeout=3) as u:
                raw_data = u.read()
            im = Image.open(io.BytesIO(raw_data))
        else:
            im = Image.open(url)
        im.thumbnail(size)
        return im
    except Exception as e:
        raise e

def completer_infos_depuis_cardinfo(cardinfo_db, data):
    """Complète le dict data avec les infos manquantes depuis cardinfo.db (si trouvée)."""
    conn = sqlite3.connect(cardinfo_db)
    cursor = conn.cursor()
    cursor.execute("PRAGMA table_info(cards)")
    all_columns = [col[1] for col in cursor.fetchall()]
    cursor.execute("""
        SELECT * FROM cards
        WHERE name = ? AND card_sets_set_code = ? AND card_sets_set_rarity = ?
    """, (data["name"], data["card_sets_set_code"], data["card_sets_set_rarity"]))
    row = cursor.fetchone()
    if row:
        cardinfo = dict(zip(all_columns, row))
        # Complète data seulement si la clé n'existe pas déjà
        for k, v in cardinfo.items():
            if k not in data:
                data[k] = v
    conn.close()
    return data

# Ce fichier contient uniquement la logique métier/fonctionnelle :
# - Accès et manipulation de la base de données (ajout, suppression, recherche, tri, etc.)
# - Chargement d'image (sans gestion d'affichage)
# - Aucune dépendance à Tkinter ou à l'UI

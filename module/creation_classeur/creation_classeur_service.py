import os
import sqlite3
import json
import shutil
import pandas as pd
import stat
from module.centralisation_dossier import CLASSEUR_FOLDER, CARDINFO_DB

def create_classeur(code_set):
    try:
        code_set = str(code_set)
        if not os.path.exists(CLASSEUR_FOLDER):
            return False

        classeur_path = os.path.join(CLASSEUR_FOLDER, code_set)
        if os.path.exists(classeur_path):
            return False

        os.makedirs(classeur_path)

        if not os.path.exists(CARDINFO_DB):
            return False

        try:
            conn = sqlite3.connect(CARDINFO_DB)
            df = pd.read_sql_query("SELECT * FROM cards", conn)
            conn.close()
        except sqlite3.Error as e:
            return False

        df["set_code_prefix"] = df["card_sets_set_code"].str.split("-").str[0]
        df_filtered = df[df["set_code_prefix"] == code_set]

        db_file = os.path.join(classeur_path, f"{code_set}.db")
        conn = sqlite3.connect(db_file)
        cursor = conn.cursor()

        df_filtered.columns = [col.replace(".", "_") for col in df_filtered.columns]
        types = [
            f'"{col}" INTEGER' if df_filtered[col].dtype == "int64" else f'"{col}" TEXT'
            for col in df_filtered.columns
        ]
        types.append('"cardmarket_url" TEXT')
        types.append('"possessed" INTEGER DEFAULT 0')
        types.append('"is_custom" INTEGER DEFAULT 0')
        types.append('"quantite" INTEGER DEFAULT 0')
        types.append('"qualite" TEXT DEFAULT NULL')

        cursor.execute("DROP TABLE IF EXISTS cards")
        create_stmt = f'CREATE TABLE cards ({", ".join(types)})'
        cursor.execute(create_stmt)

        placeholders = ", ".join(["?" for _ in df_filtered.columns])
        insert_stmt = f"INSERT INTO cards ({', '.join(df_filtered.columns)}, quantite) VALUES ({placeholders}, 0)"
        for _, row in df_filtered.iterrows():
            values = [json.dumps(v) if isinstance(v, (dict, list)) else v for v in row]
            cursor.execute(insert_stmt, values)

        conn.commit()
        conn.close()
        return True
    except Exception as e:
        return False

def remove_readonly(func, path, _):
    os.chmod(path, stat.S_IWRITE)
    func(path)

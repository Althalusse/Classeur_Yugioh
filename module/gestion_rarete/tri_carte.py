import os
import json
from module.centralisation_dossier import CONFIG_FILE

def load_rarity_priorities():
    if os.path.exists(CONFIG_FILE):
        try:
            with open(CONFIG_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}
    return {}

def sort_cartes(cartes):
    rarity_priorities = load_rarity_priorities()
    def get_rarity_priority(rarity):
        return rarity_priorities.get(rarity, 9999)
    return sorted(cartes, key=lambda c: (c.get("code", ""), get_rarity_priority(c.get("rarity", "")), c.get("name", "")))

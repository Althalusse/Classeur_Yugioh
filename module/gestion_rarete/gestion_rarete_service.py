import os
import json
from module.centralisation_dossier import CONFIG_FILE

ALL_RARITIES = [
    "Common", "Rare", "Super Rare", "Ultra Rare", "Secret Rare", "Platinum Secret Rare",
    "Ultimate Rare", "Collector's Rare", "Prismatic Secret Rare", "Quarter Century Secret Rare",
    "Short Print", "Ghost Rare", "Extra Secret", "Extra Secret Rare", "Super Short Print",
    "Starlight Rare", "Starfoil Rare", "10000 Secret Rare", "Mosaic Rare", "Shatterfoil Rare",
    "Duel Terminal Super Parallel Rare", "Duel Terminal Rare Parallel Rare",
    "Duel Terminal Normal Parallel Rare", "Duel Terminal Ultra Parallel Rare",
    "Duel Terminal Normal Rare Parallel Rare", "Gold Rare", "Ghost/Gold Rare",
    "Ultra Secret Rare", "Ultra Parallel Rare", "New", "Ultra Rare (Pharaoh's Rare)",
    "Premium Gold Rare", "Normal Parallel Rare", "Gold Secret Rare", "Platinum Rare",
    "Super Parallel Rare", "Starfoil", "Reprint", "European debut", "European & Oceanian debut",
    "Oceanian debut"
]

def load_rarity_priorities():
    if os.path.exists(CONFIG_FILE):
        try:
            with open(CONFIG_FILE, "r", encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return {}
    return {}

def save_rarity_priorities(priorities):
    with open(CONFIG_FILE, "w", encoding="utf-8") as f:
        json.dump(priorities, f, indent=4, ensure_ascii=False)

def get_default_priorities():
    return {rarity: 1 for rarity in ALL_RARITIES}

def sort_cards_by_rarity(cards_list):
    priorities = load_rarity_priorities()
    def get_priority(card):
        return priorities.get(card.get("rarity") or card.get("Rareté") or "", 9999)
    return sorted(cards_list, key=lambda c: (get_priority(c), c.get("name", "")))

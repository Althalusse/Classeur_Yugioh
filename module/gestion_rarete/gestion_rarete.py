import os
import json
import tkinter as tk
from tkinter import ttk, messagebox
from module.centralisation_dossier import ROOT_FOLDER, BDD_FOLDER, CONFIG_FILE
from .gestion_rarete_service import (
    ALL_RARITIES,
    load_rarity_priorities,
    save_rarity_priorities,
    get_default_priorities,
    sort_cards_by_rarity
)

class RarityPriorityConfig(tk.Toplevel):
    def __init__(self, master=None):
        super().__init__(master)
        self.title("Configuration des Priorités de Rareté")
        self.geometry("350x600")
        self.resizable(False, True)

        self.protocol("WM_DELETE_WINDOW", self.on_close)  # Gère la fermeture propre

        self.priorities = load_rarity_priorities() or get_default_priorities()

        self.canvas = tk.Canvas(self)
        self.frame = ttk.Frame(self.canvas)
        self.scrollbar = ttk.Scrollbar(self, orient="vertical", command=self.canvas.yview)
        self.canvas.configure(yscrollcommand=self.scrollbar.set)

        self.scrollbar.pack(side="right", fill="y")
        self.canvas.pack(side="left", fill="both", expand=True)
        self.canvas.create_window((0, 0), window=self.frame, anchor="nw")

        self.frame.bind("<Configure>", lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")))

        self.entries = {}

        ttk.Label(
            self.frame,
            text="Attribuez une priorité (nombre entier, plus petit = priorité plus haute)",
            wraplength=300
        ).pack(pady=10)

        for rarity in ALL_RARITIES:
            frame_line = ttk.Frame(self.frame)
            frame_line.pack(fill="x", padx=10, pady=2)

            label = ttk.Label(frame_line, text=rarity, width=30, anchor="w")
            label.pack(side="left")

            var = tk.IntVar(value=self.priorities.get(rarity, 1))
            entry = ttk.Entry(frame_line, textvariable=var, width=5)
            entry.pack(side="left")
            self.entries[rarity] = var

        btn_save = ttk.Button(self.frame, text="Sauvegarder", command=self.save_config)
        btn_save.pack(pady=15)

    def on_close(self):
        self.destroy()
        if isinstance(self.master, tk.Tk):
            self.master.destroy()

    def save_config(self):
        try:
            for rarity, var in self.entries.items():
                val = var.get()
                if val < 1:
                    messagebox.showerror("Erreur", f"La priorité pour '{rarity}' doit être un entier supérieur à 0.")
                    return

            self.priorities = {rarity: var.get() for rarity, var in self.entries.items()}
            save_rarity_priorities(self.priorities)

            messagebox.showinfo("Succès", "Configuration des priorités sauvegardée.")
            self.on_close()

        except Exception as e:
            messagebox.showerror("Erreur", f"Erreur lors de la sauvegarde : {e}")

class RarityPriorityFrame(ttk.Frame):
    def __init__(self, parent):
        super().__init__(parent)
        self.priorities = load_rarity_priorities() or get_default_priorities()

        self.canvas = tk.Canvas(self)
        self.frame = ttk.Frame(self.canvas)
        self.scrollbar = ttk.Scrollbar(self, orient="vertical", command=self.canvas.yview)
        self.canvas.configure(yscrollcommand=self.scrollbar.set)

        self.scrollbar.pack(side="right", fill="y")
        self.canvas.pack(side="left", fill="both", expand=True)
        self.canvas.create_window((0, 0), window=self.frame, anchor="nw")

        self.frame.bind("<Configure>", lambda e: self.canvas.configure(scrollregion=self.canvas.bbox("all")))

        self.entries = {}

        ttk.Label(
            self.frame,
            text="Attribuez une priorité (nombre entier, plus petit = priorité plus haute)",
            wraplength=300
        ).pack(pady=10)

        for rarity in ALL_RARITIES:
            frame_line = ttk.Frame(self.frame)
            frame_line.pack(fill="x", padx=10, pady=2)

            label = ttk.Label(frame_line, text=rarity, width=30, anchor="w")
            label.pack(side="left")

            var = tk.IntVar(value=self.priorities.get(rarity, 1))
            entry = ttk.Entry(frame_line, textvariable=var, width=5)
            entry.pack(side="left")
            self.entries[rarity] = var

        btn_save = ttk.Button(self.frame, text="Sauvegarder", command=self.save_config)
        btn_save.pack(pady=15)

    def save_config(self):
        try:
            for rarity, var in self.entries.items():
                val = var.get()
                if val < 1:
                    messagebox.showerror("Erreur", f"La priorité pour '{rarity}' doit être un entier supérieur à 0.")
                    return

            self.priorities = {rarity: var.get() for rarity, var in self.entries.items()}
            save_rarity_priorities(self.priorities)

            messagebox.showinfo("Succès", "Configuration des priorités sauvegardée.")
        except Exception as e:
            messagebox.showerror("Erreur", f"Erreur lors de la sauvegarde : {e}")

# Utilitaires d'accès pour d'autres modules
def load_rarity_priorities_ui():
    return load_rarity_priorities()

def sort_cards_by_rarity_ui(cards_list):
    return sort_cards_by_rarity(cards_list)

if __name__ == "__main__":
    try:
        root = tk.Tk()
        root.withdraw()
        config_window = RarityPriorityConfig(master=root)
        config_window.mainloop()
    except Exception as e:
        import traceback
        traceback.print_exc()

    # Exemple d'utilisation hors interface
    cartes = [
        {"rarity": "Rare", "name": "Carte A"},
        {"rarity": "Ultra Rare", "name": "Carte B"},
        {"rarity": "Common", "name": "Carte C"},
        {"rarity": "Secret Rare", "name": "Carte D"},
        {"rarity": "Inconnue", "name": "Carte E"},
    ]

    cartes_tries = sort_cards_by_rarity_ui(cartes)
    for c in cartes_tries:
        print(c)

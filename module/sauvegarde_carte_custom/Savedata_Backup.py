import os
import tkinter as tk
from tkinter import Frame, Button, Listbox, messagebox, END, LEFT
from module.sauvegarde_carte_custom.Savedata_Backup_service import (
    create_backup,
    restore_backup,
    get_available_backups,
    cleanup_old_backups,
    delete_backup
)
import json

class BackupUI:
    def __init__(self, master):
        self.master = master
        master.title("Gestion des sauvegardes")

        # Cadre pour les boutons du haut
        self.frame = Frame(master)
        self.frame.pack(pady=10)

        # Bouton pour créer une sauvegarde
        self.backup_button = Button(self.frame, text="Créer une sauvegarde", command=self.create_backup)
        self.backup_button.pack(side=LEFT, padx=5)

        # Liste des sauvegardes
        self.backup_list = Listbox(master, width=50)
        self.backup_list.pack(pady=10)

        # Cadre pour les boutons du bas
        self.bottom_frame = Frame(master)
        self.bottom_frame.pack(pady=5)

        self.refresh_button = Button(self.bottom_frame, text="Rafraîchir la liste", command=self.load_backups)
        self.refresh_button.pack(fill=tk.X, pady=2)
        self.restore_button = Button(self.bottom_frame, text="Restaurer la sauvegarde", command=self.restore_backup)
        self.restore_button.pack(fill=tk.X, pady=2)
        self.delete_button = Button(self.bottom_frame, text="Supprimer la sauvegarde", command=self.delete_backup)
        self.delete_button.pack(fill=tk.X, pady=2)
        self.show_classeurs_button = Button(self.bottom_frame, text="Voir les classeurs", command=self.show_classeurs, bg="yellow")
        self.show_classeurs_button.pack(fill=tk.X, pady=2)

        # Charger les sauvegardes disponibles
        self.load_backups()

    def load_backups(self):
        """Charge les sauvegardes disponibles dans la liste"""
        backups = get_available_backups()
        self.backup_list.delete(0, END)  # Effacer la liste actuelle
        for backup in backups:
            self.backup_list.insert(END, f"{backup['timestamp']} - {backup['path']}")

    def create_backup(self):
        """Créer une nouvelle sauvegarde"""
        try:
            backup_file = create_backup()
            messagebox.showinfo("Succès", f"Sauvegarde créée : {backup_file}")
            self.load_backups()  # Recharger la liste des sauvegardes
        except Exception as e:
            messagebox.showerror("Erreur", str(e))

    def restore_backup(self):
        """Restaurer la sauvegarde sélectionnée"""
        try:
            selected = self.backup_list.curselection()
            if not selected:
                raise Exception("Veuillez sélectionner une sauvegarde à restaurer.")
            backup_file = self.backup_list.get(selected[0]).split(" - ")[1]
            restore_backup(backup_file)
            messagebox.showinfo("Succès", "Sauvegarde restaurée avec succès.")
        except Exception as e:
            messagebox.showerror("Erreur", str(e))

    def delete_backup(self):
        """Supprimer la sauvegarde sélectionnée"""
        try:
            selected = self.backup_list.curselection()
            if not selected:
                raise Exception("Veuillez sélectionner une sauvegarde à supprimer.")
            backup_file = self.backup_list.get(selected[0]).split(" - ")[1]
            delete_backup(backup_file)
            messagebox.showinfo("Succès", "Sauvegarde supprimée avec succès.")
            self.load_backups()  # Recharger la liste des sauvegardes
        except Exception as e:
            messagebox.showerror("Erreur", str(e))

    def show_classeurs(self):
        """Affiche les classeurs présents dans la sauvegarde sélectionnée"""
        try:
            selected = self.backup_list.curselection()
            if not selected:
                raise Exception("Veuillez sélectionner une sauvegarde.")
            backup_file = self.backup_list.get(selected[0]).split(" - ")[1]
            if not os.path.exists(backup_file):
                raise Exception("Le fichier de sauvegarde n'existe pas.")
            with open(backup_file, "r", encoding="utf-8") as f:
                data = json.load(f)
            classeurs = data.get("classeurs", [])
            if not classeurs:
                messagebox.showinfo("Classeurs", "Aucun classeur présent dans cette sauvegarde.")
            else:
                messagebox.showinfo("Classeurs dans la sauvegarde", "\n".join(classeurs))
        except Exception as e:
            messagebox.showerror("Erreur", str(e))